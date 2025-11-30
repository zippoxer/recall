mod claude;
mod codex;

pub use claude::ClaudeParser;
pub use codex::CodexParser;

use crate::session::Session;
use anyhow::Result;
use std::path::Path;

/// Trait for parsing session files
pub trait SessionParser {
    /// Parse a session file into a Session
    fn parse_file(path: &Path) -> Result<Session>;

    /// Check if this parser can handle the given file
    fn can_parse(path: &Path) -> bool;
}

/// Discover all session files from both Claude Code and Codex CLI
pub fn discover_session_files() -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    // Allow override for testing
    let home = std::env::var("RECALL_HOME_OVERRIDE")
        .map(std::path::PathBuf::from)
        .ok()
        .or_else(dirs::home_dir);

    if let Some(home) = home {
        // Claude Code: ~/.claude/projects/*/*.jsonl
        let claude_dir = home.join(".claude/projects");
        if claude_dir.exists() {
            if let Ok(projects) = std::fs::read_dir(&claude_dir) {
                for project in projects.flatten() {
                    if let Ok(sessions) = std::fs::read_dir(project.path()) {
                        for session in sessions.flatten() {
                            let path = session.path();
                            if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                                // Skip agent sidechain files (internal subagent conversations)
                                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                    if name.starts_with("agent-") {
                                        continue;
                                    }
                                }
                                files.push(path);
                            }
                        }
                    }
                }
            }
        }

        // Codex CLI: ~/.codex/sessions/**/*.jsonl
        let codex_dir = home.join(".codex/sessions");
        if codex_dir.exists() {
            for entry in walkdir::WalkDir::new(&codex_dir)
                .into_iter()
                .flatten()
            {
                let path = entry.path();
                if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                    files.push(path.to_path_buf());
                }
            }
        }
    }

    files
}

/// Parse a session file, auto-detecting the format
pub fn parse_session_file(path: &Path) -> Result<Session> {
    if ClaudeParser::can_parse(path) {
        ClaudeParser::parse_file(path)
    } else if CodexParser::can_parse(path) {
        CodexParser::parse_file(path)
    } else {
        anyhow::bail!("Unknown session file format: {:?}", path)
    }
}
