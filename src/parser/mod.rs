mod claude;
mod codex;
mod factory;
mod opencode;

pub use claude::ClaudeParser;
pub use codex::CodexParser;
pub use factory::FactoryParser;
pub use opencode::OpenCodeParser;

use crate::session::{Message, Session};
use anyhow::Result;
use std::path::Path;

/// Join consecutive messages from the same role into single messages.
/// Uses the latest timestamp when joining.
pub fn join_consecutive_messages(messages: Vec<Message>) -> Vec<Message> {
    messages.into_iter().fold(Vec::new(), |mut acc, msg| {
        if let Some(last) = acc.last_mut() {
            if last.role == msg.role {
                last.content.push_str("\n\n");
                last.content.push_str(&msg.content);
                last.timestamp = msg.timestamp; // use latest
                return acc;
            }
        }
        acc.push(msg);
        acc
    })
}

/// Trait for parsing session files
pub trait SessionParser {
    /// Parse a session file into a Session
    fn parse_file(path: &Path) -> Result<Session>;

    /// Check if this parser can handle the given file
    fn can_parse(path: &Path) -> bool;
}

/// Discover all session files from Claude Code, Codex CLI, and Factory
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

        // Factory: ~/.factory/sessions/**/*.jsonl
        let factory_dir = home.join(".factory/sessions");
        if factory_dir.exists() {
            for entry in walkdir::WalkDir::new(&factory_dir)
                .into_iter()
                .flatten()
            {
                let path = entry.path();
                if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                    files.push(path.to_path_buf());
                }
            }
        }

        // OpenCode: ~/.local/share/opencode/storage/session/**/*.json
        let opencode_dir = home.join(".local/share/opencode/storage/session");
        if opencode_dir.exists() {
            for entry in walkdir::WalkDir::new(&opencode_dir)
                .into_iter()
                .flatten()
            {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    // Only include session files (ses_*.json)
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with("ses_") {
                            files.push(path.to_path_buf());
                        }
                    }
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
    } else if FactoryParser::can_parse(path) {
        FactoryParser::parse_file(path)
    } else if OpenCodeParser::can_parse(path) {
        OpenCodeParser::parse_file(path)
    } else {
        anyhow::bail!("Unknown session file format: {:?}", path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Role;
    use chrono::Utc;

    #[test]
    fn test_join_consecutive_messages_different_roles() {
        let now = Utc::now();
        let messages = vec![
            Message { role: Role::User, content: "Hello".to_string(), timestamp: now },
            Message { role: Role::Assistant, content: "Hi".to_string(), timestamp: now },
            Message { role: Role::User, content: "Bye".to_string(), timestamp: now },
        ];
        let joined = join_consecutive_messages(messages);
        assert_eq!(joined.len(), 3);
    }

    #[test]
    fn test_join_consecutive_messages_same_role() {
        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::seconds(10);
        let messages = vec![
            Message { role: Role::User, content: "Part 1".to_string(), timestamp: t1 },
            Message { role: Role::User, content: "Part 2".to_string(), timestamp: t2 },
            Message { role: Role::Assistant, content: "Response".to_string(), timestamp: t2 },
        ];
        let joined = join_consecutive_messages(messages);
        assert_eq!(joined.len(), 2);
        assert_eq!(joined[0].content, "Part 1\n\nPart 2");
        assert_eq!(joined[0].timestamp, t2); // Uses latest timestamp
        assert_eq!(joined[1].content, "Response");
    }

    #[test]
    fn test_join_consecutive_messages_multiple_same_role() {
        let now = Utc::now();
        let messages = vec![
            Message { role: Role::Assistant, content: "A".to_string(), timestamp: now },
            Message { role: Role::Assistant, content: "B".to_string(), timestamp: now },
            Message { role: Role::Assistant, content: "C".to_string(), timestamp: now },
        ];
        let joined = join_consecutive_messages(messages);
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].content, "A\n\nB\n\nC");
    }
}
