use crate::index::{IndexState, SessionIndex};
use crate::parser;
use crate::session::{SearchResult, Session};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Debounce delay for search (avoid searching on every keystroke during fast typing/paste)
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(50);

/// Messages from the indexing thread
pub enum IndexMsg {
    Progress { indexed: usize, total: usize },
    Done { total_sessions: usize },
    NeedsReload,
    Error(String),
}

/// Search scope
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchScope {
    /// Search all conversations
    Everything,
    /// Search only conversations from a specific folder
    Folder(String),
}

pub struct App {
    /// Current search query
    pub query: String,
    /// Cursor position in query (char index)
    pub cursor: usize,
    /// Search results
    pub results: Vec<SearchResult>,
    /// Selected result index
    pub selected: usize,
    /// Results list scroll offset
    pub list_scroll: usize,
    /// Preview scroll offset
    pub preview_scroll: usize,
    /// Whether to auto-scroll preview to matched message
    pub pending_auto_scroll: bool,
    /// Whether preview has more content than visible (for scroll hint)
    pub preview_scrollable: bool,
    /// Should quit
    pub should_quit: bool,
    /// Should execute resume (set on Enter)
    pub should_resume: Option<Session>,
    /// Session ID to copy (set on Tab)
    pub should_copy: Option<String>,
    /// Index for searching
    index: SessionIndex,
    /// Status message (for indexing progress, etc.)
    pub status: Option<String>,
    /// Total sessions indexed
    pub total_sessions: usize,
    /// Channel to receive indexing updates
    index_rx: Option<Receiver<IndexMsg>>,
    /// Is indexing in progress
    pub indexing: bool,
    /// Current search scope
    pub search_scope: SearchScope,
    /// Launch directory (for folder-scoped search)
    pub launch_cwd: String,
    /// Whether a search is pending (for debouncing)
    search_pending: bool,
    /// When the last input occurred (for debouncing)
    last_input: Instant,
    /// Error from indexing thread (shown on exit)
    pub index_error: Option<String>,
}

impl App {
    pub fn new(initial_query: String) -> Result<Self> {
        // Allow override for testing
        let cache_dir = std::env::var("RECALL_HOME_OVERRIDE")
            .map(|h| PathBuf::from(h).join(".cache").join("recall"))
            .unwrap_or_else(|_| {
                dirs::cache_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("recall")
            });

        let index_path = cache_dir.join("index");
        let state_path = cache_dir.join("state.json");

        let index = SessionIndex::open_or_create(&index_path)?;

        // Get launch directory (override for tests)
        let launch_cwd = std::env::var("RECALL_CWD_OVERRIDE").unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        });

        // Start background indexing
        let (tx, rx) = mpsc::channel();
        let index_path_clone = index_path.clone();
        thread::spawn(move || {
            background_index(index_path_clone, state_path, tx);
        });

        let initial_cursor = initial_query.chars().count();
        let mut app = Self {
            query: initial_query,
            cursor: initial_cursor,
            results: Vec::new(),
            selected: 0,
            list_scroll: 0,
            preview_scroll: 0,
            pending_auto_scroll: false,
            preview_scrollable: false,
            should_quit: false,
            should_resume: None,
            should_copy: None,
            index,
            status: None,
            total_sessions: 0,
            index_rx: Some(rx),
            indexing: true,
            search_scope: SearchScope::Folder(launch_cwd.clone()),
            launch_cwd,
            search_pending: false,
            last_input: Instant::now(),
            index_error: None,
        };

        // If there's an initial query, run the search immediately
        if !app.query.is_empty() {
            let _ = app.search();
        }

        Ok(app)
    }

    /// Check for indexing updates (call this in the main loop)
    pub fn poll_index_updates(&mut self) {
        use std::sync::mpsc::TryRecvError;

        let Some(rx) = &self.index_rx else {
            return;
        };

        // Collect messages, tracking if channel was disconnected
        let mut messages = Vec::new();
        let mut channel_disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(msg) => messages.push(msg),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    channel_disconnected = true;
                    break;
                }
            }
        }

        let mut should_close_rx = false;
        let mut needs_reload = false;
        let mut needs_search = false;

        for msg in messages {
            match msg {
                IndexMsg::Progress { indexed, total } => {
                    self.status = Some(format!("Indexing {}/{}...", indexed, total));
                    self.total_sessions = indexed;
                }
                IndexMsg::NeedsReload => {
                    needs_reload = true;
                    needs_search = true;
                }
                IndexMsg::Done { total_sessions } => {
                    self.total_sessions = total_sessions;
                    self.status = None;
                    self.indexing = false;
                    should_close_rx = true;
                    needs_reload = true;
                    needs_search = true;
                }
                IndexMsg::Error(err) => {
                    self.index_error = Some(err);
                    self.status = Some("Index error • Ctrl+C for details".to_string());
                    self.indexing = false;
                    should_close_rx = true;
                }
            }
        }

        // Detect unexpected indexer death (channel closed without Done/Error)
        if channel_disconnected && self.indexing {
            self.index_error = Some("Indexer stopped unexpectedly (possible crash)".to_string());
            self.status = Some("Index error • Ctrl+C for details".to_string());
            self.indexing = false;
            should_close_rx = true;
        }

        if needs_reload {
            let _ = self.index.reload();
        }
        if needs_search {
            let _ = self.search();
        }
        if should_close_rx {
            self.index_rx = None;
        }
    }

    /// Perform a search (or show recent sessions if query is empty)
    pub fn search(&mut self) -> Result<()> {
        // Remember currently selected session to preserve selection
        let selected_session_id = self.results.get(self.selected).map(|r| r.session.id.clone());

        let mut results = if self.query.is_empty() {
            self.index.recent(50)?
        } else {
            self.index.search(&self.query, 50)?
        };

        // Filter by scope if searching within a folder
        if let SearchScope::Folder(ref cwd) = self.search_scope {
            results.retain(|r| r.session.cwd == *cwd);
        }

        self.results = results;

        // Try to preserve selection on the same session
        if let Some(ref id) = selected_session_id {
            if let Some(pos) = self.results.iter().position(|r| &r.session.id == id) {
                self.selected = pos;
                // Scroll to keep selection visible (at top of list area)
                self.list_scroll = pos;
            } else {
                self.selected = 0;
                self.list_scroll = 0;
            }
        } else {
            self.selected = 0;
            self.list_scroll = 0;
        }
        self.update_preview_scroll();

        Ok(())
    }

    /// Toggle search scope between everything and current folder
    pub fn toggle_scope(&mut self) {
        self.search_scope = match self.search_scope {
            SearchScope::Everything => SearchScope::Folder(self.launch_cwd.clone()),
            SearchScope::Folder(_) => SearchScope::Everything,
        };
        let _ = self.search();
    }

    /// Get the folder name for display (last component of path)
    pub fn scope_folder_name(&self) -> Option<&str> {
        match &self.search_scope {
            SearchScope::Everything => None,
            SearchScope::Folder(path) => {
                path.rsplit(std::path::MAIN_SEPARATOR).next()
            }
        }
    }

    /// Get a compact display path for the scope
    /// - Replaces home dir with ~
    /// - If short enough, shows full path
    /// - Otherwise shows ~/.../<dir> or /.../<dir>
    pub fn scope_display_path(&self) -> Option<String> {
        let path = match &self.search_scope {
            SearchScope::Everything => return None,
            SearchScope::Folder(path) => path.as_str(),
        };

        // Replace home dir with ~ (HOME on Unix, USERPROFILE on Windows)
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default();
        let display_path = if !home.is_empty() && path.starts_with(&home) {
            format!("~{}", &path[home.len()..])
        } else {
            path.to_string()
        };

        // If short enough, show full path
        const MAX_LEN: usize = 25;
        if display_path.len() <= MAX_LEN {
            return Some(display_path);
        }

        // Otherwise show prefix/.../<last_dir>
        let last_component = path.rsplit(std::path::MAIN_SEPARATOR).next().unwrap_or(path);
        let prefix = if display_path.starts_with('~') { "~" } else { "" };
        Some(format!("{}/.../{}", prefix, last_component))
    }

    /// Handle character input
    pub fn on_char(&mut self, c: char) {
        // Insert at cursor position
        let byte_pos = self.cursor_byte_pos();
        self.query.insert(byte_pos, c);
        self.cursor += 1;
        self.mark_search_pending();
    }

    /// Handle backspace
    pub fn on_backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_pos = self.cursor_byte_pos();
            self.query.remove(byte_pos);
            self.mark_search_pending();
        }
    }

    /// Handle delete key
    pub fn on_delete(&mut self) {
        let char_count = self.query.chars().count();
        if self.cursor < char_count {
            let byte_pos = self.cursor_byte_pos();
            self.query.remove(byte_pos);
            self.mark_search_pending();
        }
    }

    /// Clear search
    pub fn on_escape(&mut self) {
        if self.query.is_empty() {
            self.should_quit = true;
        } else {
            self.query.clear();
            self.cursor = 0;
            self.mark_search_pending();
        }
    }

    /// Move cursor left
    pub fn on_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move cursor right
    pub fn on_right(&mut self) {
        let char_count = self.query.chars().count();
        if self.cursor < char_count {
            self.cursor += 1;
        }
    }

    /// Move cursor to start
    pub fn on_home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end
    pub fn on_end(&mut self) {
        self.cursor = self.query.chars().count();
    }

    /// Convert cursor (char index) to byte position
    fn cursor_byte_pos(&self) -> usize {
        self.query.char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.query.len())
    }

    /// Mark that a search is needed (debounced)
    fn mark_search_pending(&mut self) {
        self.search_pending = true;
        self.last_input = Instant::now();
    }

    /// Check if debounce period has elapsed and trigger search if needed
    pub fn maybe_search(&mut self) {
        if self.search_pending && self.last_input.elapsed() >= SEARCH_DEBOUNCE {
            self.search_pending = false;
            let _ = self.search();
        }
    }

    /// Force any pending search to run immediately (for tests)
    pub fn flush_pending_search(&mut self) {
        if self.search_pending {
            self.search_pending = false;
            let _ = self.search();
        }
    }

    /// Move selection up
    pub fn on_up(&mut self) {
        if !self.results.is_empty() {
            self.selected = self.selected.saturating_sub(1);
            self.update_preview_scroll();
        }
    }

    /// Move selection down
    pub fn on_down(&mut self) {
        if !self.results.is_empty() {
            self.selected = (self.selected + 1).min(self.results.len() - 1);
            self.update_preview_scroll();
        }
    }

    /// Handle Tab key - copy session ID
    pub fn on_tab(&mut self) {
        if let Some(result) = self.results.get(self.selected) {
            self.should_copy = Some(result.session.id.clone());
        }
    }

    /// Handle Enter key - open conversation
    pub fn on_enter(&mut self) {
        if let Some(result) = self.results.get(self.selected) {
            if let Ok(session) = parser::parse_session_file(&result.session.file_path) {
                self.should_resume = Some(session);
            }
        }
    }

    /// Update preview scroll to show the matched message
    fn update_preview_scroll(&mut self) {
        // Signal that we need to auto-scroll to the matched message
        // The actual scroll position is calculated in render_preview
        // since it depends on wrapped line counts
        self.pending_auto_scroll = true;
        self.preview_scroll = 0;
    }

    /// Scroll preview up
    pub fn scroll_preview_up(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_sub(lines);
    }

    /// Scroll preview down
    pub fn scroll_preview_down(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_add(lines);
    }

    /// Get the currently selected result
    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.selected)
    }
}

/// Background indexing function
fn background_index(index_path: PathBuf, state_path: PathBuf, tx: Sender<IndexMsg>) {
    let index = match SessionIndex::open_or_create(&index_path) {
        Ok(idx) => idx,
        Err(e) => {
            let _ = tx.send(IndexMsg::Error(format!("Failed to open index: {}", e)));
            return;
        }
    };
    let mut state = match IndexState::load(&state_path) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(IndexMsg::Error(format!("Failed to load index state: {}", e)));
            return;
        }
    };

    // Discover and sort files by mtime (most recent first)
    let mut files = parser::discover_session_files();
    files.sort_by(|a, b| {
        let mtime_a = std::fs::metadata(a)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mtime_b = std::fs::metadata(b)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        mtime_b.cmp(&mtime_a) // Descending (most recent first)
    });

    let files_to_index: Vec<_> = files
        .iter()
        .filter(|f| state.needs_reindex(f))
        .cloned()
        .collect();

    let total = files_to_index.len();
    if total == 0 {
        let _ = tx.send(IndexMsg::Done {
            total_sessions: files.len(),
        });
        return;
    }

    let mut writer = match index.writer() {
        Ok(w) => w,
        Err(e) => {
            let _ = tx.send(IndexMsg::Error(format!("Failed to create index writer: {}", e)));
            return;
        }
    };

    for (i, file_path) in files_to_index.iter().enumerate() {
        // Delete existing documents for this file
        index.delete_session(&mut writer, file_path);

        // Parse and index
        match parser::parse_session_file(file_path) {
            Ok(session) => {
                if !session.messages.is_empty() {
                    let _ = index.index_session(&mut writer, &session);
                    state.mark_indexed(file_path);
                }
            }
            Err(_) => {
                // Skip failed files silently
            }
        }

        // Progress update every 50 files
        if (i + 1) % 50 == 0 || i + 1 == total {
            let _ = tx.send(IndexMsg::Progress {
                indexed: i + 1,
                total,
            });
        }

        // Commit and notify for reload every 200 files
        if (i + 1) % 200 == 0 {
            let _ = writer.commit();
            let _ = tx.send(IndexMsg::NeedsReload);
        }
    }

    // Final commit
    let _ = writer.commit();
    let _ = state.save(&state_path);

    let _ = tx.send(IndexMsg::Done {
        total_sessions: files.len(),
    });
}
