use crate::app::{App, SearchScope};
use crate::session::{Role, SessionSource};
use crate::theme::Theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};
use std::sync::OnceLock;

fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(Theme::detect)
}

/// Main UI rendering
pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Main layout: search bar (3 lines with padding), spacing, content, spacing, status bar
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Search bar with top/bottom padding
            Constraint::Length(1), // Spacing
            Constraint::Min(0),    // Content area
            Constraint::Length(1), // Spacing before status bar
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Add horizontal margin around search bar
    let search_with_margin = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(1), // Left margin
            Constraint::Min(0),    // Search bar
            Constraint::Length(1), // Right margin
        ])
        .split(main_layout[0]);

    render_search_bar(frame, app, search_with_margin[1]);
    // main_layout[1] is spacing - left empty

    // Add horizontal padding (1 char each side)
    let content_with_padding = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(1), // Left padding
            Constraint::Min(0),    // Content
            Constraint::Length(1), // Right padding
        ])
        .split(main_layout[2]);

    // When no results, use full width for the hint message
    if app.results.is_empty() {
        render_results_list(frame, app, content_with_padding[1]);
    } else {
        // Two-pane layout: 40% results, 2 space padding, 60% preview
        let content_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Length(2), // Padding between panes
                Constraint::Percentage(60),
            ])
            .split(content_with_padding[1]);

        render_results_list(frame, app, content_layout[0]);
        // content_layout[1] is the padding space - left empty
        render_preview(frame, app, content_layout[2]);
    }

    // Add horizontal padding to status bar
    let status_with_padding = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(1), // Left padding
            Constraint::Min(0),    // Status bar
            Constraint::Length(1), // Right padding
        ])
        .split(main_layout[4]);

    render_status_bar(frame, app, status_with_padding[1]);
}

fn render_search_bar(frame: &mut Frame, app: &App, area: Rect) {
    let t = theme();

    // Scope widget content
    let scope_label = match app.scope_display_path() {
        Some(path) => path,
        None => "everywhere".to_string(),
    };

    // Widget: separator + keycap + label (no bg on label)
    let separator_color = t.separator_fg;
    let label_color = t.scope_label_fg;
    let scope_widget = vec![
        Span::styled(" │ ", Style::default().fg(separator_color)),  // separator
        Span::styled(" / ", Style::default().bg(t.keycap_bg)),  // keycap like status bar
        Span::styled(format!(" {} ", scope_label), Style::default().fg(label_color)),  // label
    ];
    let scope_width: usize = 3 + 3 + 1 + scope_label.len() + 1; // " │ " + " / " + " label "

    // Calculate how much space for search text (leave room for scope widget + left margin)
    let search_width = (area.width as usize).saturating_sub(scope_width + 1); // +1 for left margin before widget

    // Build middle line with search on left, scope widget on right
    let middle_line = if app.query.is_empty() {
        let placeholder = " Search...";
        let padding = search_width.saturating_sub(placeholder.len());
        let mut spans = vec![
            Span::styled(placeholder, Style::default().fg(t.placeholder_fg)),
            Span::styled(" ".repeat(padding), Style::default()), // fill to push scope right
            Span::styled(" ", Style::default()), // margin before widget
        ];
        spans.extend(scope_widget.clone());
        Line::from(spans)
    } else {
        let query_len = 1 + app.query.chars().count() + 1; // space + query + cursor
        let padding = search_width.saturating_sub(query_len);
        let mut spans = vec![
            Span::raw(" "),
            Span::raw(&app.query),
            Span::styled("█", Style::default().fg(t.accent)),
            Span::raw(" ".repeat(padding)), // fill to push scope right
            Span::styled(" ", Style::default()), // margin before widget
        ];
        spans.extend(scope_widget.clone());
        Line::from(spans)
    };

    // Top and bottom lines need separator at same position
    let separator_pos = search_width + 1; // +1 for margin before widget
    let top_line = Line::from(vec![
        Span::raw(" ".repeat(separator_pos)),
        Span::styled(" │ ", Style::default().fg(separator_color)),
    ]);
    let bottom_line = Line::from(vec![
        Span::raw(" ".repeat(separator_pos)),
        Span::styled(" │ ", Style::default().fg(separator_color)),
    ]);
    let lines = vec![top_line, middle_line, bottom_line];

    let paragraph = Paragraph::new(lines)
        .style(Style::default().bg(t.search_bg));

    frame.render_widget(paragraph, area);
}

fn render_results_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = theme();
    // Available width for snippet text
    let available_width = area.width.saturating_sub(2) as usize;

    if app.results.is_empty() {
        // Show hint to search everywhere if scoped and no results
        let is_scoped = !matches!(app.search_scope, SearchScope::Everything);
        if is_scoped {
            let prefix = if app.query.is_empty() { "Nothing here." } else { "No results." };
            let hint = Line::from(vec![
                Span::styled(format!(" {} Press ", prefix), Style::default().fg(t.snippet_fg)),
                Span::styled(" / ", Style::default().bg(t.keycap_bg)),
                Span::styled(" to search everywhere.", Style::default().fg(t.snippet_fg)),
            ]);
            frame.render_widget(Paragraph::new(hint), area);
        } else if !app.query.is_empty() {
            let paragraph = Paragraph::new(Span::styled(
                " No results.",
                Style::default().fg(t.snippet_fg),
            ));
            frame.render_widget(paragraph, area);
        }
        return;
    }

    let items: Vec<ListItem> = app
        .results
        .iter()
        .enumerate()
        .map(|(i, result)| {
            let is_selected = i == app.selected;

            // Format time ago
            let time_ago = format_time_ago(result.session.timestamp);

            // Source-specific color
            let source_color = match result.session.source {
                SessionSource::ClaudeCode => t.claude_source,
                SessionSource::CodexCli => t.codex_source,
            };

            // Build header with colored source indicator
            let header_style = if is_selected {
                Style::default().fg(t.selection_header_fg)
            } else {
                Style::default()
            };

            let header_spans = vec![
                Span::styled("📁 ", header_style),
                Span::styled(result.session.project_name(), header_style),
                Span::styled("  ", header_style),
                Span::styled(
                    format!("{} {}", result.session.source.icon(), result.session.source.display_name()),
                    Style::default().fg(source_color),
                ),
                Span::styled(format!("  {}", time_ago), header_style),
            ];

            // Truncate snippet to fit available width, keeping match centered
            let snippet = truncate_snippet_around_match(&result.snippet, &app.query, available_width);

            // Snippet with highlighted matches (owned version for local variable)
            let snippet_spans = highlight_matches_owned(&snippet, &app.query);

            let lines = vec![
                Line::from(header_spans),
                Line::from(
                    snippet_spans
                        .into_iter()
                        .map(|s| {
                            if s.style.add_modifier.contains(Modifier::BOLD) {
                                // Highlight for matches
                                Span::styled(s.content, Style::default().fg(t.match_fg).add_modifier(Modifier::BOLD))
                            } else {
                                let fg = if is_selected { t.selection_snippet_fg } else { t.snippet_fg };
                                Span::styled(s.content, Style::default().fg(fg))
                            }
                        })
                        .collect::<Vec<_>>(),
                ),
                Line::from(""), // Empty line between conversations
            ];

            if is_selected {
                ListItem::new(lines).style(Style::default().bg(t.selection_bg))
            } else {
                ListItem::new(lines)
            }
        })
        .collect();

    let list = List::new(items);

    // Calculate visible items (each item is 3 lines: header, snippet, empty)
    let lines_per_item = 3;
    let visible_items = (area.height as usize) / lines_per_item;

    // Update scroll offset to keep selected item visible
    if app.selected < app.list_scroll {
        // Selected above visible area - scroll up
        app.list_scroll = app.selected;
    } else if app.selected >= app.list_scroll + visible_items && visible_items > 0 {
        // Selected below visible area - scroll down
        app.list_scroll = app.selected - visible_items + 1;
    }

    // Use ListState with our tracked scroll offset
    let mut list_state = ListState::default();
    list_state.select(Some(app.selected));
    *list_state.offset_mut() = app.list_scroll;

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn render_preview(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = theme();
    let Some(result) = app.selected_result() else {
        return;
    };

    // Load the full session for preview
    let session = match crate::parser::parse_session_file(&result.session.file_path) {
        Ok(s) => s,
        Err(_) => {
            return;
        }
    };

    // Build preview lines with chat bubble style
    let mut lines: Vec<Line> = Vec::new();
    let bubble_width = area.width.saturating_sub(4) as usize;

    let mut prev_role: Option<Role> = None;
    let mut matched_line: Option<usize> = None;

    for (i, message) in session.messages.iter().enumerate() {
        // Track where the matched message starts
        if i == result.matched_message_index {
            matched_line = Some(lines.len());
        }
        let (accent_color, msg_bg) = match message.role {
            Role::User => (t.user_label, t.user_bubble_bg),
            Role::Assistant => match session.source {
                crate::session::SessionSource::ClaudeCode => (t.claude_source, t.claude_bubble_bg),
                crate::session::SessionSource::CodexCli => (t.codex_source, t.codex_bubble_bg),
            },
        };

        let speaker_changed = prev_role != Some(message.role);

        // Add spacing between speakers (not between consecutive same-speaker messages)
        if i > 0 && speaker_changed {
            lines.push(Line::from(""));
        }

        // Only show role label when speaker changes
        if speaker_changed {
            let role_label = match message.role {
                Role::User => " You",
                Role::Assistant => match session.source {
                    crate::session::SessionSource::ClaudeCode => " Claude",
                    crate::session::SessionSource::CodexCli => " Codex",
                },
            };

            let time_str = format_time_ago(message.timestamp);

            // Role header with timestamp
            lines.push(Line::from(vec![
                Span::styled(
                    role_label,
                    Style::default().fg(accent_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", time_str),
                    Style::default().fg(t.dim_fg),
                ),
            ]));
        }

        prev_role = Some(message.role);

        // Message content with word wrapping
        let wrapped_lines = wrap_text(&message.content, bubble_width);

        for (line_idx, display_line) in wrapped_lines.iter().take(12).enumerate() {
            let content_len = display_line.chars().count();
            let right_pad = bubble_width.saturating_sub(content_len);

            // Build line: [1 space padding] [content] [right padding to fill width]
            let mut spans = vec![Span::styled(" ", Style::default().bg(msg_bg))];

            if !display_line.is_empty() {
                let highlighted = highlight_matches_owned(display_line, &app.query);
                for span in highlighted {
                    spans.push(Span::styled(span.content, span.style.bg(msg_bg)));
                }
            }

            spans.push(Span::styled(" ".repeat(right_pad + 1), Style::default().bg(msg_bg)));
            lines.push(Line::from(spans));

            // Show truncation indicator
            if line_idx == 11 && wrapped_lines.len() > 12 {
                let trunc_msg = format!("... ({} more lines)", wrapped_lines.len() - 12);
                lines.push(Line::from(Span::styled(
                    format!(" {:<width$}", trunc_msg, width = bubble_width + 1),
                    Style::default().fg(t.dim_fg).bg(msg_bg),
                )));
            }
        }
    }

    // Clamp scroll to valid range (leave at least one screen of content)
    let visible_height = area.height as usize;
    let max_scroll = lines.len().saturating_sub(visible_height.min(lines.len()));
    app.preview_scrollable = max_scroll > 0;

    // Auto-scroll to matched message when pending (triggered by selection/query change)
    if app.pending_auto_scroll {
        if let Some(line) = matched_line {
            // Scroll to show matched message with some context above
            app.preview_scroll = line.saturating_sub(3).min(max_scroll);
        }
        app.pending_auto_scroll = false;
    }

    app.preview_scroll = app.preview_scroll.min(max_scroll);

    // Use app's preview_scroll for manual scrolling
    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(app.preview_scroll)
        .collect();

    let paragraph = Paragraph::new(visible_lines);

    frame.render_widget(paragraph, area);
}

fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let t = theme();
    let keycap = Style::default().bg(t.keycap_bg);
    let label = Style::default();
    let dim = Style::default().fg(t.dim_fg);

    let hints: Line = if let Some(ref msg) = app.status {
        Line::from(Span::styled(msg, Style::default().fg(t.match_fg)))
    } else {
        let has_selection = !app.results.is_empty();
        let mut spans = vec![
            Span::styled(" ↑↓ ", keycap),
            Span::styled(" navigate ", label),
        ];
        // Show Enter/Tab only when there's a selection
        if has_selection {
            spans.extend([
                Span::styled(" │ ", dim),
                Span::styled(" Enter ", keycap),
                Span::styled(" open ", label),
                Span::styled(" │ ", dim),
                Span::styled(" Tab ", keycap),
                Span::styled(" copy ID ", label),
            ]);
        }
        // Show Pg↑/↓ hint only if terminal is wide enough and preview is scrollable
        if area.width > 90 && app.preview_scrollable {
            spans.extend([
                Span::styled(" │ ", dim),
                Span::styled(" Pg↑/↓ ", keycap),
                Span::styled(" scroll ", label),
            ]);
        }
        spans.extend([
            Span::styled(" │ ", dim),
            Span::styled(" Esc ", keycap),
            Span::styled(" quit", label),
        ]);
        Line::from(spans)
    };

    let sessions_count = Span::styled(
        format!(" {} sessions", app.total_sessions),
        dim,
    );

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(sessions_count.width() as u16)])
        .split(area);

    frame.render_widget(Paragraph::new(hints), layout[0]);
    frame.render_widget(Paragraph::new(sessions_count), layout[1]);
}

/// Word-wrap text to fit within max_width characters
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let mut result = Vec::new();

    for line in text.lines() {
        // Empty or whitespace-only lines become blank lines
        if line.trim().is_empty() {
            result.push(String::new());
            continue;
        }

        let mut current_line = String::new();
        let mut current_width = 0;

        for word in line.split_whitespace() {
            let word_width = word.chars().count();

            if current_width == 0 {
                // First word on line
                if word_width > max_width {
                    // Word too long, force break it
                    for chunk in word.chars().collect::<Vec<_>>().chunks(max_width) {
                        result.push(chunk.iter().collect());
                    }
                } else {
                    current_line = word.to_string();
                    current_width = word_width;
                }
            } else if current_width + 1 + word_width <= max_width {
                // Word fits on current line
                current_line.push(' ');
                current_line.push_str(word);
                current_width += 1 + word_width;
            } else {
                // Word doesn't fit, start new line
                result.push(current_line);
                if word_width > max_width {
                    // Word too long, force break it
                    for chunk in word.chars().collect::<Vec<_>>().chunks(max_width) {
                        result.push(chunk.iter().collect());
                    }
                    current_line = String::new();
                    current_width = 0;
                } else {
                    current_line = word.to_string();
                    current_width = word_width;
                }
            }
        }

        if !current_line.is_empty() {
            result.push(current_line);
        }
    }

    if result.is_empty() {
        result.push(String::new());
    }

    result
}

/// Highlight query matches, returning owned Spans (for use with local variables)
/// Splits query into words and highlights each word separately
fn highlight_matches_owned(text: &str, query: &str) -> Vec<Span<'static>> {
    let t = theme();
    if query.is_empty() {
        return vec![Span::raw(text.to_owned())];
    }

    let lower_text = text.to_lowercase();

    // Split query into words and find all match positions
    let query_words: Vec<&str> = query.split_whitespace().filter(|w| !w.is_empty()).collect();
    if query_words.is_empty() {
        return vec![Span::raw(text.to_owned())];
    }

    // Collect all match ranges (byte positions in original text)
    let mut matches: Vec<(usize, usize)> = Vec::new();
    for word in &query_words {
        let lower_word = word.to_lowercase();
        for (match_start_lower, matched_str) in lower_text.match_indices(&lower_word) {
            let char_offset = lower_text[..match_start_lower].chars().count();
            let start = text.char_indices().nth(char_offset).map(|(i, _)| i).unwrap_or(text.len());

            let match_char_len = matched_str.chars().count();
            let end = text[start..].char_indices()
                .nth(match_char_len)
                .map(|(i, _)| start + i)
                .unwrap_or(text.len());

            matches.push((start, end));
        }
    }

    // Sort by start position and merge overlapping ranges
    matches.sort_by_key(|m| m.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in matches {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    // Build spans
    let mut spans = Vec::new();
    let mut last_end = 0;

    for (start, end) in merged {
        if start > last_end {
            spans.push(Span::raw(text[last_end..start].to_owned()));
        }
        spans.push(Span::styled(
            text[start..end].to_owned(),
            Style::default()
                .fg(t.match_fg)
                .add_modifier(Modifier::BOLD),
        ));
        last_end = end;
    }

    if last_end < text.len() {
        spans.push(Span::raw(text[last_end..].to_owned()));
    }

    if spans.is_empty() {
        spans.push(Span::raw(text.to_owned()));
    }

    spans
}

/// Truncate snippet to fit available width, keeping the match visible and centered
/// Finds the first matching word from the query to center around
fn truncate_snippet_around_match(snippet: &str, query: &str, max_width: usize) -> String {
    let chars: Vec<char> = snippet.chars().collect();
    if chars.len() <= max_width {
        return snippet.to_string();
    }

    let lower_snippet: String = chars.iter().collect::<String>().to_lowercase();

    // Find the first matching word from the query
    let query_words: Vec<&str> = query.split_whitespace().collect();
    let mut best_match: Option<(usize, usize)> = None; // (char_pos, word_len)

    for word in &query_words {
        let lower_word = word.to_lowercase();
        if let Some(byte_pos) = lower_snippet.find(&lower_word) {
            let char_pos = lower_snippet[..byte_pos].chars().count();
            let word_len = lower_word.chars().count();
            if best_match.is_none() || char_pos < best_match.unwrap().0 {
                best_match = Some((char_pos, word_len));
            }
        }
    }

    if let Some((match_char_pos, query_char_len)) = best_match {
        // Center the match in available width
        let half_width = max_width.saturating_sub(query_char_len) / 2;
        let start = match_char_pos.saturating_sub(half_width);
        let end = (start + max_width).min(chars.len());
        let start = if end == chars.len() {
            end.saturating_sub(max_width)
        } else {
            start
        };

        let mut result: String = chars[start..end].iter().collect();

        if start > 0 {
            result = format!("...{}", result.trim_start());
        }
        if end < chars.len() {
            result = format!("{}...", result.trim_end());
        }

        result
    } else {
        // No match, just truncate from start
        let truncated: String = chars.iter().take(max_width).collect();
        if chars.len() > max_width {
            format!("{}...", truncated.trim_end())
        } else {
            truncated
        }
    }
}

/// Highlight query matches in text (case-insensitive, Unicode-safe)
#[allow(dead_code)]
fn highlight_matches<'a>(text: &'a str, query: &str) -> Vec<Span<'a>> {
    if query.is_empty() {
        return vec![Span::raw(text)];
    }

    let lower_text = text.to_lowercase();
    let lower_query = query.to_lowercase();

    let mut spans = Vec::new();
    let mut last_end = 0;

    for (match_start_lower, matched_str) in lower_text.match_indices(&lower_query) {
        // Find corresponding position in original text
        // Count chars up to match_start_lower in lowercased text, then find byte pos in original
        let char_offset = lower_text[..match_start_lower].chars().count();
        let start = text.char_indices().nth(char_offset).map(|(i, _)| i).unwrap_or(text.len());

        // Find end position: start + same number of chars as the match
        let match_char_len = matched_str.chars().count();
        let end = text[start..].char_indices()
            .nth(match_char_len)
            .map(|(i, _)| start + i)
            .unwrap_or(text.len());

        // Text before match
        if start > last_end {
            spans.push(Span::raw(&text[last_end..start]));
        }
        // Highlighted match (use original text casing)
        spans.push(Span::styled(
            &text[start..end],
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        last_end = end;
    }

    // Remaining text
    if last_end < text.len() {
        spans.push(Span::raw(&text[last_end..]));
    }

    if spans.is_empty() {
        spans.push(Span::raw(text));
    }

    spans
}

/// Format a timestamp as a human-readable "time ago" string
fn format_time_ago(timestamp: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(timestamp);

    if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{}m ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_days() < 7 {
        format!("{}d ago", duration.num_days())
    } else if duration.num_weeks() < 4 {
        format!("{}w ago", duration.num_weeks())
    } else {
        timestamp.format("%b %d").to_string()
    }
}
