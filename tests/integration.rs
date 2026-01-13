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
    app.flush_pending_search();

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
    app.flush_pending_search();

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
    app.flush_pending_search();

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
    app.flush_pending_search();

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
    app.flush_pending_search();

    let terminal = render_app(&mut app);

    cleanup_ui_test();

    assert_snapshot!(buffer_to_string(&terminal));
}

// =============================================================================
// CLI Integration Tests
// =============================================================================

use std::process::Command;

fn recall_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_recall"))
}

fn run_cli(args: &[&str], home_override: &std::path::Path) -> (String, String, bool) {
    let output = Command::new(recall_bin())
        .args(args)
        .env("RECALL_HOME_OVERRIDE", home_override)
        .output()
        .expect("Failed to run recall");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

#[test]
fn test_cli_search_returns_json() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["search", "hello", "--limit", "5"],
        temp_dir.path(),
    );

    assert!(success, "CLI search should succeed");

    // Parse as JSON
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");

    assert_eq!(json["query"], "hello");
    assert!(json["results"].is_array());
}

#[test]
fn test_cli_search_finds_fixture_content() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["search", "hello", "--limit", "10"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = json["results"].as_array().unwrap();

    // Should find the Claude fixture session
    assert!(
        results.iter().any(|r| r["session_id"] == "test-claude-123"),
        "Should find Claude fixture session"
    );
}

#[test]
fn test_cli_search_with_source_filter() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["search", "hello", "--source", "claude", "--limit", "10"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = json["results"].as_array().unwrap();

    // All results should be Claude
    for result in results {
        assert_eq!(result["source"], "claude");
    }
}

#[test]
fn test_cli_search_no_results() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["search", "xyznonexistent12345"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = json["results"].as_array().unwrap();

    assert!(results.is_empty(), "Should have no results for nonexistent query");
}

#[test]
fn test_cli_list_returns_json() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["list", "--limit", "5"],
        temp_dir.path(),
    );

    assert!(success, "CLI list should succeed");

    let json: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");

    assert!(json["sessions"].is_array());
}

#[test]
fn test_cli_list_with_source_filter() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["list", "--source", "codex", "--limit", "10"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let sessions = json["sessions"].as_array().unwrap();

    // All sessions should be Codex
    for session in sessions {
        assert_eq!(session["source"], "codex");
    }
}

#[test]
fn test_cli_read_returns_session() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["read", "test-claude-123"],
        temp_dir.path(),
    );

    assert!(success, "CLI read should succeed");

    let json: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");

    assert_eq!(json["session_id"], "test-claude-123");
    assert_eq!(json["source"], "claude");
    assert!(json["messages"].is_array());
    assert!(!json["messages"].as_array().unwrap().is_empty());
}

#[test]
fn test_cli_read_nonexistent_session() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (_stdout, stderr, success) = run_cli(
        &["read", "nonexistent-session-id"],
        temp_dir.path(),
    );

    assert!(!success, "Should fail for nonexistent session");
    assert!(stderr.contains("Session not found"), "Should show error message");
}

#[test]
fn test_cli_invalid_source() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (_stdout, stderr, success) = run_cli(
        &["search", "test", "--source", "invalid"],
        temp_dir.path(),
    );

    assert!(!success, "Should fail for invalid source");
    assert!(stderr.contains("Invalid source"), "Should show error message");
}

#[test]
fn test_cli_help() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["--help"],
        temp_dir.path(),
    );

    assert!(success);
    assert!(stdout.contains("search"));
    assert!(stdout.contains("list"));
    assert!(stdout.contains("read"));
}

#[test]
fn test_cli_search_with_cwd_filter() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Search with matching cwd
    let (stdout, _stderr, success) = run_cli(
        &["search", "hello", "--cwd", "/test/project", "--limit", "10"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = json["results"].as_array().unwrap();

    // Should find results with matching cwd
    assert!(!results.is_empty(), "Should find results with matching cwd");
    for result in results {
        assert_eq!(result["cwd"], "/test/project");
    }
}

#[test]
fn test_cli_search_with_cwd_filter_no_match() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Search with non-matching cwd
    let (stdout, _stderr, success) = run_cli(
        &["search", "hello", "--cwd", "/nonexistent/path", "--limit", "10"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = json["results"].as_array().unwrap();

    assert!(results.is_empty(), "Should have no results for non-matching cwd");
}

#[test]
fn test_cli_list_with_cwd_filter() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // List with matching cwd
    let (stdout, _stderr, success) = run_cli(
        &["list", "--cwd", "/test/project", "--limit", "10"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let sessions = json["sessions"].as_array().unwrap();

    // Should find sessions with matching cwd
    for session in sessions {
        assert_eq!(session["cwd"], "/test/project");
    }
}

// =============================================================================
// CLI Tests for Selectors and Tool Calls
// =============================================================================

#[test]
fn test_cli_read_with_tool_calls() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456"],
        temp_dir.path(),
    );

    assert!(success, "CLI read should succeed");

    let json: serde_json::Value = serde_json::from_str(&stdout)
        .expect("Output should be valid JSON");

    assert_eq!(json["session_id"], "test-with-tools-456");

    let messages = json["messages"].as_array().unwrap();

    // Message 2 (assistant) should have a tool call
    let msg2 = &messages[1];
    assert_eq!(msg2["role"], "assistant");
    let tool_calls = msg2["tool_calls"].as_array().unwrap();
    assert!(!tool_calls.is_empty(), "Should have tool calls");

    // Check first tool call structure
    let tool = &tool_calls[0];
    assert_eq!(tool["name"], "Bash");
    assert_eq!(tool["status"], "success");
    assert!(tool["duration_ms"].is_number());
    assert!(tool["output"]["content"].is_string());
}

#[test]
fn test_cli_read_message_selector_single() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Get message 2 only
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456:2"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Should have only message 2
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "assistant");
}

#[test]
fn test_cli_read_message_selector_range() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Get messages 2-4
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456:2-4"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Should have messages 2, 3, 4
    assert_eq!(messages.len(), 3);
}

#[test]
fn test_cli_read_message_selector_last() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Get last 2 messages
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456:-2"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Should have exactly 2 messages
    assert_eq!(messages.len(), 2);
}

#[test]
fn test_cli_read_message_selector_errors() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Get only messages with error tool calls
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456:errors"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Should find the message with the error tool call
    let has_error = messages.iter().any(|m| {
        m["tool_calls"]
            .as_array()
            .map(|calls| calls.iter().any(|c| c["status"] == "error"))
            .unwrap_or(false)
    });
    assert!(has_error, "Should include message with error tool call");
}

#[test]
fn test_cli_read_tool_selector() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Get specific tool call (message 2, tool 1)
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456:2.1"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Should return the message containing the tool
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "assistant");

    let tool_calls = messages[0]["tool_calls"].as_array().unwrap();
    assert!(!tool_calls.is_empty());
    assert_eq!(tool_calls[0]["name"], "Bash");
}

#[test]
fn test_cli_read_context_after() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Get message 2 with 1 message after
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456:2", "-A", "1"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Should have messages 2 and 3
    assert_eq!(messages.len(), 2);
}

#[test]
fn test_cli_read_context_before() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Get message 3 with 1 message before
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456:3", "-B", "1"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Should have messages 2 and 3
    assert_eq!(messages.len(), 2);
}

#[test]
fn test_cli_read_context_both() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Get message 3 with 1 message of context on each side
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456:3", "-C", "1"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Should have messages 2, 3, and 4
    assert_eq!(messages.len(), 3);
}

#[test]
fn test_cli_read_pretty_output() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456", "--pretty"],
        temp_dir.path(),
    );

    assert!(success, "CLI read --pretty should succeed");

    // Pretty output should have gutter format
    assert!(stdout.contains("│"), "Should have gutter separator");
    assert!(stdout.contains("test-with-tools-456"), "Should have session ID in header");
    assert!(stdout.contains("resume:"), "Should have resume footer");

    // Should show tool call status icons
    assert!(stdout.contains("✓") || stdout.contains("✗"), "Should have status icons");
}

#[test]
fn test_cli_read_invalid_selector() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Invalid message index
    let (_stdout, stderr, success) = run_cli(
        &["read", "test-with-tools-456:999"],
        temp_dir.path(),
    );

    assert!(!success, "Should fail for invalid message index");
    assert!(stderr.contains("not found"), "Should show error message");
}

#[test]
fn test_cli_read_invalid_tool_selector() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Invalid tool index
    let (_stdout, stderr, success) = run_cli(
        &["read", "test-with-tools-456:2.99"],
        temp_dir.path(),
    );

    assert!(!success, "Should fail for invalid tool index");
    assert!(stderr.contains("Tool"), "Should show tool error message");
}

#[test]
fn test_cli_read_tool_output_content() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Find the Bash tool call and verify output
    for msg in messages {
        if let Some(tool_calls) = msg["tool_calls"].as_array() {
            for tool in tool_calls {
                if tool["name"] == "Bash" {
                    let output = &tool["output"];
                    assert!(output["content"].as_str().unwrap().contains("main.rs"),
                        "Bash output should contain ls result");
                    assert_eq!(output["truncated"], false);
                }
            }
        }
    }
}

#[test]
fn test_cli_read_error_tool_call() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    let (stdout, _stderr, success) = run_cli(
        &["read", "test-with-tools-456"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Find the error tool call
    let mut found_error = false;
    for msg in messages {
        if let Some(tool_calls) = msg["tool_calls"].as_array() {
            for tool in tool_calls {
                if tool["status"] == "error" {
                    found_error = true;
                    assert!(tool["output"]["content"].as_str().unwrap().contains("No such file"),
                        "Error output should contain error message");
                }
            }
        }
    }
    assert!(found_error, "Should have found error tool call");
}

#[test]
fn test_cli_read_truncation() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // Without --full, large output should be truncated
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-large-output-789"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Find the tool call with large output
    let mut found_truncated = false;
    for msg in messages {
        if let Some(tool_calls) = msg["tool_calls"].as_array() {
            for tool in tool_calls {
                if let Some(output) = tool.get("output") {
                    if output["truncated"] == true {
                        found_truncated = true;
                        let content = output["content"].as_str().unwrap();
                        assert!(content.contains("START_MARKER"), "Should keep start");
                        assert!(content.contains("END_MARKER"), "Should keep end");
                        assert!(content.contains("truncated"), "Should have truncation marker");
                        // Middle should be truncated
                        assert!(!content.contains("MIDDLE_MARKER"), "Middle should be truncated");
                    }
                }
            }
        }
    }
    assert!(found_truncated, "Should have found truncated tool output");
}

#[test]
fn test_cli_read_full_flag() {
    let _lock = lock_test();
    let temp_dir = setup_test_env();

    // With --full, large output should NOT be truncated
    let (stdout, _stderr, success) = run_cli(
        &["read", "test-large-output-789", "--full"],
        temp_dir.path(),
    );

    assert!(success);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let messages = json["messages"].as_array().unwrap();

    // Find the tool call with large output
    let mut found_full = false;
    for msg in messages {
        if let Some(tool_calls) = msg["tool_calls"].as_array() {
            for tool in tool_calls {
                if let Some(output) = tool.get("output") {
                    let content = output["content"].as_str().unwrap();
                    if content.contains("START_MARKER") {
                        found_full = true;
                        assert_eq!(output["truncated"], false, "Should not be truncated with --full");
                        assert!(content.contains("MIDDLE_MARKER"), "Should contain full middle content");
                        assert!(content.contains("END_MARKER"), "Should contain end");
                        assert!(!content.contains("[...truncated"), "Should not have truncation marker");
                    }
                }
            }
        }
    }
    assert!(found_full, "Should have found full tool output");
}
