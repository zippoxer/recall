//! CLI subcommands for non-interactive mode (JSON output for agents)

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use recall::{
    index::{ensure_index_fresh, SessionIndex},
    parser,
    session::{ListOutput, Message, SearchOutput, SearchResultOutput, SessionSource},
};

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

/// Run the read subcommand
pub fn run_read(session_id: &str) -> Result<()> {
    let index = SessionIndex::open_default()?;
    ensure_index_fresh(&index)?;

    // Find the session by ID
    let file_path = index
        .get_by_id(session_id)?
        .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

    // Parse full session
    let session = parser::parse_session_file(&file_path)?;
    let output = session.to_read_output();

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
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
