use crate::app::{App, SearchScope};
use crate::session::{Role, SessionSource};
use crate::theme::Theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
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
        let chars: Vec<char> = app.query.chars().collect();
        let cursor_at_end = app.cursor >= chars.len();
        // space + query + (1 extra if cursor at end adds a space)
        let query_display_len = 1 + chars.len() + if cursor_at_end { 1 } else { 0 };
        let padding = search_width.saturating_sub(query_display_len);

        // Split query: before cursor, char at cursor (or space if at end), after cursor
        let before: String = chars[..app.cursor].iter().collect();
        let cursor_char = chars.get(app.cursor).copied().unwrap_or(' ');
        let after: String = if app.cursor < chars.len() {
            chars[app.cursor + 1..].iter().collect()
        } else {
            String::new()
        };

        let mut spans = vec![
            Span::raw(" "),
            Span::raw(before),
            Span::styled(
                cursor_char.to_string(),
                Style::default().fg(t.search_bg).bg(t.accent),
            ),
            Span::raw(after),
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
                SessionSource::Factory => t.factory_source,
                SessionSource::OpenCode => t.opencode_source,
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

            // Truncate snippet to fit available width (Tantivy already centered it)
            let snippet: String = result.snippet.chars().take(available_width).collect();
            let truncated = snippet.len() < result.snippet.len();
            let snippet = if truncated {
                format!("{}...", snippet.trim_end())
            } else {
                snippet
            };

            // Use pre-computed match spans from Tantivy for highlighting
            // Adjust spans if we truncated
            let adjusted_spans: Vec<(usize, usize)> = result
                .match_spans
                .iter()
                .filter(|&&(start, _)| start < snippet.len())
                .map(|&(start, end)| (start, end.min(snippet.len())))
                .collect();
            let snippet_spans = highlight_with_spans(&snippet, &adjusted_spans);

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

    // Store preview area for mouse click detection
    app.preview_area = (area.x, area.y, area.width, area.height);

    let Some(result) = app.selected_result() else {
        app.message_line_ranges.clear();
        return;
    };

    // Extract values we need before mutating app
    let file_path = result.session.file_path.clone();
    let matched_message_index = result.matched_message_index;
    let match_fragment = result.match_fragment.clone();

    // Load the full session for preview
    let session = match crate::parser::parse_session_file(&file_path) {
        Ok(s) => s,
        Err(_) => {
            app.message_line_ranges.clear();
            return;
        }
    };

    // Store message count for navigation
    app.preview_message_count = session.messages.len();

    // Determine focused message (default to matched message)
    let focused_idx = app.focused_message.unwrap_or(matched_message_index);

    // Build preview lines with chat bubble style
    let mut lines: Vec<Line> = Vec::new();
    // Reserve chars for: focus indicator (1-2) + bubble padding (2 left/right)
    let bubble_width = area.width.saturating_sub(5) as usize;

    // Track line ranges for each message (start, end) for mouse click mapping
    let mut message_line_ranges: Vec<(usize, usize)> = Vec::new();
    // Track line index where each message starts (for scrolling)
    let mut message_start_lines: Vec<usize> = Vec::new();

    for (i, message) in session.messages.iter().enumerate() {
        // Track where this message starts
        message_start_lines.push(lines.len());

        let is_focused = i == focused_idx;
        let is_expanded = app.expanded_messages.contains(&i);

        let (accent_color, msg_bg) = match message.role {
            Role::User => (t.user_label, t.user_bubble_bg),
            Role::Assistant => match session.source {
                crate::session::SessionSource::ClaudeCode => (t.claude_source, t.claude_bubble_bg),
                crate::session::SessionSource::CodexCli => (t.codex_source, t.codex_bubble_bg),
                crate::session::SessionSource::Factory => (t.factory_source, t.factory_bubble_bg),
                crate::session::SessionSource::OpenCode => (t.opencode_source, t.opencode_bubble_bg),
            },
        };

        // Focus indicator - ▎ for focused, space for unfocused (same width)
        let focus_prefix = Span::styled("▎", Style::default().fg(t.focus_indicator));
        let unfocused_prefix = Span::raw(" ");

        // Add spacing between messages
        if i > 0 {
            lines.push(Line::from(""));
        }

        // Role label
        let role_label = match message.role {
            Role::User => "You",
            Role::Assistant => match session.source {
                crate::session::SessionSource::ClaudeCode => "Claude",
                crate::session::SessionSource::CodexCli => "Codex",
                crate::session::SessionSource::Factory => "Droid",
                crate::session::SessionSource::OpenCode => "OpenCode",
            },
        };

        let time_str = format_time_ago(message.timestamp);

        // Role header with timestamp and focus indicator
        lines.push(Line::from(vec![
            if is_focused { focus_prefix.clone() } else { unfocused_prefix.clone() },
            Span::styled(
                role_label,
                Style::default().fg(accent_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", time_str),
                Style::default().fg(t.dim_fg),
            ),
        ]));

        // Message content with word wrapping
        let wrapped_lines = wrap_text(&message.content, bubble_width);
        let is_matched = i == matched_message_index;
        let max_lines = if is_expanded { usize::MAX } else { 12 };

        // Determine which line indices to show (use Tantivy's fragment for centering)
        let line_indices = select_lines_to_show(
            &wrapped_lines,
            is_matched,
            &match_fragment,
            max_lines,
        );
        let lines_to_show: Vec<(usize, &str)> = line_indices
            .iter()
            .map(|&idx| {
                if idx == usize::MAX {
                    (usize::MAX, "") // Truncation marker sentinel
                } else {
                    (idx, wrapped_lines[idx].as_str())
                }
            })
            .collect();

        let hidden_count = wrapped_lines.len().saturating_sub(12);

        // Track if focused message can be expanded/collapsed
        if is_focused {
            app.focused_message_expandable = wrapped_lines.len() > 12 || is_expanded;
        }

        for (line_idx, display_line) in &lines_to_show {
            let prefix = if is_focused { focus_prefix.clone() } else { unfocused_prefix.clone() };

            // Check if this is the truncation placeholder (sentinel value)
            if *line_idx == usize::MAX {
                let trunc_msg = format!("... ({} more lines)", hidden_count);
                lines.push(Line::from(vec![
                    prefix,
                    Span::styled(
                        format!(" {:<width$}", trunc_msg, width = bubble_width + 1),
                        Style::default().fg(t.dim_fg).bg(msg_bg),
                    ),
                ]));
                continue;
            }

            let content_len = display_line.chars().count();
            let right_pad = bubble_width.saturating_sub(content_len);

            // Build line: [focus indicator] [1 space padding] [content] [right padding to fill width]
            let mut spans = vec![
                prefix,
                Span::styled(" ", Style::default().bg(msg_bg)),
            ];

            if !display_line.is_empty() {
                let highlighted = highlight_matches_owned(display_line, &app.query);
                for span in highlighted {
                    spans.push(Span::styled(span.content, span.style.bg(msg_bg)));
                }
            }

            spans.push(Span::styled(" ".repeat(right_pad + 1), Style::default().bg(msg_bg)));
            lines.push(Line::from(spans));
        }

        // Record the line range for this message
        message_line_ranges.push((message_start_lines[i], lines.len()));
    }

    // Store message line ranges for mouse click detection
    app.message_line_ranges = message_line_ranges;

    // Clamp scroll to valid range (leave at least one screen of content)
    let visible_height = area.height as usize;
    let max_scroll = lines.len().saturating_sub(visible_height.min(lines.len()));
    app.preview_scrollable = max_scroll > 0;

    // Auto-scroll to focused message when pending (triggered by selection change or navigation)
    if app.pending_auto_scroll {
        if let Some(&start_line) = message_start_lines.get(focused_idx) {
            // Scroll to show focused message with some context above
            app.preview_scroll = start_line.saturating_sub(2).min(max_scroll);
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
        // Show Pg↑/↓ hint only if terminal is wide enough and there are messages
        if area.width > 90 && app.preview_message_count > 1 {
            spans.extend([
                Span::styled(" │ ", dim),
                Span::styled(" Pg↑/↓ ", keycap),
                Span::styled(" message ", label),
            ]);
        }
        // Show Ctrl+E expand/collapse hint if terminal is wide enough and message is expandable
        if area.width > 110 && app.focused_message_expandable {
            // Check if focused message is currently expanded
            let is_expanded = if let Some(result) = app.selected_result() {
                let focused = app.focused_message.unwrap_or(result.matched_message_index);
                app.expanded_messages.contains(&focused)
            } else {
                false
            };
            let action = if is_expanded { " collapse " } else { " expand " };
            spans.extend([
                Span::styled(" │ ", dim),
                Span::styled(" ^E ", keycap),
                Span::styled(action, label),
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

/// Find the wrapped line index that contains the given fragment.
/// Searches for the fragment in the joined wrapped text.
fn find_fragment_line(wrapped_lines: &[String], fragment: &str) -> usize {
    if fragment.is_empty() || wrapped_lines.is_empty() {
        return 0;
    }

    // Normalize fragment: collapse whitespace to single spaces
    let norm_fragment: String = fragment.split_whitespace().collect::<Vec<_>>().join(" ");
    if norm_fragment.is_empty() {
        return 0;
    }

    // Build cumulative text from wrapped lines, track where each line starts
    let mut cumulative = String::new();
    let mut line_starts: Vec<usize> = Vec::new();

    for line in wrapped_lines {
        line_starts.push(cumulative.len());
        if !cumulative.is_empty() {
            cumulative.push(' ');
        }
        cumulative.push_str(line);
    }

    // Normalize cumulative text the same way
    let norm_cumulative: String = cumulative.split_whitespace().collect::<Vec<_>>().join(" ");

    // Find fragment in normalized cumulative text
    if let Some(pos) = norm_cumulative.find(&norm_fragment) {
        // Find which line this position corresponds to
        // We need to map the position in normalized text back to line index
        // Build normalized line lengths to track positions
        let mut norm_char_count = 0;
        for (idx, line) in wrapped_lines.iter().enumerate() {
            let norm_line: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
            let line_len = norm_line.len();
            // Check if pos falls within this line's range
            if pos < norm_char_count + line_len {
                return idx;
            }
            norm_char_count += line_len;
            if idx < wrapped_lines.len() - 1 {
                norm_char_count += 1; // +1 for space between lines
            }
        }
    }

    0
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

/// Highlight text using pre-computed byte spans (from Tantivy)
fn highlight_with_spans(text: &str, spans: &[(usize, usize)]) -> Vec<Span<'static>> {
    let t = theme();
    if spans.is_empty() {
        return vec![Span::raw(text.to_owned())];
    }

    let mut result = Vec::new();
    let mut last_end = 0;

    for &(start, end) in spans {
        // Ensure spans are within bounds
        let start = start.min(text.len());
        let end = end.min(text.len());
        if start >= end {
            continue;
        }

        if start > last_end {
            result.push(Span::raw(text[last_end..start].to_owned()));
        }
        result.push(Span::styled(
            text[start..end].to_owned(),
            Style::default()
                .fg(t.match_fg)
                .add_modifier(Modifier::BOLD),
        ));
        last_end = end;
    }

    if last_end < text.len() {
        result.push(Span::raw(text[last_end..].to_owned()));
    }

    if result.is_empty() {
        result.push(Span::raw(text.to_owned()));
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

/// Select which line indices to show from a long message.
/// Returns a Vec of (original_line_index, is_truncation_marker).
/// The truncation marker uses usize::MAX as a sentinel value.
fn select_lines_to_show(
    wrapped_lines: &[String],
    is_matched: bool,
    match_fragment: &str,
    max_lines: usize,
) -> Vec<usize> {
    if wrapped_lines.len() <= max_lines {
        // Short message - show all
        return (0..wrapped_lines.len()).collect();
    }

    if is_matched && !match_fragment.is_empty() {
        // Matched message - center around the match by finding fragment in wrapped text
        let match_line = find_fragment_line(wrapped_lines, match_fragment);
        let half = max_lines / 2;
        let start = match_line.saturating_sub(half);
        let end = (start + max_lines).min(wrapped_lines.len());
        let start = end.saturating_sub(max_lines); // Adjust if we hit the end
        return (start..end).collect();
    }

    // Non-matched long message - show first N + last N
    let head_count = 6.min(wrapped_lines.len());
    let tail_count = 5.min(wrapped_lines.len().saturating_sub(head_count));
    let tail_start = wrapped_lines.len().saturating_sub(tail_count);

    let mut result: Vec<usize> = (0..head_count).collect();
    result.push(usize::MAX); // Truncation indicator sentinel
    result.extend(tail_start..wrapped_lines.len());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_text_short_line() {
        let lines = wrap_text("Hello world", 80);
        assert_eq!(lines, vec!["Hello world"]);
    }

    #[test]
    fn test_wrap_text_exact_width() {
        let lines = wrap_text("Hello world", 11);
        assert_eq!(lines, vec!["Hello world"]);
    }

    #[test]
    fn test_wrap_text_wraps_long_line() {
        let lines = wrap_text("Hello world this is a test", 12);
        assert_eq!(lines, vec!["Hello world", "this is a", "test"]);
    }

    #[test]
    fn test_wrap_text_preserves_newlines() {
        let lines = wrap_text("Line one\nLine two\nLine three", 80);
        assert_eq!(lines, vec!["Line one", "Line two", "Line three"]);
    }

    #[test]
    fn test_wrap_text_blank_lines() {
        let lines = wrap_text("Line one\n\nLine three", 80);
        assert_eq!(lines, vec!["Line one", "", "Line three"]);
    }

    #[test]
    fn test_wrap_text_long_word() {
        let lines = wrap_text("supercalifragilisticexpialidocious", 10);
        assert_eq!(lines, vec!["supercalif", "ragilistic", "expialidoc", "ious"]);
    }

    #[test]
    fn test_select_lines_short_message() {
        let lines: Vec<String> = (0..5).map(|i| format!("Line {}", i)).collect();
        let result = select_lines_to_show(&lines, false, "", 12);
        assert_eq!(result, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_select_lines_long_unmatched_message() {
        let lines: Vec<String> = (0..30).map(|i| format!("Line {}", i)).collect();
        let result = select_lines_to_show(&lines, false, "", 12);

        // Should have: 6 head + 1 truncation marker + 5 tail = 12 entries
        // But after blank trimming, might be slightly different
        // The truncation marker should be usize::MAX
        assert!(result.contains(&usize::MAX), "Should contain truncation marker");

        // Count actual lines (excluding truncation marker)
        let line_count = result.iter().filter(|&&i| i != usize::MAX).count();
        assert!(line_count <= 11, "Should show at most 11 content lines, got {}", line_count);
    }

    #[test]
    fn test_select_lines_matched_message() {
        // Create wrapped lines where "MATCH keyword" appears at a known line
        let lines: Vec<String> = vec![
            "Line 0", "Line 1", "Line 2", "Line 3", "Line 4", "Line 5",
            "Line 6", "Line 7", "Line 8", "Line 9", "Line 10", "Line 11",
            "Line 12", "Line 13", "Line 14",
            "This line contains the MATCH keyword",
            "Line 16", "Line 17", "Line 18", "Line 19", "Line 20", "Line 21",
            "Line 22", "Line 23", "Line 24", "Line 25", "Line 26", "Line 27",
            "Line 28", "Line 29",
        ].into_iter().map(String::from).collect();

        // Use a fragment that would come from Tantivy
        let fragment = "contains the MATCH keyword";

        let result = select_lines_to_show(&lines, true, fragment, 12);

        // Should NOT contain truncation marker for matched messages
        assert!(!result.contains(&usize::MAX), "Matched message shouldn't have truncation marker");

        // Should show exactly max_lines
        assert_eq!(result.len(), 12, "Should show exactly 12 lines");

        // Should include line 15 (the match)
        assert!(result.contains(&15), "Should include the matched line");
    }

    #[test]
    fn test_select_lines_27_line_message() {
        let lines: Vec<String> = (0..27).map(|i| format!("Content line {}", i)).collect();

        // Test unmatched case
        let result = select_lines_to_show(&lines, false, "", 12);
        let line_count = result.iter().filter(|&&i| i != usize::MAX).count();
        assert!(line_count <= 11, "Unmatched 27-line msg should show at most 11 lines, got {}", line_count);

        // Test matched case - use fragment from line 13
        let fragment = "Content line 13";
        let result = select_lines_to_show(&lines, true, fragment, 12);
        assert_eq!(result.len(), 12, "Matched 27-line msg should show exactly 12 lines");
        assert!(!result.contains(&usize::MAX), "Matched message shouldn't have truncation marker");
    }

    #[test]
    fn test_select_lines_with_blank_lines() {
        let mut lines: Vec<String> = (0..20).map(|i| format!("Line {}", i)).collect();
        // Add some blank lines near the truncation boundaries
        lines[5] = String::new();
        lines[6] = String::new();
        lines[14] = String::new();
        lines[15] = String::new();

        let result = select_lines_to_show(&lines, false, "", 12);
        let line_count = result.iter().filter(|&&i| i != usize::MAX).count();
        // Simplified algorithm: always shows exactly 6 head + 5 tail (no blank trimming)
        assert_eq!(line_count, 11, "Should show exactly 11 lines");
    }

    #[test]
    fn test_find_fragment_line() {
        let lines: Vec<String> = vec![
            "First line",
            "Second line",
            "Third has MATCH here",
            "Fourth line",
        ].into_iter().map(String::from).collect();

        // Fragment containing "MATCH"
        assert_eq!(find_fragment_line(&lines, "MATCH here"), 2);

        // Empty fragment
        assert_eq!(find_fragment_line(&lines, ""), 0);

        // Fragment from first line
        assert_eq!(find_fragment_line(&lines, "First line"), 0);

        // Fragment from second line
        assert_eq!(find_fragment_line(&lines, "Second line"), 1);
    }

    #[test]
    fn test_find_fragment_line_wrapped() {
        // Simulate wrapped lines where a phrase spans lines
        let lines: Vec<String> = vec![
            "This is a long",
            "message that was",
            "wrapped at word",
            "boundaries for display",
        ].into_iter().map(String::from).collect();

        // Fragment that spans across wrapped lines
        assert_eq!(find_fragment_line(&lines, "message that was wrapped"), 1);

        // Fragment from the middle
        assert_eq!(find_fragment_line(&lines, "at word boundaries"), 2);
    }

    #[test]
    fn test_select_lines_realistic_long_message() {
        // Simulate a real Claude response with ~40 wrapped lines
        let lines: Vec<String> = vec![
            "I'll help you add support for Factory/Droid conversations. Let me",
            "first explore the existing codebase structure and then examine the",
            "Factory session files to understand their format.",
            "Let me check more of the Factory session format and the index",
            "module:",
            "Let me look more closely at the Factory sessions structure:",
            "Now I have all the information I need. Let me implement Factory",
            "support.",
            "Now let me test the TUI with tmux to verify Factory sessions are",
            "discovered and displayed correctly:",
            "Factory sessions are appearing with the `◆ Factory` indicator. Let",
            "me navigate and test opening one:",
            "Let me fix the warning about the unused field:",
            "Done! The Factory/Droid support implementation is ready for your",
            "review and testing.",
            "",
            "## Summary of changes",
            "",
            "**New file:**",
            "- `src/parser/factory.rs` - Parser for Factory sessions at",
            "`~/.factory/sessions/`",
            "",
            "**Modified files:**",
            "- `src/session.rs` - Added `Factory` variant to `SessionSource`",
            "- `src/parser/mod.rs` - Added Factory discovery and parsing",
            "- `src/theme.rs` - Added Factory-specific colors",
            "- `src/ui.rs` - Added Factory handling in source-specific displays",
            "",
            "## Key features",
            "",
            "1. Parses Factory JSONL format",
            "2. Shows `◆ Factory` indicator in list",
            "3. Uses purple theme colors for Factory messages",
            "4. Resume command: `droid --resume {id}`",
            "",
            "```bash",
            "cargo build && ./target/debug/recall",
            "# Then press / to toggle to everywhere scope",
            "```",
        ].into_iter().map(String::from).collect();

        // Non-matched case - should get head + truncation + tail
        let result = select_lines_to_show(&lines, false, "", 12);

        // Count actual lines (excluding truncation marker)
        let line_count = result.iter().filter(|&&i| i != usize::MAX).count();

        // Debug: print what we got
        eprintln!("Total lines: {}", lines.len());
        eprintln!("Result indices: {:?}", result);
        eprintln!("Line count (excl. marker): {}", line_count);

        assert!(result.contains(&usize::MAX), "Should have truncation marker");
        assert_eq!(line_count, 11, "Should show exactly 11 lines (6 head + 5 tail)");

        // Verify head is exactly 6 lines and tail is exactly 5 lines
        let marker_pos = result.iter().position(|&i| i == usize::MAX).unwrap();
        let head_count = marker_pos;
        let tail_count = result.len() - marker_pos - 1;

        eprintln!("Head count: {}, Tail count: {}", head_count, tail_count);

        assert_eq!(head_count, 6, "Head should be exactly 6 lines");
        assert_eq!(tail_count, 5, "Tail should be exactly 5 lines");
    }
}
