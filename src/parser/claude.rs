use crate::session::{Message, Role, Session, SessionSource, ToolCall, ToolOutput, ToolStatus};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
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
    /// Compaction summary flag (v2.0.56+)
    #[serde(rename = "isCompactSummary")]
    is_compact_summary: Option<bool>,
    /// Transcript-only flag (v2.0.55 compaction, also set in v2.0.56+)
    #[serde(rename = "isVisibleInTranscriptOnly")]
    is_visible_in_transcript_only: Option<bool>,
    /// Meta message flag (slash command prompt expansions)
    #[serde(rename = "isMeta")]
    is_meta: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    role: String,
    content: serde_json::Value,
}

pub struct ClaudeParser;

/// Represents a pending tool result to be matched to its tool call
#[derive(Debug)]
struct ToolResult {
    tool_use_id: String,
    content: String,
    is_error: bool,
    duration_ms: Option<u64>,
}

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
        // Collect tool results for matching after all messages are parsed
        let mut tool_results: HashMap<String, ToolResult> = HashMap::new();

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

            // Skip synthetic messages (not actual user input):
            // - Compaction summaries (v2.0.56+ isCompactSummary, v2.0.55 isVisibleInTranscriptOnly)
            // - Slash command prompt expansions (isMeta)
            if entry.is_compact_summary == Some(true)
                || entry.is_visible_in_transcript_only == Some(true)
                || entry.is_meta == Some(true)
            {
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

            // Extract message content and tool calls
            if let Some(msg) = &entry.message {
                let role = match msg.role.as_str() {
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    _ => continue,
                };

                let (content, tool_calls) = extract_content_and_tools(&msg.content);

                // For user messages, extract tool results
                if role == Role::User {
                    for result in extract_tool_results(&msg.content) {
                        tool_results.insert(result.tool_use_id.clone(), result);
                    }
                }

                if content.is_empty() && tool_calls.is_empty() {
                    continue;
                }

                // Skip slash command expansions (internal Claude Code messages)
                let trimmed = content.trim_start();
                if trimmed.starts_with("<command-message>")
                    || trimmed.starts_with("<command-name>")
                {
                    continue;
                }

                messages.push(Message {
                    role,
                    content,
                    timestamp,
                    tool_calls,
                });
            }
        }

        // Match tool results to tool calls
        for message in &mut messages {
            for tool_call in &mut message.tool_calls {
                if let Some(result) = tool_results.remove(&tool_call.tool_use_id) {
                    tool_call.status = if result.is_error {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Success
                    };
                    tool_call.duration_ms = result.duration_ms;
                    tool_call.output = Some(ToolOutput::new(result.content, result.is_error));
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

/// Extract text content and tool calls from Claude's message content field.
/// - User messages: content is a plain string (no tool calls)
/// - Assistant messages: content is an array of {type, text} and {type: tool_use} objects
fn extract_content_and_tools(content: &serde_json::Value) -> (String, Vec<ToolCall>) {
    match content {
        // Direct string (user messages)
        serde_json::Value::String(s) => (s.clone(), Vec::new()),

        // Array of content blocks (assistant messages)
        serde_json::Value::Array(arr) => {
            let mut texts = Vec::new();
            let mut tool_calls = Vec::new();

            for item in arr {
                if let Some(obj) = item.as_object() {
                    let block_type = obj.get("type").and_then(|v| v.as_str());

                    match block_type {
                        // Text blocks
                        Some("text") => {
                            if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                                texts.push(text.to_string());
                            }
                        }
                        // Tool use blocks
                        Some("tool_use") => {
                            let tool_use_id = obj
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = obj
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let input = obj
                                .get("input")
                                .cloned()
                                .unwrap_or(serde_json::Value::Null);

                            tool_calls.push(ToolCall {
                                id: None, // Assigned during output formatting
                                tool_use_id,
                                name,
                                input,
                                status: ToolStatus::Pending, // Will be updated when result is matched
                                duration_ms: None,
                                output: None,
                            });
                        }
                        // Skip thinking, etc.
                        _ => {}
                    }
                }
            }
            (texts.join("\n"), tool_calls)
        }

        _ => (String::new(), Vec::new()),
    }
}

/// Extract tool results from user message content.
/// Tool results appear as {type: "tool_result", tool_use_id: "...", content: "..."} blocks.
fn extract_tool_results(content: &serde_json::Value) -> Vec<ToolResult> {
    let mut results = Vec::new();

    if let serde_json::Value::Array(arr) = content {
        for item in arr {
            if let Some(obj) = item.as_object() {
                if obj.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                    let tool_use_id = obj
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    if tool_use_id.is_empty() {
                        continue;
                    }

                    // Content can be a string or array of content blocks
                    let content_str = match obj.get("content") {
                        Some(serde_json::Value::String(s)) => s.clone(),
                        Some(serde_json::Value::Array(arr)) => {
                            // Extract text from content blocks
                            arr.iter()
                                .filter_map(|block| {
                                    block.as_object().and_then(|o| {
                                        if o.get("type").and_then(|v| v.as_str()) == Some("text") {
                                            o.get("text").and_then(|v| v.as_str()).map(String::from)
                                        } else {
                                            None
                                        }
                                    })
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                        _ => String::new(),
                    };

                    // Check for error status
                    let is_error = obj.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);

                    // Extract duration if available (Claude Code adds this as metadata)
                    let duration_ms = obj.get("durationMs").and_then(|v| v.as_u64());

                    results.push(ToolResult {
                        tool_use_id,
                        content: content_str,
                        is_error,
                        duration_ms,
                    });
                }
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_content_string() {
        let content = serde_json::json!("Hello, world!");
        let (text, tools) = extract_content_and_tools(&content);
        assert_eq!(text, "Hello, world!");
        assert!(tools.is_empty());
    }

    #[test]
    fn test_extract_content_array_with_tools() {
        let content = serde_json::json!([
            {"type": "text", "text": "Hello"},
            {"type": "tool_use", "id": "tool_123", "name": "Read", "input": {"file_path": "/test"}},
            {"type": "text", "text": "World"}
        ]);
        let (text, tools) = extract_content_and_tools(&content);
        assert_eq!(text, "Hello\nWorld");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "Read");
        assert_eq!(tools[0].tool_use_id, "tool_123");
        assert_eq!(tools[0].status, ToolStatus::Pending);
    }

    #[test]
    fn test_extract_tool_results() {
        let content = serde_json::json!([
            {
                "type": "tool_result",
                "tool_use_id": "tool_123",
                "content": "File contents here",
                "is_error": false,
                "durationMs": 42
            }
        ]);
        let results = extract_tool_results(&content);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_use_id, "tool_123");
        assert_eq!(results[0].content, "File contents here");
        assert!(!results[0].is_error);
        assert_eq!(results[0].duration_ms, Some(42));
    }

    #[test]
    fn test_extract_tool_results_error() {
        let content = serde_json::json!([
            {
                "type": "tool_result",
                "tool_use_id": "tool_456",
                "content": "No such file",
                "is_error": true
            }
        ]);
        let results = extract_tool_results(&content);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
    }

    #[test]
    fn test_extract_tool_results_nested_content() {
        let content = serde_json::json!([
            {
                "type": "tool_result",
                "tool_use_id": "tool_789",
                "content": [
                    {"type": "text", "text": "Line 1"},
                    {"type": "text", "text": "Line 2"}
                ]
            }
        ]);
        let results = extract_tool_results(&content);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "Line 1\nLine 2");
    }
}
