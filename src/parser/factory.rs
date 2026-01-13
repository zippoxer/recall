use crate::session::{Message, Role, Session, SessionSource};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use super::{join_consecutive_messages, SessionParser};

#[derive(Debug, Deserialize)]
struct FactoryLine {
    #[serde(rename = "type")]
    entry_type: String,
    id: Option<String>,
    #[allow(dead_code)]
    title: Option<String>,
    cwd: Option<String>,
    timestamp: Option<String>,
    message: Option<FactoryMessage>,
}

#[derive(Debug, Deserialize)]
struct FactoryMessage {
    role: String,
    content: serde_json::Value,
}

pub struct FactoryParser;

impl SessionParser for FactoryParser {
    fn can_parse(path: &Path) -> bool {
        // Factory sessions are in ~/.factory/sessions/
        path.to_str()
            .map(|s| s.contains(".factory/sessions") || s.contains(".factory\\sessions"))
            .unwrap_or(false)
    }

    fn parse_file(path: &Path) -> Result<Session> {
        let file = File::open(path).context("Failed to open file")?;
        let reader = BufReader::with_capacity(64 * 1024, file);

        let mut session_id: Option<String> = None;
        let mut cwd: Option<String> = None;
        let mut latest_timestamp: Option<DateTime<Utc>> = None;
        let mut messages: Vec<Message> = Vec::new();

        for line in reader.lines() {
            let line = line.context("Failed to read line")?;
            if line.trim().is_empty() {
                continue;
            }

            let entry: FactoryLine = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => continue, // Skip malformed lines
            };

            match entry.entry_type.as_str() {
                "session_start" => {
                    // Extract session metadata
                    if session_id.is_none() {
                        session_id = entry.id.clone();
                    }
                    if cwd.is_none() {
                        cwd = entry.cwd.clone();
                    }
                }
                "message" => {
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
                                tool_calls: Vec::new(), // TODO: Extract tool calls for Factory
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // Try to extract cwd from parent directory name if not found in session
        if cwd.is_none() {
            cwd = extract_cwd_from_path(path);
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
            source: SessionSource::Factory,
            file_path: path.to_path_buf(),
            cwd: cwd.unwrap_or_else(|| ".".to_string()),
            git_branch: None,
            timestamp: latest_timestamp.unwrap_or_else(Utc::now),
            messages: join_consecutive_messages(messages),
        })
    }
}

/// Extract text content from Factory's message content field.
/// Content is an array of {type, text} objects.
/// Filters out system-reminder blocks which are injected by the CLI.
fn extract_content(content: &serde_json::Value) -> String {
    let serde_json::Value::Array(arr) = content else {
        return String::new();
    };

    let mut texts = Vec::new();
    for item in arr {
        if let Some(obj) = item.as_object() {
            // Only extract "text" type blocks, skip tool_use, tool_result, etc.
            if obj.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    // Skip system-reminder blocks (injected by CLI, not user input)
                    // Must have both opening and closing tags to filter
                    let trimmed = text.trim();
                    if trimmed.starts_with("<system-reminder>")
                        && trimmed.ends_with("</system-reminder>")
                    {
                        continue;
                    }
                    texts.push(text.to_string());
                }
            }
        }
    }
    texts.join("\n")
}

/// Extract cwd from Factory's directory structure.
/// Factory stores sessions in subdirectories like `-Users-zippo-code-recall/`
/// which encodes the cwd path.
fn extract_cwd_from_path(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let dir_name = parent.file_name()?.to_str()?;

    // Check if it's an encoded path (starts with -)
    if dir_name.starts_with('-') {
        // Convert -Users-zippo-code-recall to /Users/zippo/code/recall
        let decoded = dir_name.replacen('-', "/", 1).replace('-', "/");
        Some(decoded)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_content() {
        let content = serde_json::json!([
            {"type": "text", "text": "Hello"},
            {"type": "tool_use", "name": "Read"},
            {"type": "text", "text": "World"}
        ]);
        assert_eq!(extract_content(&content), "Hello\nWorld");
    }

    #[test]
    fn test_extract_content_filters_system_reminders() {
        let content = serde_json::json!([
            {"type": "text", "text": "<system-reminder>\nSome system info\n</system-reminder>"},
            {"type": "text", "text": "<system-reminder>TodoWrite reminder</system-reminder>"},
            {"type": "text", "text": "actual user message"}
        ]);
        assert_eq!(extract_content(&content), "actual user message");
    }

    #[test]
    fn test_extract_content_keeps_partial_system_reminder() {
        // User might ask about system-reminder tags - don't filter if not properly closed
        let content = serde_json::json!([
            {"type": "text", "text": "<system-reminder> what is this tag?"}
        ]);
        assert_eq!(extract_content(&content), "<system-reminder> what is this tag?");
    }

    #[test]
    fn test_extract_cwd_from_path() {
        let path = Path::new("/home/user/.factory/sessions/-Users-zippo-code-recall/abc.jsonl");
        assert_eq!(
            extract_cwd_from_path(path),
            Some("/Users/zippo/code/recall".to_string())
        );
    }

    #[test]
    fn test_extract_cwd_from_path_root() {
        let path = Path::new("/home/user/.factory/sessions/abc.jsonl");
        assert_eq!(extract_cwd_from_path(path), None);
    }
}
