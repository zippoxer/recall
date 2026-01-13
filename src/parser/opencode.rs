use crate::session::{Message, Role, Session, SessionSource};
use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use super::{join_consecutive_messages, SessionParser};

/// OpenCode session metadata from session/<project_id>/ses_*.json
#[derive(Debug, Deserialize)]
struct OpenCodeSession {
    id: String,
    #[serde(rename = "projectID")]
    #[allow(dead_code)]
    project_id: Option<String>,
    directory: Option<String>,
    #[allow(dead_code)]
    title: Option<String>,
    time: Option<TimeInfo>,
}

/// OpenCode message metadata from message/ses_*/msg_*.json
#[derive(Debug, Deserialize)]
struct OpenCodeMessage {
    id: String,
    #[serde(rename = "sessionID")]
    #[allow(dead_code)]
    session_id: String,
    role: String,
    time: Option<TimeInfo>,
    #[serde(rename = "parentID")]
    #[allow(dead_code)]
    parent_id: Option<String>,
    path: Option<PathInfo>,
}

/// Time information with millisecond timestamps
#[derive(Debug, Deserialize)]
struct TimeInfo {
    created: i64,
    #[allow(dead_code)]
    updated: Option<i64>,
}

/// Path information from assistant messages
#[derive(Debug, Deserialize)]
struct PathInfo {
    cwd: Option<String>,
    #[allow(dead_code)]
    root: Option<String>,
}

/// OpenCode part (content) from part/msg_*/prt_*.json
#[derive(Debug, Deserialize)]
struct OpenCodePart {
    #[allow(dead_code)]
    id: String,
    #[serde(rename = "type")]
    part_type: String,
    text: Option<String>,
}

pub struct OpenCodeParser;

impl SessionParser for OpenCodeParser {
    fn can_parse(path: &Path) -> bool {
        // OpenCode sessions are in ~/.local/share/opencode/storage/session/
        path.to_str()
            .map(|s| s.contains(".local/share/opencode/storage/session"))
            .unwrap_or(false)
    }

    fn parse_file(path: &Path) -> Result<Session> {
        // 1. Read session JSON
        let file = File::open(path).context("Failed to open session file")?;
        let reader = BufReader::new(file);
        let session: OpenCodeSession =
            serde_json::from_reader(reader).context("Failed to parse session JSON")?;

        // 2. Get storage root (go up from session/<project>/ses_*.json to storage/)
        let storage_root = get_storage_root(path).context("Failed to get storage root")?;

        // 3. Find and read all messages for this session
        let message_dir = storage_root.join("message").join(&session.id);
        let mut messages: Vec<Message> = Vec::new();
        let mut latest_timestamp: Option<DateTime<Utc>> = None;
        let mut cwd: Option<String> = session.directory.clone();

        if message_dir.exists() {
            // Collect and sort message files by creation time
            let mut msg_entries: Vec<(PathBuf, OpenCodeMessage)> = Vec::new();

            if let Ok(entries) = std::fs::read_dir(&message_dir) {
                for entry in entries.flatten() {
                    let msg_path = entry.path();
                    if msg_path.extension().map(|e| e == "json").unwrap_or(false) {
                        if let Ok(file) = File::open(&msg_path) {
                            let reader = BufReader::new(file);
                            if let Ok(msg) = serde_json::from_reader::<_, OpenCodeMessage>(reader) {
                                msg_entries.push((msg_path, msg));
                            }
                        }
                    }
                }
            }

            // Sort by creation time
            msg_entries.sort_by(|a, b| {
                let time_a = a.1.time.as_ref().map(|t| t.created).unwrap_or(0);
                let time_b = b.1.time.as_ref().map(|t| t.created).unwrap_or(0);
                time_a.cmp(&time_b)
            });

            // Process each message
            for (_msg_path, msg) in msg_entries {
                // Get timestamp
                let timestamp = msg
                    .time
                    .as_ref()
                    .map(|t| millis_to_datetime(t.created))
                    .unwrap_or_else(Utc::now);

                // Update latest timestamp
                if latest_timestamp.is_none() || timestamp > latest_timestamp.unwrap() {
                    latest_timestamp = Some(timestamp);
                }

                // Get cwd from message path info if available
                if cwd.is_none() {
                    if let Some(path_info) = &msg.path {
                        cwd = path_info.cwd.clone();
                    }
                }

                // Determine role
                let role = match msg.role.as_str() {
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    _ => continue, // Skip unknown roles
                };

                // Read parts for this message
                let content = read_message_parts(&storage_root, &msg.id);
                if !content.is_empty() {
                    messages.push(Message {
                        role,
                        content,
                        timestamp,
                        tool_calls: Vec::new(), // TODO: Extract tool calls for OpenCode
                    });
                }
            }
        }

        Ok(Session {
            id: session.id,
            source: SessionSource::OpenCode,
            file_path: path.to_path_buf(),
            cwd: cwd.unwrap_or_else(|| ".".to_string()),
            git_branch: None, // OpenCode doesn't store git branch in session metadata
            timestamp: latest_timestamp.unwrap_or_else(|| {
                session
                    .time
                    .as_ref()
                    .map(|t| millis_to_datetime(t.created))
                    .unwrap_or_else(Utc::now)
            }),
            messages: join_consecutive_messages(messages),
        })
    }
}

/// Get the storage root directory from a session file path
/// Path: storage/session/<project_id>/ses_*.json
/// Returns: storage/
fn get_storage_root(session_path: &Path) -> Option<PathBuf> {
    session_path
        .parent()? // ses_*.json -> <project_id>/
        .parent()? // <project_id> -> session/
        .parent() // session -> storage/
        .map(|p| p.to_path_buf())
}

/// Convert milliseconds timestamp to DateTime<Utc>
fn millis_to_datetime(millis: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(millis).single().unwrap_or_else(Utc::now)
}

/// Read all text parts for a message and concatenate them
fn read_message_parts(storage_root: &Path, message_id: &str) -> String {
    let parts_dir = storage_root.join("part").join(message_id);
    let mut texts: Vec<String> = Vec::new();

    if !parts_dir.exists() {
        return String::new();
    }

    // Read all part files
    let mut part_entries: Vec<(String, OpenCodePart)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&parts_dir) {
        for entry in entries.flatten() {
            let part_path = entry.path();
            if part_path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(file) = File::open(&part_path) {
                    let reader = BufReader::new(file);
                    if let Ok(part) = serde_json::from_reader::<_, OpenCodePart>(reader) {
                        let filename = part_path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        part_entries.push((filename, part));
                    }
                }
            }
        }
    }

    // Sort by filename to maintain order (prt_* IDs are sortable)
    part_entries.sort_by(|a, b| a.0.cmp(&b.0));

    // Extract text from text parts only
    for (_filename, part) in part_entries {
        if part.part_type == "text" {
            if let Some(text) = part.text {
                if !text.is_empty() {
                    texts.push(text);
                }
            }
        }
        // Skip step-start, step-finish, tool parts (per user preference)
    }

    texts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_parse_opencode_path() {
        assert!(OpenCodeParser::can_parse(Path::new(
            "/home/user/.local/share/opencode/storage/session/project123/ses_abc.json"
        )));
        assert!(!OpenCodeParser::can_parse(Path::new(
            "/home/user/.claude/projects/foo/session.jsonl"
        )));
        assert!(!OpenCodeParser::can_parse(Path::new(
            "/home/user/.codex/sessions/session.jsonl"
        )));
    }

    #[test]
    fn test_millis_to_datetime() {
        let dt = millis_to_datetime(1763499168814);
        assert!(dt.timestamp_millis() == 1763499168814);
    }

    #[test]
    fn test_get_storage_root() {
        let path = Path::new("/home/user/.local/share/opencode/storage/session/proj/ses_123.json");
        let root = get_storage_root(path);
        assert_eq!(
            root,
            Some(PathBuf::from(
                "/home/user/.local/share/opencode/storage"
            ))
        );
    }
}

#[cfg(test)]
mod real_data_tests {
    use super::*;
    
    #[test]
    #[ignore] // Run with: cargo test test_parse_real_opencode -- --ignored --nocapture
    fn test_parse_real_opencode() {
        let home = std::env::var("HOME").unwrap();
        let session_path = format!("{}/.local/share/opencode/storage/session/global/ses_5675050f7ffeivkIg0jm0b0D30.json", home);
        let path = std::path::Path::new(&session_path);
        
        println!("Testing path: {}", session_path);
        println!("Path exists: {}", path.exists());
        
        if path.exists() {
            match OpenCodeParser::parse_file(path) {
                Ok(session) => {
                    println!("Parsed session: {}", session.id);
                    println!("  Source: {:?}", session.source);
                    println!("  CWD: {}", session.cwd);
                    println!("  Messages: {}", session.messages.len());
                    for (i, msg) in session.messages.iter().enumerate() {
                        println!("  Message {}: {:?} - {} chars", i, msg.role, msg.content.len());
                        if !msg.content.is_empty() {
                            let preview: String = msg.content.chars().take(100).collect();
                            println!("    Preview: {}...", preview);
                        }
                    }
                    assert!(!session.messages.is_empty(), "Should have messages");
                }
                Err(e) => panic!("Error parsing: {}", e),
            }
        }
    }
}
