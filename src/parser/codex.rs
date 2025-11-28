use crate::session::{Message, Role, Session, SessionSource};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use super::SessionParser;

#[derive(Debug, Deserialize)]
struct CodexLine {
    #[serde(rename = "type")]
    entry_type: String,
    timestamp: Option<String>,
    payload: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SessionMeta {
    id: String,
    cwd: Option<String>,
    git: Option<GitInfo>,
}

#[derive(Debug, Deserialize)]
struct GitInfo {
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseItem {
    role: Option<String>,
    content: Option<Vec<ContentBlock>>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

pub struct CodexParser;

impl SessionParser for CodexParser {
    fn can_parse(path: &Path) -> bool {
        // Codex sessions are in ~/.codex/sessions/ and start with "rollout-"
        path.to_str()
            .map(|s| s.contains(".codex/sessions"))
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

            let entry: CodexLine = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Parse timestamp from entry
            let timestamp = entry
                .timestamp
                .as_ref()
                .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);

            match entry.entry_type.as_str() {
                "session_meta" => {
                    if let Some(payload) = &entry.payload {
                        if let Ok(meta) = serde_json::from_value::<SessionMeta>(payload.clone()) {
                            // Only set if not already set (first session_meta wins)
                            if session_id.is_none() {
                                session_id = Some(meta.id);
                            }
                            if cwd.is_none() {
                                cwd = meta.cwd;
                            }
                            if git_branch.is_none() {
                                git_branch = meta.git.and_then(|g| g.branch);
                            }
                        }
                    }
                }
                "response_item" => {
                    if let Some(payload) = &entry.payload {
                        if let Ok(item) = serde_json::from_value::<ResponseItem>(payload.clone()) {
                            let role = match item.role.as_deref() {
                                Some("user") => Role::User,
                                Some("assistant") => Role::Assistant,
                                _ => {
                                    // Try to infer role from content type
                                    if let Some(content) = &item.content {
                                        if content.iter().any(|c| c.content_type == "input_text") {
                                            Role::User
                                        } else if content
                                            .iter()
                                            .any(|c| c.content_type == "output_text")
                                        {
                                            Role::Assistant
                                        } else {
                                            continue;
                                        }
                                    } else {
                                        continue;
                                    }
                                }
                            };

                            let content = extract_codex_content(&item);
                            if !content.is_empty() {
                                messages.push(Message {
                                    role,
                                    content,
                                    timestamp,
                                });

                                // Update latest timestamp
                                if latest_timestamp.is_none()
                                    || timestamp > latest_timestamp.unwrap()
                                {
                                    latest_timestamp = Some(timestamp);
                                }
                            }
                        }
                    }
                }
                _ => {}
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
            source: SessionSource::CodexCli,
            file_path: path.to_path_buf(),
            cwd: cwd.unwrap_or_else(|| ".".to_string()),
            git_branch,
            timestamp: latest_timestamp.unwrap_or_else(Utc::now),
            messages,
        })
    }
}

/// Extract text content from a Codex response item
fn extract_codex_content(item: &ResponseItem) -> String {
    let Some(content) = &item.content else {
        return String::new();
    };

    let mut texts = Vec::new();
    for block in content {
        // Extract from input_text or output_text blocks
        if (block.content_type == "input_text" || block.content_type == "output_text")
            && block.text.is_some()
        {
            if let Some(text) = &block.text {
                texts.push(text.clone());
            }
        }
    }
    texts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_codex_content() {
        let item = ResponseItem {
            role: Some("user".to_string()),
            content: Some(vec![ContentBlock {
                content_type: "input_text".to_string(),
                text: Some("Hello Codex".to_string()),
            }]),
        };
        assert_eq!(extract_codex_content(&item), "Hello Codex");
    }
}
