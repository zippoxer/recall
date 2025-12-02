use crate::session::{Message, Role, Session, SessionSource};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use super::{join_consecutive_messages, SessionParser};

#[derive(Debug, Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    timestamp: Option<String>,
    message: Option<ClaudeMessage>,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    role: String,
    content: serde_json::Value,
}

pub struct ClaudeParser;

impl SessionParser for ClaudeParser {
    fn can_parse(path: &Path) -> bool {
        // Claude Code sessions are in ~/.claude/projects/
        path.to_str()
            .map(|s| s.contains(".claude/projects"))
            .unwrap_or(false)
    }

    fn parse_file(path: &Path) -> Result<Session> {
        let file = File::open(path).context("Failed to open file")?;
        let reader = BufReader::with_capacity(64 * 1024, file);

        let mut session_id: Option<String> = None;
        let mut cwd: Option<String> = None;
        let mut git_branch: Option<String> = None;
        let mut latest_timestamp: Option<DateTime<Utc>> = None;
        let mut messages: Vec<Message> = Vec::new();

        for line in reader.lines() {
            let line = line.context("Failed to read line")?;
            if line.trim().is_empty() {
                continue;
            }

            let entry: ClaudeLine = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => continue, // Skip malformed lines
            };

            // Skip non-message entries
            if entry.entry_type != "user" && entry.entry_type != "assistant" {
                continue;
            }

            // Extract session metadata from the first valid message
            if session_id.is_none() {
                session_id = entry.session_id.clone();
            }
            if cwd.is_none() {
                cwd = entry.cwd.clone();
            }
            if git_branch.is_none() {
                git_branch = entry.git_branch.clone();
            }

            // Parse timestamp
            let timestamp = entry
                .timestamp
                .as_ref()
                .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);

            // Update latest timestamp
            if latest_timestamp.is_none() || timestamp > latest_timestamp.unwrap() {
                latest_timestamp = Some(timestamp);
            }

            // Extract message content
            if let Some(msg) = &entry.message {
                let role = match msg.role.as_str() {
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    _ => continue,
                };

                let content = extract_content(&msg.content);
                if !content.is_empty() {
                    messages.push(Message {
                        role,
                        content,
                        timestamp,
                    });
                }
            }
        }

        // Fall back to filename for session ID if not found
        let session_id = session_id.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

        Ok(Session {
            id: session_id,
            source: SessionSource::ClaudeCode,
            file_path: path.to_path_buf(),
            cwd: cwd.unwrap_or_else(|| ".".to_string()),
            git_branch,
            timestamp: latest_timestamp.unwrap_or_else(Utc::now),
            messages: join_consecutive_messages(messages),
        })
    }
}

/// Extract text content from Claude's message content field.
/// - User messages: content is a plain string
/// - Assistant messages: content is an array of {type, text} objects
fn extract_content(content: &serde_json::Value) -> String {
    match content {
        // Direct string (user messages)
        serde_json::Value::String(s) => s.clone(),

        // Array of content blocks (assistant messages)
        serde_json::Value::Array(arr) => {
            let mut texts = Vec::new();
            for item in arr {
                if let Some(obj) = item.as_object() {
                    // Only extract "text" type blocks, skip tool_use, thinking, etc.
                    if obj.get("type").and_then(|v| v.as_str()) == Some("text") {
                        if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                            texts.push(text.to_string());
                        }
                    }
                }
            }
            texts.join("\n")
        }

        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_content_string() {
        let content = serde_json::json!("Hello, world!");
        assert_eq!(extract_content(&content), "Hello, world!");
    }

    #[test]
    fn test_extract_content_array() {
        let content = serde_json::json!([
            {"type": "text", "text": "Hello"},
            {"type": "tool_use", "name": "Read"},
            {"type": "text", "text": "World"}
        ]);
        assert_eq!(extract_content(&content), "Hello\nWorld");
    }

}
