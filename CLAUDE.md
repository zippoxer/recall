# CLAUDE.md

## Purpose

Search and resume past conversations from Claude Code and Codex CLI.

## Principles

- **Delightful** - Made with love, feels good to use
- **Simple, uncluttered, focused** - Two panes, keyboard-driven, no chrome
- **Responsive** - Instant startup via background indexing, sub-100ms search
- **Match-recency matters** - Rank by most recent message containing the match (human memory anchors to recent context)
- **Seamless resume** - Enter execs directly into the CLI, no intermediate steps

## Development

```bash
cargo check          # Fast compile check (no binary)
cargo run            # Build debug + run
cargo test           # Run tests
cargo clippy         # Lint
```

To test the TUI end-to-end, use tmux:
```bash
cargo build && tmux new-session -d -s test './target/debug/recall'
tmux send-keys -t test 'search query'
tmux capture-pane -t test -p          # See output
tmux kill-session -t test             # Cleanup
```

## Install

```bash
cargo install --path .
```

## Architecture

Rust TUI for searching Claude Code and Codex CLI conversation history.

- `src/main.rs` - Entry point, event loop, exec into CLI on resume
- `src/app.rs` - Application state, search logic, background indexing thread
- `src/ui.rs` - Two-pane ratatui rendering, match highlighting
- `src/tui.rs` - Terminal setup/teardown
- `src/theme.rs` - Light/dark theme with auto-detection
- `src/session.rs` - Core types: Session, Message, SearchResult
- `src/parser/` - JSONL parsers for Claude (`~/.claude/projects/`) and Codex (`~/.codex/sessions/`)
- `src/index/` - Tantivy full-text search index, stored in `~/.cache/recall/`

## Key Patterns

- Background indexing: spawns thread on startup, indexes most recent files first, sends progress via mpsc channel
- Unicode-safe string handling: use char indices not byte indices when slicing (see `highlight_matches`, `create_snippet`)
- Search ranking: combines BM25 relevance with recency boost (exponential decay, 7-day half-life)
- Theme detection: queries terminal bg color via crossterm, falls back to COLORFGBG env var
- Event handling: drains all pending events each frame to prevent mouse event flooding
- Contextual status bar: hints adapt to state (e.g., scroll hint only when preview is scrollable)
