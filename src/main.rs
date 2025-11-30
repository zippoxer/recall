use anyhow::Result;
use recall::{app::App, session, tui, ui};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> Result<()> {
    // Handle --help and --version
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("recall {}", VERSION);
        return Ok(());
    }

    // Collect remaining args as initial search query
    let initial_query = args.join(" ");

    // Initialize app (starts background indexing automatically)
    let mut app = App::new(initial_query)?;

    // Initialize terminal
    let mut terminal = tui::init()?;

    // Main event loop
    let result = run(&mut terminal, &mut app);

    // Restore terminal
    tui::restore()?;

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
    loop {
        // Poll for indexing updates
        app.poll_index_updates();

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
                    KeyCode::PageUp => app.scroll_preview_up(15),
                    KeyCode::PageDown => app.scroll_preview_down(15),
                    KeyCode::Backspace => app.on_backspace(),
                    KeyCode::Char('/') => app.toggle_scope(),
                    KeyCode::Char(c) => app.on_char(c),
                    _ => {}
                },
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => app.scroll_preview_up(3),
                    MouseEventKind::ScrollDown => app.scroll_preview_down(3),
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

    // Change to conversation's working directory
    if !session.cwd.is_empty() {
        let _ = std::env::set_current_dir(&session.cwd);
    }

    let (program, args) = session.resume_command();

    // This replaces the current process - never returns on success
    let err = std::process::Command::new(&program).args(&args).exec();

    // Only reached if exec fails
    anyhow::bail!("Failed to exec {}: {}", program, err)
}

#[cfg(not(unix))]
fn resume_session(session: &session::Session) -> Result<()> {
    // Change to conversation's working directory
    if !session.cwd.is_empty() {
        let _ = std::env::set_current_dir(&session.cwd);
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

fn print_help() {
    println!(
        "recall {} - Search and resume Claude Code and Codex CLI conversations

Usage: recall [query]

Examples:
  recall
  recall foo
  recall foo bar

Options:
  -h, --help     Print help
  -V, --version  Print version",
        VERSION
    );
}
