use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use recall::{app::App, session, session::SessionSource, tui, ui};
use std::time::Duration;

mod cli;

#[derive(Parser)]
#[command(name = "recall")]
#[command(version, about = "Search and resume Claude Code, Codex CLI, and Factory conversations")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Clear index and rebuild from scratch
    #[arg(long, global = true)]
    reindex: bool,

    /// Initial search query (for interactive TUI mode)
    #[arg(trailing_var_arg = true)]
    query: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Search conversations and output JSON
    Search {
        /// Search query
        #[arg(required = true)]
        query: Vec<String>,

        /// Filter by source (claude, codex, factory, opencode)
        #[arg(long, short)]
        source: Option<String>,

        /// Search within a specific session
        #[arg(long)]
        session: Option<String>,

        /// Maximum number of results
        #[arg(long, short, default_value = "10")]
        limit: usize,

        /// Number of context messages around each match
        #[arg(short = 'C', long = "context", default_value = "0")]
        context: usize,

        /// Only include sessions after this time (e.g., "1 week ago", "2025-12-01")
        #[arg(long)]
        since: Option<String>,

        /// Only include sessions before this time
        #[arg(long)]
        until: Option<String>,

        /// Filter by working directory (exact match)
        #[arg(long)]
        cwd: Option<String>,
    },

    /// List recent sessions and output JSON
    List {
        /// Maximum number of sessions
        #[arg(long, short, default_value = "20")]
        limit: usize,

        /// Filter by source (claude, codex, factory, opencode)
        #[arg(long, short)]
        source: Option<String>,

        /// Only include sessions after this time
        #[arg(long)]
        since: Option<String>,

        /// Only include sessions before this time
        #[arg(long)]
        until: Option<String>,

        /// Filter by working directory (exact match)
        #[arg(long)]
        cwd: Option<String>,
    },

    /// Read a full conversation by session ID and output JSON
    Read {
        /// Session ID to read
        session_id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle --reindex
    if cli.reindex {
        clear_index_cache();
    }

    // Dispatch based on command
    match cli.command {
        Some(Command::Search {
            query,
            source,
            session,
            limit,
            context,
            since,
            until,
            cwd,
        }) => {
            let source = parse_source(&source)?;
            cli::run_search(
                &query.join(" "),
                source,
                session,
                limit,
                context,
                since,
                until,
                cwd,
            )
        }
        Some(Command::List {
            limit,
            source,
            since,
            until,
            cwd,
        }) => {
            let source = parse_source(&source)?;
            cli::run_list(limit, source, since, until, cwd)
        }
        Some(Command::Read { session_id }) => cli::run_read(&session_id),
        None => {
            // Interactive TUI mode
            let initial_query = cli.query.join(" ");
            run_tui(initial_query)
        }
    }
}

fn parse_source(source: &Option<String>) -> Result<Option<SessionSource>> {
    match source {
        Some(s) => SessionSource::parse(s)
            .ok_or_else(|| anyhow::anyhow!("Invalid source '{}'. Valid: claude, codex, factory, opencode", s))
            .map(Some),
        None => Ok(None),
    }
}

fn run_tui(initial_query: String) -> Result<()> {
    // Initialize app (starts background indexing automatically)
    let mut app = App::new(initial_query)?;

    // Initialize terminal
    let mut terminal = tui::init()?;

    // Main event loop
    let result = run(&mut terminal, &mut app);

    // Restore terminal
    tui::restore()?;

    // Print any indexing error
    if let Some(ref err) = app.index_error {
        eprintln!("\nIndexing error:\n  {}\n", err);
        eprintln!("Try: recall --reindex\n");
    }

    // Handle post-exit actions
    if let Some(session) = app.should_resume {
        resume_session(&session)?;
    } else if let Some(session_id) = app.should_copy {
        copy_to_clipboard(&session_id)?;
        println!("Copied session ID: {}", session_id);
    }

    result
}

fn run(terminal: &mut tui::Tui, app: &mut App) -> Result<()> {
    // Track last click for double-click detection
    let mut last_click: Option<(std::time::Instant, u16, u16)> = None;
    const DOUBLE_CLICK_MS: u128 = 400;

    loop {
        // Poll for indexing updates
        app.poll_index_updates();

        // Check for debounced search
        app.maybe_search();

        // Render
        terminal.draw(|frame| ui::render(frame, app))?;

        // Check for exit conditions
        if app.should_quit || app.should_resume.is_some() || app.should_copy.is_some() {
            break;
        }

        // Handle all pending events (drain queue to prevent mouse event flooding)
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                // On Windows, crossterm sends both Press and Release events.
                // Only handle Press to avoid double input.
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.should_quit = true;
                    }
                    KeyCode::Esc => app.on_escape(),
                    KeyCode::Enter => app.on_enter(),
                    KeyCode::Tab => app.on_tab(),
                    KeyCode::Up => app.on_up(),
                    KeyCode::Down => app.on_down(),
                    KeyCode::Left => app.on_left(),
                    KeyCode::Right => app.on_right(),
                    KeyCode::Home => app.on_home(),
                    KeyCode::End => app.on_end(),
                    KeyCode::Delete => app.on_delete(),
                    KeyCode::PageUp => app.focus_prev_message(),
                    KeyCode::PageDown => app.focus_next_message(),
                    KeyCode::Backspace => app.on_backspace(),
                    KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.toggle_focused_expansion();
                    }
                    KeyCode::Char('/') => app.toggle_scope(),
                    KeyCode::Char(c) => app.on_char(c),
                    _ => {}
                },
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => app.scroll_preview_up(3),
                    MouseEventKind::ScrollDown => app.scroll_preview_down(3),
                    MouseEventKind::Down(event::MouseButton::Left) => {
                        let now = std::time::Instant::now();
                        let (x, y) = (mouse.column, mouse.row);

                        // Check for double-click
                        let is_double_click = if let Some((last_time, lx, ly)) = last_click {
                            now.duration_since(last_time).as_millis() < DOUBLE_CLICK_MS
                                && lx == x && ly == y
                        } else {
                            false
                        };

                        if app.click_preview_message(x, y) {
                            if is_double_click {
                                app.toggle_focused_expansion();
                                last_click = None; // Reset after double-click
                            } else {
                                last_click = Some((now, x, y));
                            }
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        // Small sleep to prevent busy loop
        std::thread::sleep(Duration::from_millis(16));
    }

    Ok(())
}

/// Resume a session by exec'ing into the appropriate CLI
#[cfg(unix)]
fn resume_session(session: &session::Session) -> Result<()> {
    use std::os::unix::process::CommandExt;

    // Change to the appropriate directory for resuming
    let resume_cwd = session.resume_cwd();
    if !resume_cwd.is_empty() {
        let _ = std::env::set_current_dir(&resume_cwd);
    }

    let (program, args) = session.resume_command();

    // This replaces the current process - never returns on success
    let err = std::process::Command::new(&program).args(&args).exec();

    // Only reached if exec fails
    anyhow::bail!("Failed to exec {}: {}", program, err)
}

#[cfg(not(unix))]
fn resume_session(session: &session::Session) -> Result<()> {
    // Change to the appropriate directory for resuming
    let resume_cwd = session.resume_cwd();
    if !resume_cwd.is_empty() {
        let _ = std::env::set_current_dir(&resume_cwd);
    }

    let (program, args) = session.resume_command();

    // On non-Unix, just spawn the process
    std::process::Command::new(&program)
        .args(&args)
        .status()?;

    Ok(())
}

/// Copy session ID to clipboard
fn copy_to_clipboard(text: &str) -> Result<()> {
    use arboard::Clipboard;
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text)?;
    Ok(())
}

/// Clear the index cache directory
fn clear_index_cache() {
    let cache_dir = std::env::var("RECALL_HOME_OVERRIDE")
        .map(|h| std::path::PathBuf::from(h).join(".cache").join("recall"))
        .unwrap_or_else(|_| {
            dirs::cache_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("recall")
        });

    if cache_dir.exists() {
        let _ = std::fs::remove_dir_all(&cache_dir);
    }
}
