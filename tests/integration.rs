use insta::assert_snapshot;
use ratatui::{backend::TestBackend, Terminal};
use std::path::PathBuf;
use std::sync::Mutex;
use tempfile::TempDir;

// Serialize tests since they modify env vars
static TEST_MUTEX: Mutex<()> = Mutex::new(());

fn lock_test() -> std::sync::MutexGuard<'static, ()> {
    // Handle poisoned mutex from failed tests
    TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
}

/// Get the path to test fixtures
fn fixtures_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Setup test environment with fixtures
fn setup_test_env() -> TempDir {
    let temp_dir = TempDir::new().unwrap();

    // Copy fixtures to temp dir
    let fixtures = fixtures_path();
    let temp_path = temp_dir.path();

    // Copy .claude directory
    let claude_src = fixtures.join(".claude");
    let claude_dst = temp_path.join(".claude");
    copy_dir_recursive(&claude_src, &claude_dst);

    // Copy .codex directory
    let codex_src = fixtures.join(".codex");
    let codex_dst = temp_path.join(".codex");
    copy_dir_recursive(&codex_src, &codex_dst);

    temp_dir
}

/// Recursively copy a directory
fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) {
    if !src.exists() {
        return;
    }
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            std::fs::copy(&src_path, &dst_path).unwrap();
        }
    }
}

/// Wait for indexing to complete, polling up to max_polls times
fn wait_for_indexing(app: &mut recall::App, max_polls: usize) {
    for _ in 0..max_polls {
        app.poll_index_updates();
        if !app.indexing {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Check if buffer contains text
fn buffer_contains(terminal: &Terminal<TestBackend>, text: &str) -> bool {
    let buffer = terminal.backend().buffer();
    let content: String = buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    content.contains(text)
}

/// Render app to test terminal
fn render_app(app: &mut recall::App) -> Terminal<TestBackend> {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| recall::ui::render(f, app)).unwrap();
    terminal
}

/// Convert terminal buffer to string for snapshot testing
fn buffer_to_string(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let mut result = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = buffer.cell((x, y)).unwrap();
            result.push_str(cell.symbol());
        }
        // Trim trailing whitespace from each line
        while result.ends_with(' ') {
            result.pop();
        }
        result.push('\n');
    }
    // Remove trailing empty lines
    while result.ends_with("\n\n") {
        result.pop();
    }
    result
}

// =============================================================================
// Tests
// =============================================================================

#[test]
fn test_discovers_claude_sessions() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let files = recall::parser::discover_session_files();

    std::env::remove_var("RECALL_HOME_OVERRIDE");

    assert!(!files.is_empty(), "Should discover Claude session files");
    assert!(
        files.iter().any(|f| f.to_string_lossy().contains(".claude/projects")),
        "Should find files in .claude/projects"
    );
}

#[test]
fn test_discovers_codex_sessions() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let files = recall::parser::discover_session_files();

    std::env::remove_var("RECALL_HOME_OVERRIDE");

    assert!(
        files.iter().any(|f| f.to_string_lossy().contains(".codex/sessions")),
        "Should find files in .codex/sessions"
    );
}

#[test]
fn test_search_finds_matching_content() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    // Toggle to everywhere scope (CWD won't match fixtures)
    app.toggle_scope();

    // Search for content from Claude fixture
    for c in "hello".chars() {
        app.on_char(c);
    }

    std::env::remove_var("RECALL_HOME_OVERRIDE");

    assert!(!app.results.is_empty(), "Should find results for 'hello'");
    assert!(
        app.results.iter().any(|r| r.session.id == "test-claude-123"),
        "Should find Claude session"
    );
}

#[test]
fn test_search_no_results_shows_hint() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    // Toggle to everywhere then back to folder scope to ensure we're scoped
    app.toggle_scope(); // now everywhere
    app.toggle_scope(); // now folder

    // Search for something that doesn't exist
    for c in "xyznonexistent".chars() {
        app.on_char(c);
    }

    let terminal = render_app(&mut app);

    std::env::remove_var("RECALL_HOME_OVERRIDE");

    assert!(app.results.is_empty(), "Should have no results");
    // When scoped with no results, shows "No results. Press / to search everywhere."
    assert!(
        buffer_contains(&terminal, "No results"),
        "Should show 'No results' hint"
    );
}

#[test]
fn test_navigation_up_down() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    // Toggle to everywhere to see all sessions
    app.toggle_scope();

    std::env::remove_var("RECALL_HOME_OVERRIDE");

    if app.results.len() >= 2 {
        assert_eq!(app.selected, 0, "Should start at first result");

        app.on_down();
        assert_eq!(app.selected, 1, "Should move to second result");

        app.on_up();
        assert_eq!(app.selected, 0, "Should move back to first result");
    }
}

#[test]
fn test_toggle_scope() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    // Should start in folder scope
    assert!(matches!(app.search_scope, recall::SearchScope::Folder(_)));

    // Toggle to everywhere
    app.toggle_scope();
    assert!(matches!(app.search_scope, recall::SearchScope::Everything));

    // Toggle back
    app.toggle_scope();
    assert!(matches!(app.search_scope, recall::SearchScope::Folder(_)));

    std::env::remove_var("RECALL_HOME_OVERRIDE");
}

#[test]
fn test_renders_status_bar() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    let terminal = render_app(&mut app);

    std::env::remove_var("RECALL_HOME_OVERRIDE");

    // Status bar should show session count
    assert!(
        buffer_contains(&terminal, "sessions"),
        "Should show session count in status bar"
    );
}

#[test]
fn test_search_during_indexing() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    // Create app but don't wait for full indexing
    let mut app = recall::App::new(String::new()).unwrap();

    // Poll just once to start processing
    app.poll_index_updates();

    // Should be able to search even during indexing
    app.on_char('t');
    app.on_char('e');
    app.on_char('s');
    app.on_char('t');

    let terminal = render_app(&mut app);

    std::env::remove_var("RECALL_HOME_OVERRIDE");

    // Should render without crashing
    assert!(terminal.backend().buffer().area.width > 0);
}

#[test]
fn test_escape_clears_query() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    // Type a query
    app.on_char('t');
    app.on_char('e');
    app.on_char('s');
    app.on_char('t');
    assert_eq!(app.query, "test");

    // Escape should clear
    app.on_escape();
    assert!(app.query.is_empty(), "Escape should clear query");
    assert!(!app.should_quit, "First escape should not quit");

    // Second escape should quit
    app.on_escape();
    assert!(app.should_quit, "Second escape should quit");

    std::env::remove_var("RECALL_HOME_OVERRIDE");
}

#[test]
fn test_backspace_removes_char() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let mut app = recall::App::new(String::new()).unwrap();

    app.on_char('a');
    app.on_char('b');
    app.on_char('c');
    assert_eq!(app.query, "abc");

    app.on_backspace();
    assert_eq!(app.query, "ab");

    std::env::remove_var("RECALL_HOME_OVERRIDE");
}

#[test]
fn test_initial_query() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());

    let app = recall::App::new("initial".to_string()).unwrap();

    std::env::remove_var("RECALL_HOME_OVERRIDE");

    assert_eq!(app.query, "initial", "Should have initial query");
}

// =============================================================================
// UI Snapshot Tests
// =============================================================================

// Note: We only snapshot "no results" states because result ordering from Tantivy
// is non-deterministic, making snapshots with results flaky.

const TEST_CWD: &str = "/test/cwd";

fn setup_ui_test() -> TempDir {
    let temp_dir = setup_test_env();
    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());
    std::env::set_var("RECALL_CWD_OVERRIDE", TEST_CWD);
    temp_dir
}

fn cleanup_ui_test() {
    std::env::remove_var("RECALL_HOME_OVERRIDE");
    std::env::remove_var("RECALL_CWD_OVERRIDE");
}

#[test]
fn test_ui_no_query_folder_scope() {
    let _lock = lock_test();
    let _temp_dir = setup_ui_test();

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    // Stay in folder scope (no sessions match CWD)
    let terminal = render_app(&mut app);

    cleanup_ui_test();

    assert_snapshot!(buffer_to_string(&terminal));
}

#[test]
fn test_ui_no_query_everywhere_scope() {
    let _lock = lock_test();
    // Use empty temp dir (no fixtures) so there are no results
    let temp_dir = TempDir::new().unwrap();
    std::fs::create_dir_all(temp_dir.path().join(".claude/projects")).unwrap();
    std::fs::create_dir_all(temp_dir.path().join(".codex/sessions")).unwrap();

    std::env::set_var("RECALL_HOME_OVERRIDE", temp_dir.path());
    std::env::set_var("RECALL_CWD_OVERRIDE", TEST_CWD);

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    // Toggle to everywhere scope
    app.toggle_scope();

    let terminal = render_app(&mut app);

    cleanup_ui_test();

    assert_snapshot!(buffer_to_string(&terminal));
}

#[test]
fn test_ui_with_query_folder_scope_no_results() {
    let _lock = lock_test();
    let _temp_dir = setup_ui_test();

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    // Stay in folder scope and search
    for c in "zzzznotfound".chars() {
        app.on_char(c);
    }

    let terminal = render_app(&mut app);

    cleanup_ui_test();

    assert_snapshot!(buffer_to_string(&terminal));
}

#[test]
fn test_ui_with_query_everywhere_scope_no_results() {
    let _lock = lock_test();
    let _temp_dir = setup_ui_test();

    let mut app = recall::App::new(String::new()).unwrap();
    wait_for_indexing(&mut app, 100);

    // Toggle to everywhere and search for something that doesn't exist
    app.toggle_scope();
    for c in "zzzznotfound".chars() {
        app.on_char(c);
    }

    let terminal = render_app(&mut app);

    cleanup_ui_test();

    assert_snapshot!(buffer_to_string(&terminal));
}
