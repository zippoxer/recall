//! CLI subcommands for non-interactive mode (JSON output for agents)

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use recall::{
    index::{ensure_index_fresh, SessionIndex},
    parser,
    session::{ListOutput, Message, SearchOutput, SearchResultOutput, SessionSource},
    DISABLE_TRUNCATION,
};
use std::sync::atomic::Ordering;

const DEFAULT_MESSAGES_PER_SESSION: usize = 5;

/// Run the search subcommand
#[allow(clippy::too_many_arguments)]
pub fn run_search(
    query: &str,
    source: Option<SessionSource>,
    session_id: Option<String>,
    limit: usize,
    context: usize,
    since: Option<String>,
    until: Option<String>,
    cwd: Option<String>,
) -> Result<()> {
    let index = SessionIndex::open_default()?;
    ensure_index_fresh(&index)?;

    // Parse time filters
    let since_dt = since.as_ref().map(|s| parse_time(s)).transpose()?;
    let until_dt = until.as_ref().map(|s| parse_time(s)).transpose()?;

    // If searching within a specific session, handle separately
    if let Some(sid) = session_id {
        return search_in_session(&index, query, &sid, context);
    }

    let results = index.search(query, limit * 2)?; // Get more to filter

    // Pre-compute query terms once (not per-session)
    let query_lower = query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

    // Convert to output format
    let output = SearchOutput {
        query: query.to_string(),
        results: results
            .into_iter()
            // Filter by source
            .filter(|r| source.is_none_or(|s| r.session.source == s))
            // Filter by time
            .filter(|r| since_dt.is_none_or(|t| r.session.timestamp >= t))
            .filter(|r| until_dt.is_none_or(|t| r.session.timestamp <= t))
            // Filter by working directory
            .filter(|r| cwd.as_ref().is_none_or(|c| r.session.cwd == *c))
            .take(limit)
            .map(|r| {
                // Load full session to get messages
                let session = parser::parse_session_file(&r.session.file_path)
                    .unwrap_or(r.session.clone());

                // Filter and score messages in one pass (avoids repeated to_lowercase in sort)
                let mut scored_messages: Vec<(usize, usize, &Message)> = session
                    .messages
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, m)| {
                        let content_lower = m.content.to_lowercase();
                        let score: usize = query_terms
                            .iter()
                            .map(|t| content_lower.matches(t).count())
                            .sum();
                        if score > 0 {
                            Some((idx, score, m))
                        } else {
                            None
                        }
                    })
                    .collect();

                // Sort by pre-computed score (higher first), then recency (higher index first)
                scored_messages.sort_by(|(idx_a, score_a, _), (idx_b, score_b, _)| {
                    score_b.cmp(score_a).then_with(|| idx_b.cmp(idx_a))
                });

                // Get top N messages, with context if requested
                let relevant_messages = if context > 0 {
                    // Convert to format expected by collect_with_context
                    let for_context: Vec<(usize, &Message)> = scored_messages
                        .iter()
                        .map(|(idx, _, m)| (*idx, *m))
                        .collect();
                    collect_with_context(&session.messages, &for_context, context)
                } else {
                    scored_messages
                        .into_iter()
                        .take(DEFAULT_MESSAGES_PER_SESSION)
                        .map(|(_, _, m)| m.clone())
                        .collect()
                };

                let (cmd, args) = r.session.resume_command();
                let resume_command = std::iter::once(cmd)
                    .chain(args)
                    .collect::<Vec<_>>()
                    .join(" ");

                SearchResultOutput {
                    session_id: r.session.id,
                    source: r.session.source,
                    cwd: r.session.cwd,
                    timestamp: r.session.timestamp,
                    relevant_messages,
                    resume_command,
                }
            })
            .collect(),
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Search within a specific session (returns all matches)
fn search_in_session(
    index: &SessionIndex,
    query: &str,
    session_id: &str,
    context: usize,
) -> Result<()> {
    let file_path = index
        .get_by_id(session_id)?
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

    let session = parser::parse_session_file(&file_path)?;

    let query_lower = query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

    // Filter and score messages in one pass (avoids repeated to_lowercase in sort)
    let mut scored_messages: Vec<(usize, usize, &Message)> = session
        .messages
        .iter()
        .enumerate()
        .filter_map(|(idx, m)| {
            let content_lower = m.content.to_lowercase();
            let score: usize = query_terms
                .iter()
                .map(|t| content_lower.matches(t).count())
                .sum();
            if score > 0 {
                Some((idx, score, m))
            } else {
                None
            }
        })
        .collect();

    // Sort by pre-computed score (higher first), then recency (higher index first)
    scored_messages.sort_by(|(idx_a, score_a, _), (idx_b, score_b, _)| {
        score_b.cmp(score_a).then_with(|| idx_b.cmp(idx_a))
    });

    // Return all matches (no limit for single session search)
    let relevant_messages = if context > 0 {
        let for_context: Vec<(usize, &Message)> = scored_messages
            .iter()
            .map(|(idx, _, m)| (*idx, *m))
            .collect();
        collect_with_context(&session.messages, &for_context, context)
    } else {
        scored_messages
            .into_iter()
            .map(|(_, _, m)| m.clone())
            .collect()
    };

    let (cmd, args) = session.resume_command();
    let resume_command = std::iter::once(cmd)
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ");

    let output = SearchOutput {
        query: query.to_string(),
        results: vec![SearchResultOutput {
            session_id: session.id,
            source: session.source,
            cwd: session.cwd,
            timestamp: session.timestamp,
            relevant_messages,
            resume_command,
        }],
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Collect messages with context around matches, deduplicating overlaps
fn collect_with_context(
    all_messages: &[Message],
    scored: &[(usize, &Message)],
    context: usize,
) -> Vec<Message> {
    let mut indices: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();

    for (idx, _) in scored.iter().take(DEFAULT_MESSAGES_PER_SESSION) {
        let start = idx.saturating_sub(context);
        let end = (*idx + context + 1).min(all_messages.len());
        for i in start..end {
            indices.insert(i);
        }
    }

    indices
        .into_iter()
        .map(|i| all_messages[i].clone())
        .collect()
}

/// Run the list subcommand
pub fn run_list(
    limit: usize,
    source: Option<SessionSource>,
    since: Option<String>,
    until: Option<String>,
    cwd: Option<String>,
) -> Result<()> {
    let index = SessionIndex::open_default()?;
    ensure_index_fresh(&index)?;

    // Parse time filters
    let since_dt = since.as_ref().map(|s| parse_time(s)).transpose()?;
    let until_dt = until.as_ref().map(|s| parse_time(s)).transpose()?;

    let results = index.recent(limit * 2)?; // Get more to filter

    let output = ListOutput {
        sessions: results
            .iter()
            // Filter by source
            .filter(|r| source.is_none_or(|s| r.session.source == s))
            // Filter by time
            .filter(|r| since_dt.is_none_or(|t| r.session.timestamp >= t))
            .filter(|r| until_dt.is_none_or(|t| r.session.timestamp <= t))
            // Filter by working directory
            .filter(|r| cwd.as_ref().is_none_or(|c| r.session.cwd == *c))
            .take(limit)
            .map(|r| r.session.to_summary())
            .collect(),
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Run the read subcommand with selector support
#[allow(clippy::too_many_arguments)]
pub fn run_read(
    selector_str: &str,
    after: Option<usize>,
    before: Option<usize>,
    context: Option<usize>,
    full: bool,
    pretty: bool,
) -> Result<()> {
    use recall::selector::{parse_selector, MessageSelector, Selector};
    use recall::session::{ReadOutput, ToolStatus};

    let index = SessionIndex::open_default()?;
    ensure_index_fresh(&index)?;

    // Parse the selector
    let selector = parse_selector(selector_str)
        .map_err(|e| anyhow::anyhow!("Invalid selector: {}", e))?;

    // Find the session by ID
    let session_id = selector.session_id();
    let file_path = index
        .get_by_id(session_id)?
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

    // Set truncation flag before parsing
    if full {
        DISABLE_TRUNCATION.store(true, Ordering::SeqCst);
    }

    // Parse full session
    let session = parser::parse_session_file(&file_path)?;

    // Reset truncation flag after parsing
    if full {
        DISABLE_TRUNCATION.store(false, Ordering::SeqCst);
    }

    // Calculate context bounds
    let before_ctx = before.or(context).unwrap_or(0);
    let after_ctx = after.or(context).unwrap_or(0);

    // Apply selector filtering and get messages with their IDs
    let (selected_messages, focus_range): (Vec<(usize, Message)>, std::ops::Range<usize>) =
        match &selector {
            Selector::Session { .. } => {
                // Return all messages
                let msgs: Vec<_> = session
                    .messages
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (i + 1, m.clone()))
                    .collect();
                let range = if msgs.is_empty() { 0..0 } else { 0..msgs.len() };
                (msgs, range)
            }
            Selector::Message { message, .. } => {
                let total = session.messages.len();

                match message {
                    MessageSelector::Errors => {
                        // Filter to messages with error tool calls
                        let error_indices: Vec<usize> = session
                            .messages
                            .iter()
                            .enumerate()
                            .filter(|(_, m)| {
                                m.tool_calls.iter().any(|t| t.status == ToolStatus::Error)
                            })
                            .map(|(i, _)| i)
                            .collect();

                        if error_indices.is_empty() {
                            // Return empty
                            (Vec::new(), 0..0)
                        } else {
                            // Collect with context
                            let mut indices: std::collections::BTreeSet<usize> =
                                std::collections::BTreeSet::new();
                            for idx in &error_indices {
                                let start = idx.saturating_sub(before_ctx);
                                let end = (*idx + 1 + after_ctx).min(total);
                                for i in start..end {
                                    indices.insert(i);
                                }
                            }

                            let msgs: Vec<_> = indices
                                .into_iter()
                                .map(|i| (i + 1, session.messages[i].clone()))
                                .collect();
                            let range = 0..msgs.len();
                            (msgs, range)
                        }
                    }
                    _ => {
                        let (start, end, focus_start, focus_end) = match message {
                            MessageSelector::Single(idx) => {
                                let idx = *idx;
                                if idx == 0 || idx > total {
                                    return Err(anyhow::anyhow!(
                                        "Message {} not found (session has {} messages)",
                                        idx,
                                        total
                                    ));
                                }
                                let start = idx.saturating_sub(1 + before_ctx);
                                let end = (idx + after_ctx).min(total);
                                (start, end, idx - 1 - start, idx - start)
                            }
                            MessageSelector::Range(s, e) => {
                                let s = *s;
                                let e = *e;
                                if s == 0 || e == 0 || s > total || e > total || s > e {
                                    return Err(anyhow::anyhow!(
                                        "Invalid range {}-{} (session has {} messages)",
                                        s,
                                        e,
                                        total
                                    ));
                                }
                                let start = s.saturating_sub(1 + before_ctx);
                                let end = (e + after_ctx).min(total);
                                (start, end, s - 1 - start, e - start)
                            }
                            MessageSelector::Last(n) => {
                                let n = *n;
                                let start = total.saturating_sub(n + before_ctx);
                                let end = total;
                                let focus_start = if total >= n { total - n - start } else { 0 };
                                (start, end, focus_start, end - start)
                            }
                            MessageSelector::Errors => unreachable!(),
                        };

                        let msgs: Vec<_> = (start..end)
                            .map(|i| (i + 1, session.messages[i].clone()))
                            .collect();
                        (msgs, focus_start..focus_end)
                    }
                }
            }
            Selector::Tool {
                message_idx,
                tool_idx,
                ..
            } => {
                let msg_idx = *message_idx;
                let tool_idx = *tool_idx;
                let total = session.messages.len();

                if msg_idx == 0 || msg_idx > total {
                    return Err(anyhow::anyhow!(
                        "Message {} not found (session has {} messages)",
                        msg_idx,
                        total
                    ));
                }

                let msg = &session.messages[msg_idx - 1];
                let tool_count = msg.tool_calls.len();

                if tool_idx == 0 || tool_idx > tool_count {
                    return Err(anyhow::anyhow!(
                        "Tool {} not found in message {} ({} tool calls)",
                        tool_idx,
                        msg_idx,
                        tool_count
                    ));
                }

                let start = msg_idx.saturating_sub(1 + before_ctx);
                let end = (msg_idx + after_ctx).min(total);

                let msgs: Vec<_> = (start..end)
                    .map(|i| (i + 1, session.messages[i].clone()))
                    .collect();
                (msgs, (msg_idx - 1 - start)..(msg_idx - start))
            }
        };

    // Assign display IDs to tool calls (e.g., "2.1", "2.2") and keep message numbers
    let messages_with_nums: Vec<(usize, Message)> = selected_messages
        .into_iter()
        .map(|(msg_num, mut msg)| {
            for (tool_idx, tool) in msg.tool_calls.iter_mut().enumerate() {
                tool.id = Some(format!("{}.{}", msg_num, tool_idx + 1));
            }
            (msg_num, msg)
        })
        .collect();

    // Output
    if pretty {
        print_pretty(&session, &messages_with_nums, &focus_range);
    } else {
        let (cmd, args) = session.resume_command();
        let resume_str = std::iter::once(cmd)
            .chain(args)
            .collect::<Vec<_>>()
            .join(" ");

        let messages: Vec<Message> = messages_with_nums.into_iter().map(|(_, m)| m).collect();
        let output = ReadOutput {
            session_id: session.id.clone(),
            source: session.source,
            cwd: session.cwd.clone(),
            timestamp: session.timestamp,
            messages,
            resume_command: resume_str,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

/// Print session in human-readable pretty format
fn print_pretty(
    session: &recall::Session,
    messages: &[(usize, Message)],
    _focus_range: &std::ops::Range<usize>, // Could be used for highlighting in future
) {
    use recall::session::ToolStatus;

    // Header
    println!(
        "─── {} ({}, {}, {}) ───",
        session.id,
        session.timestamp.format("%Y-%m-%d"),
        session.source.as_str(),
        session.cwd
    );
    println!();

    // Messages
    for (msg_num, msg) in messages.iter() {
        let role = msg.role.as_str();

        // Print message header
        println!("{:>5} │ {}: {}", msg_num, role, first_line(&msg.content));

        // Print rest of content if multi-line
        for line in msg.content.lines().skip(1).take(5) {
            println!("      │ {}", line);
        }
        if msg.content.lines().count() > 6 {
            println!("      │ ...");
        }

        // Print tool calls
        for tool in &msg.tool_calls {
            let id = tool.id.as_deref().unwrap_or("?");
            let status_icon = match tool.status {
                ToolStatus::Success => "✓",
                ToolStatus::Error => "✗",
                ToolStatus::Pending => "…",
            };
            let duration = tool
                .duration_ms
                .map(|d| format!(" ({}ms)", d))
                .unwrap_or_default();

            println!();
            println!("{:>5} │   {} {}{}", id, status_icon, tool.name, duration);

            // Print tool input summary
            if let Some(cmd) = tool.input.get("command").and_then(|v| v.as_str()) {
                let cmd_short = if cmd.len() > 60 {
                    format!("{}...", &cmd[..57])
                } else {
                    cmd.to_string()
                };
                println!("      │   $ {}", cmd_short);
            } else if let Some(path) = tool.input.get("file_path").and_then(|v| v.as_str()) {
                println!("      │   {}", path);
            }

            // Print output summary (if exists)
            if let Some(output) = &tool.output {
                let first = first_line(&output.content);
                println!("      │   → {}", first);

                if output.truncated {
                    println!(
                        "      │     [...truncated {:.1}kb...]",
                        output.total_bytes as f64 / 1024.0
                    );
                }
            }
        }
        println!("      │");
    }

    // Footer
    let (cmd, args) = session.resume_command();
    let resume_str = std::iter::once(cmd)
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ");
    println!();
    println!("─── resume: {} ───", resume_str);
}

/// Get the first line of a string (truncated if needed)
fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.len() > 80 {
        format!("{}...", &line[..77])
    } else {
        line.to_string()
    }
}

/// Parse a human-friendly time string into a DateTime
/// Supports: "1 week ago", "2 days ago", "yesterday", "2025-12-01", ISO 8601
fn parse_time(s: &str) -> Result<DateTime<Utc>> {
    let s = s.trim().to_lowercase();

    // Handle relative times
    if s == "yesterday" {
        return Ok(Utc::now() - Duration::days(1));
    }
    if s == "today" {
        return Ok(Utc::now());
    }

    // Handle "N unit ago" patterns
    if s.ends_with(" ago") {
        let parts: Vec<&str> = s.trim_end_matches(" ago").split_whitespace().collect();
        if parts.len() == 2 {
            let n: i64 = parts[0].parse().map_err(|_| {
                anyhow::anyhow!("Invalid time format: {}. Try '1 week ago' or '2025-12-01'", s)
            })?;
            let unit = parts[1].trim_end_matches('s'); // "weeks" -> "week"

            let duration = match unit {
                "minute" | "min" => Duration::minutes(n),
                "hour" | "hr" => Duration::hours(n),
                "day" => Duration::days(n),
                "week" | "wk" => Duration::weeks(n),
                "month" | "mo" => Duration::days(n * 30), // Approximate
                _ => {
                    return Err(anyhow::anyhow!(
                        "Unknown time unit: {}. Use minutes, hours, days, weeks, months",
                        unit
                    ))
                }
            };

            return Ok(Utc::now() - duration);
        }
    }

    // Try parsing as ISO 8601 or date
    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Try parsing as simple date (YYYY-MM-DD)
    if let Ok(date) = chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        return Ok(date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc());
    }

    Err(anyhow::anyhow!(
        "Invalid time format: {}. Try '1 week ago', 'yesterday', or '2025-12-01'",
        s
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_parse_time_yesterday() {
        let result = parse_time("yesterday").unwrap();
        let expected = Utc::now() - Duration::days(1);
        // Allow 1 second tolerance for test execution time
        assert!((result - expected).num_seconds().abs() < 2);
    }

    #[test]
    fn test_parse_time_today() {
        let result = parse_time("today").unwrap();
        let expected = Utc::now();
        assert!((result - expected).num_seconds().abs() < 2);
    }

    #[test]
    fn test_parse_time_relative_days() {
        let result = parse_time("3 days ago").unwrap();
        let expected = Utc::now() - Duration::days(3);
        assert!((result - expected).num_seconds().abs() < 2);
    }

    #[test]
    fn test_parse_time_relative_weeks() {
        let result = parse_time("2 weeks ago").unwrap();
        let expected = Utc::now() - Duration::weeks(2);
        assert!((result - expected).num_seconds().abs() < 2);
    }

    #[test]
    fn test_parse_time_relative_hours() {
        let result = parse_time("5 hours ago").unwrap();
        let expected = Utc::now() - Duration::hours(5);
        assert!((result - expected).num_seconds().abs() < 2);
    }

    #[test]
    fn test_parse_time_relative_minutes() {
        let result = parse_time("30 minutes ago").unwrap();
        let expected = Utc::now() - Duration::minutes(30);
        assert!((result - expected).num_seconds().abs() < 2);
    }

    #[test]
    fn test_parse_time_relative_months() {
        let result = parse_time("2 months ago").unwrap();
        let expected = Utc::now() - Duration::days(60); // 2 * 30
        assert!((result - expected).num_seconds().abs() < 2);
    }

    #[test]
    fn test_parse_time_short_units() {
        // Test abbreviated units
        assert!(parse_time("1 hr ago").is_ok());
        assert!(parse_time("5 min ago").is_ok());
        assert!(parse_time("1 wk ago").is_ok());
        assert!(parse_time("1 mo ago").is_ok());
    }

    #[test]
    fn test_parse_time_date() {
        let result = parse_time("2025-12-01").unwrap();
        assert_eq!(result.year(), 2025);
        assert_eq!(result.month(), 12);
        assert_eq!(result.day(), 1);
    }

    #[test]
    fn test_parse_time_iso8601() {
        let result = parse_time("2025-12-01T14:30:00Z").unwrap();
        assert_eq!(result.year(), 2025);
        assert_eq!(result.month(), 12);
        assert_eq!(result.day(), 1);
        assert_eq!(result.hour(), 14);
        assert_eq!(result.minute(), 30);
    }

    #[test]
    fn test_parse_time_case_insensitive() {
        assert!(parse_time("YESTERDAY").is_ok());
        assert!(parse_time("Today").is_ok());
        assert!(parse_time("3 DAYS AGO").is_ok());
    }

    #[test]
    fn test_parse_time_whitespace() {
        assert!(parse_time("  yesterday  ").is_ok());
        assert!(parse_time("\tyesterday\n").is_ok());
    }

    #[test]
    fn test_parse_time_invalid() {
        assert!(parse_time("invalid").is_err());
        assert!(parse_time("a week ago").is_err()); // "a" is not a number
        assert!(parse_time("5 fortnights ago").is_err()); // unknown unit
    }
}
