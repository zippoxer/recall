use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::PathBuf;

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

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub timestamp: DateTime<Utc>,
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

    /// Get the directory to cd into before resuming the session.
    /// For Claude Code: decodes the project folder from file_path since sessions
    /// are stored in project-specific folders, not the cwd recorded in messages.
    /// For other sources: uses the cwd field.
    pub fn resume_cwd(&self) -> String {
        match self.source {
            SessionSource::ClaudeCode => {
                // Extract project folder from file_path: ~/.claude/projects/<project>/session.jsonl
                // The project folder encodes the original cwd:
                // "-Users-bob--config-nvim" -> "/Users/bob/.config/nvim"
                if let Some(project_dir) = self.file_path.parent() {
                    if let Some(project_name) = project_dir.file_name().and_then(|s| s.to_str()) {
                        // Decode: "--" -> "/." (hidden dir), "-" -> "/"
                        let decoded = project_name
                            .replace("--", "\x00")  // Temporarily mark hidden dirs
                            .replace('-', "/")
                            .replace('\x00', "/.");
                        if std::path::Path::new(&decoded).exists() {
                            return decoded;
                        }
                    }
                }
                // Fall back to cwd if decoding fails
                self.cwd.clone()
            }
            _ => self.cwd.clone(),
        }
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
                vec!["--session".to_string(), self.id.clone()],
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
