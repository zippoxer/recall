use chrono::{DateTime, Utc};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSource {
    ClaudeCode,
    CodexCli,
}

impl SessionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionSource::ClaudeCode => "claude",
            SessionSource::CodexCli => "codex",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(SessionSource::ClaudeCode),
            "codex" => Some(SessionSource::CodexCli),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            SessionSource::ClaudeCode => "Claude",
            SessionSource::CodexCli => "Codex",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            SessionSource::ClaudeCode => "●",
            SessionSource::CodexCli => "■",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
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

    /// Get the resume command for this session
    /// Checks RECALL_CLAUDE_CMD / RECALL_CODEX_CMD env vars first, falls back to defaults
    /// Env var format: "program arg1 arg2 {id}" where {id} is replaced with session ID
    pub fn resume_command(&self) -> (String, Vec<String>) {
        let env_var = match self.source {
            SessionSource::ClaudeCode => "RECALL_CLAUDE_CMD",
            SessionSource::CodexCli => "RECALL_CODEX_CMD",
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
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub session: Session,
    pub score: f32,
    /// Index of the most recent message containing a match
    pub matched_message_index: usize,
    /// Snippet from the matched message
    pub snippet: String,
    /// Byte ranges of matches within the snippet for highlighting
    pub match_spans: Vec<(usize, usize)>,
}
