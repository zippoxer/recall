use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Global flag to disable truncation (for --full mode)
pub static DISABLE_TRUNCATION: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SessionSource {
    #[serde(rename = "claude")]
    ClaudeCode,
    #[serde(rename = "codex")]
    CodexCli,
    #[serde(rename = "factory")]
    Factory,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl SessionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionSource::ClaudeCode => "claude",
            SessionSource::CodexCli => "codex",
            SessionSource::Factory => "factory",
            SessionSource::OpenCode => "opencode",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(SessionSource::ClaudeCode),
            "codex" => Some(SessionSource::CodexCli),
            "factory" => Some(SessionSource::Factory),
            "opencode" => Some(SessionSource::OpenCode),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            SessionSource::ClaudeCode => "Claude",
            SessionSource::CodexCli => "Codex",
            SessionSource::Factory => "Factory",
            SessionSource::OpenCode => "OpenCode",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            SessionSource::ClaudeCode => "●",
            SessionSource::CodexCli => "■",
            SessionSource::Factory => "◆",
            SessionSource::OpenCode => "○",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

// ============================================================================
// Tool Call Types
// ============================================================================

/// Status of a tool call execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    Success,
    Error,
    Pending, // No result found (session ended mid-tool)
}

/// Tool output with truncation metadata
#[derive(Debug, Clone, Serialize)]
pub struct ToolOutput {
    pub content: String,
    pub truncated: bool,
    pub total_bytes: usize,
}

impl ToolOutput {
    /// Maximum bytes to keep when truncating (first + last)
    const MAX_BYTES: usize = 2000;
    const KEEP_BYTES: usize = 1000; // Keep this many from start and end

    /// Create a new ToolOutput, applying truncation if necessary
    pub fn new(content: String, is_error: bool) -> Self {
        let total_bytes = content.len();

        // Check if truncation is disabled globally (--full mode)
        if DISABLE_TRUNCATION.load(Ordering::Relaxed) {
            return Self {
                content,
                truncated: false,
                total_bytes,
            };
        }

        // Never truncate errors (they're usually short and critical)
        if is_error || total_bytes <= Self::MAX_BYTES {
            return Self {
                content,
                truncated: false,
                total_bytes,
            };
        }

        // Truncate: keep first KEEP_BYTES + last KEEP_BYTES
        // Find safe char boundaries
        let start_end = content
            .char_indices()
            .take_while(|(i, _)| *i < Self::KEEP_BYTES)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(Self::KEEP_BYTES.min(total_bytes));

        let last_start = content
            .char_indices()
            .rev()
            .take_while(|(i, _)| total_bytes - *i < Self::KEEP_BYTES)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(total_bytes.saturating_sub(Self::KEEP_BYTES));

        let truncated_content = format!(
            "{}\n[...truncated {:.1}kb...]\n{}",
            &content[..start_end],
            (total_bytes as f64 - Self::MAX_BYTES as f64) / 1024.0,
            &content[last_start..]
        );

        Self {
            content: truncated_content,
            truncated: true,
            total_bytes,
        }
    }

    /// Create without truncation (for --full mode during output)
    pub fn new_untruncated(content: String) -> Self {
        let total_bytes = content.len();
        Self {
            content,
            truncated: false,
            total_bytes,
        }
    }
}

/// A single tool call with its result
#[derive(Debug, Clone, Serialize)]
pub struct ToolCall {
    /// Display ID like "2.1" (message 2, tool 1) - assigned during output formatting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Original Claude tool_use_id for matching results (internal use only)
    #[serde(skip_serializing)]
    pub tool_use_id: String,
    /// Tool name: "Bash", "Read", "Edit", etc.
    pub name: String,
    /// Tool-specific input (command, file_path, etc.)
    pub input: serde_json::Value,
    /// Execution status
    pub status: ToolStatus,
    /// Execution duration in milliseconds (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Tool output (if result was received)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<ToolOutput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub source: SessionSource,
    pub file_path: PathBuf,
    pub cwd: String,
    pub git_branch: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub messages: Vec<Message>,
}

impl Session {
    /// Get the project name from cwd (last path component)
    pub fn project_name(&self) -> &str {
        std::path::Path::new(&self.cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&self.cwd)
    }

    /// Get the resume command for this session
    /// Checks RECALL_CLAUDE_CMD / RECALL_CODEX_CMD / RECALL_FACTORY_CMD env vars first, falls back to defaults
    /// Env var format: "program arg1 arg2 {id}" where {id} is replaced with session ID
    pub fn resume_command(&self) -> (String, Vec<String>) {
        let env_var = match self.source {
            SessionSource::ClaudeCode => "RECALL_CLAUDE_CMD",
            SessionSource::CodexCli => "RECALL_CODEX_CMD",
            SessionSource::Factory => "RECALL_FACTORY_CMD",
            SessionSource::OpenCode => "RECALL_OPENCODE_CMD",
        };

        if let Ok(cmd) = std::env::var(env_var) {
            let cmd = cmd.replace("{id}", &self.id);
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if !parts.is_empty() {
                return (
                    parts[0].to_string(),
                    parts[1..].iter().map(|s| s.to_string()).collect(),
                );
            }
        }

        // Default commands
        match self.source {
            SessionSource::ClaudeCode => (
                "claude".to_string(),
                vec!["--resume".to_string(), self.id.clone()],
            ),
            SessionSource::CodexCli => (
                "codex".to_string(),
                vec!["resume".to_string(), self.id.clone()],
            ),
            SessionSource::Factory => (
                "droid".to_string(),
                vec!["--resume".to_string(), self.id.clone()],
            ),
            SessionSource::OpenCode => (
                "opencode".to_string(),
                vec!["run".to_string(), "-s".to_string(), self.id.clone()],
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub session: Session,
    pub score: f32,
    /// Index of the most recent message containing a match
    pub matched_message_index: usize,
    /// Snippet from the matched message (newlines replaced with spaces)
    pub snippet: String,
    /// Byte ranges of matches within the snippet for highlighting
    pub match_spans: Vec<(usize, usize)>,
    /// Original fragment from Tantivy (for finding match in wrapped text)
    pub match_fragment: String,
}

// ============================================================================
// CLI Output Types (JSON serialization for non-interactive mode)
// ============================================================================

/// Output format for `recall search`
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    pub query: String,
    pub results: Vec<SearchResultOutput>,
}

/// Single search result in JSON output
#[derive(Debug, Serialize)]
pub struct SearchResultOutput {
    pub session_id: String,
    pub source: SessionSource,
    pub cwd: String,
    pub timestamp: DateTime<Utc>,
    pub relevant_messages: Vec<Message>,
    pub resume_command: String,
}

/// Output format for `recall list`
#[derive(Debug, Serialize)]
pub struct ListOutput {
    pub sessions: Vec<SessionSummary>,
}

/// Session summary for list output (no messages)
#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub source: SessionSource,
    pub cwd: String,
    pub timestamp: DateTime<Utc>,
    pub resume_command: String,
}

/// Output format for `recall read`
#[derive(Debug, Serialize)]
pub struct ReadOutput {
    pub session_id: String,
    pub source: SessionSource,
    pub cwd: String,
    pub timestamp: DateTime<Utc>,
    pub messages: Vec<Message>,
    pub resume_command: String,
}

impl Session {
    /// Convert to ReadOutput for JSON serialization
    pub fn to_read_output(&self) -> ReadOutput {
        let (cmd, args) = self.resume_command();
        let resume_str = std::iter::once(cmd)
            .chain(args)
            .collect::<Vec<_>>()
            .join(" ");

        ReadOutput {
            session_id: self.id.clone(),
            source: self.source,
            cwd: self.cwd.clone(),
            timestamp: self.timestamp,
            messages: self.messages.clone(),
            resume_command: resume_str,
        }
    }

    /// Convert to SessionSummary for list output
    pub fn to_summary(&self) -> SessionSummary {
        let (cmd, args) = self.resume_command();
        let resume_str = std::iter::once(cmd)
            .chain(args)
            .collect::<Vec<_>>()
            .join(" ");

        SessionSummary {
            session_id: self.id.clone(),
            source: self.source,
            cwd: self.cwd.clone(),
            timestamp: self.timestamp,
            resume_command: resume_str,
        }
    }
}
