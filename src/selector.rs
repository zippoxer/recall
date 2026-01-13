//! Selector parsing for recall read command.
//!
//! Supports the following formats:
//! - `abc123` - Session only
//! - `abc123:5` - Message 5
//! - `abc123:5.2` - Message 5, Tool 2
//! - `abc123:2-5` - Messages 2 through 5
//! - `abc123:-3` - Last 3 messages
//! - `abc123:errors` - Only messages with failed tool calls

use std::fmt;

/// Parsed selector for session/message/tool addressing
#[derive(Debug, Clone, PartialEq)]
pub enum Selector {
    /// Just the session ID
    Session { id: String },
    /// Session with message selector
    Message {
        session_id: String,
        message: MessageSelector,
    },
    /// Session with specific tool call
    Tool {
        session_id: String,
        message_idx: usize,
        tool_idx: usize,
    },
}

/// Message selection within a session
#[derive(Debug, Clone, PartialEq)]
pub enum MessageSelector {
    /// Single message by index (1-based)
    Single(usize),
    /// Range of messages (1-based, inclusive)
    Range(usize, usize),
    /// Last N messages
    Last(usize),
    /// Only messages with error tool calls
    Errors,
}

/// Error type for selector parsing
#[derive(Debug, Clone, PartialEq)]
pub enum SelectorError {
    EmptyInput,
    InvalidMessageIndex(String),
    InvalidToolIndex(String),
    InvalidRange(String),
    InvalidLastCount(String),
}

impl fmt::Display for SelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectorError::EmptyInput => write!(f, "empty selector"),
            SelectorError::InvalidMessageIndex(s) => write!(f, "invalid message index: {}", s),
            SelectorError::InvalidToolIndex(s) => write!(f, "invalid tool index: {}", s),
            SelectorError::InvalidRange(s) => write!(f, "invalid range: {}", s),
            SelectorError::InvalidLastCount(s) => write!(f, "invalid last count: {}", s),
        }
    }
}

impl std::error::Error for SelectorError {}

impl Selector {
    /// Get the session ID from any selector type
    pub fn session_id(&self) -> &str {
        match self {
            Selector::Session { id } => id,
            Selector::Message { session_id, .. } => session_id,
            Selector::Tool { session_id, .. } => session_id,
        }
    }
}

/// Parse a selector string into a Selector
///
/// # Examples
///
/// ```
/// use recall::selector::{parse_selector, Selector, MessageSelector};
///
/// // Session only
/// let s = parse_selector("abc123").unwrap();
/// assert!(matches!(s, Selector::Session { id } if id == "abc123"));
///
/// // Message 5
/// let s = parse_selector("abc123:5").unwrap();
/// assert!(matches!(s, Selector::Message { session_id, message: MessageSelector::Single(5) } if session_id == "abc123"));
///
/// // Message 5, Tool 2
/// let s = parse_selector("abc123:5.2").unwrap();
/// assert!(matches!(s, Selector::Tool { session_id, message_idx: 5, tool_idx: 2 } if session_id == "abc123"));
/// ```
pub fn parse_selector(input: &str) -> Result<Selector, SelectorError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SelectorError::EmptyInput);
    }

    // Split on first colon to separate session ID from the rest
    if let Some(colon_pos) = input.find(':') {
        let session_id = input[..colon_pos].to_string();
        let rest = &input[colon_pos + 1..];

        // Check for special selectors
        if rest == "errors" {
            return Ok(Selector::Message {
                session_id,
                message: MessageSelector::Errors,
            });
        }

        // Check for tool selector (contains a dot)
        if let Some(dot_pos) = rest.find('.') {
            let msg_part = &rest[..dot_pos];
            let tool_part = &rest[dot_pos + 1..];

            let message_idx = msg_part
                .parse::<usize>()
                .map_err(|_| SelectorError::InvalidMessageIndex(msg_part.to_string()))?;
            let tool_idx = tool_part
                .parse::<usize>()
                .map_err(|_| SelectorError::InvalidToolIndex(tool_part.to_string()))?;

            return Ok(Selector::Tool {
                session_id,
                message_idx,
                tool_idx,
            });
        }

        // Check for range selector (contains a dash)
        if let Some(dash_pos) = rest.find('-') {
            // Could be :-3 (last 3) or :2-5 (range)
            if dash_pos == 0 {
                // Last N: :-3
                let count = rest[1..]
                    .parse::<usize>()
                    .map_err(|_| SelectorError::InvalidLastCount(rest.to_string()))?;
                return Ok(Selector::Message {
                    session_id,
                    message: MessageSelector::Last(count),
                });
            } else {
                // Range: :2-5
                let start_part = &rest[..dash_pos];
                let end_part = &rest[dash_pos + 1..];

                let start = start_part
                    .parse::<usize>()
                    .map_err(|_| SelectorError::InvalidRange(rest.to_string()))?;
                let end = end_part
                    .parse::<usize>()
                    .map_err(|_| SelectorError::InvalidRange(rest.to_string()))?;

                return Ok(Selector::Message {
                    session_id,
                    message: MessageSelector::Range(start, end),
                });
            }
        }

        // Single message index
        let message_idx = rest
            .parse::<usize>()
            .map_err(|_| SelectorError::InvalidMessageIndex(rest.to_string()))?;

        Ok(Selector::Message {
            session_id,
            message: MessageSelector::Single(message_idx),
        })
    } else {
        // No colon, just session ID
        Ok(Selector::Session {
            id: input.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_session_only() {
        let s = parse_selector("abc123").unwrap();
        assert_eq!(
            s,
            Selector::Session {
                id: "abc123".to_string()
            }
        );
    }

    #[test]
    fn test_parse_session_with_uuid() {
        let s = parse_selector("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(
            s,
            Selector::Session {
                id: "550e8400-e29b-41d4-a716-446655440000".to_string()
            }
        );
    }

    #[test]
    fn test_parse_single_message() {
        let s = parse_selector("abc123:5").unwrap();
        assert_eq!(
            s,
            Selector::Message {
                session_id: "abc123".to_string(),
                message: MessageSelector::Single(5),
            }
        );
    }

    #[test]
    fn test_parse_tool() {
        let s = parse_selector("abc123:5.2").unwrap();
        assert_eq!(
            s,
            Selector::Tool {
                session_id: "abc123".to_string(),
                message_idx: 5,
                tool_idx: 2,
            }
        );
    }

    #[test]
    fn test_parse_range() {
        let s = parse_selector("abc123:2-5").unwrap();
        assert_eq!(
            s,
            Selector::Message {
                session_id: "abc123".to_string(),
                message: MessageSelector::Range(2, 5),
            }
        );
    }

    #[test]
    fn test_parse_last() {
        let s = parse_selector("abc123:-3").unwrap();
        assert_eq!(
            s,
            Selector::Message {
                session_id: "abc123".to_string(),
                message: MessageSelector::Last(3),
            }
        );
    }

    #[test]
    fn test_parse_errors() {
        let s = parse_selector("abc123:errors").unwrap();
        assert_eq!(
            s,
            Selector::Message {
                session_id: "abc123".to_string(),
                message: MessageSelector::Errors,
            }
        );
    }

    #[test]
    fn test_parse_empty() {
        let err = parse_selector("").unwrap_err();
        assert_eq!(err, SelectorError::EmptyInput);
    }

    #[test]
    fn test_parse_invalid_message_index() {
        let err = parse_selector("abc123:foo").unwrap_err();
        assert!(matches!(err, SelectorError::InvalidMessageIndex(_)));
    }

    #[test]
    fn test_parse_invalid_tool_index() {
        let err = parse_selector("abc123:5.foo").unwrap_err();
        assert!(matches!(err, SelectorError::InvalidToolIndex(_)));
    }

    #[test]
    fn test_session_id_extraction() {
        assert_eq!(
            parse_selector("abc123").unwrap().session_id(),
            "abc123"
        );
        assert_eq!(
            parse_selector("abc123:5").unwrap().session_id(),
            "abc123"
        );
        assert_eq!(
            parse_selector("abc123:5.2").unwrap().session_id(),
            "abc123"
        );
    }

    #[test]
    fn test_whitespace_trimmed() {
        let s = parse_selector("  abc123:5  ").unwrap();
        assert_eq!(
            s,
            Selector::Message {
                session_id: "abc123".to_string(),
                message: MessageSelector::Single(5),
            }
        );
    }
}
