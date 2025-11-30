use ratatui::style::Color;

/// Terminal theme colors, adapts to light/dark mode
pub struct Theme {
    /// Background for selected items
    pub selection_bg: Color,
    /// Text color for selected item header
    pub selection_header_fg: Color,
    /// Text color for selected item snippet
    pub selection_snippet_fg: Color,
    /// Text color for unselected item snippet
    pub snippet_fg: Color,
    /// Highlight color for search matches
    pub match_fg: Color,
    /// Search bar background
    pub search_bg: Color,
    /// Placeholder text color
    pub placeholder_fg: Color,
    /// Accent color (cursor, user messages)
    pub accent: Color,
    /// Secondary accent (assistant messages)
    pub accent_secondary: Color,
    /// Dim text (status bar, etc)
    pub dim_fg: Color,
    /// Keycap background in status bar
    pub keycap_bg: Color,
    /// User message bubble background
    pub user_bubble_bg: Color,
    /// User label color (matches user bubble)
    pub user_label: Color,
    /// Claude message bubble background
    pub claude_bubble_bg: Color,
    /// Codex message bubble background
    pub codex_bubble_bg: Color,
    /// Claude source indicator color
    pub claude_source: Color,
    /// Codex source indicator color
    pub codex_source: Color,
    /// Scope indicator background (slightly different from search_bg)
    pub scope_bg: Color,
    /// Scope keycap background (for "/" key)
    pub scope_key_bg: Color,
    /// Separator color in search bar
    pub separator_fg: Color,
    /// Scope label text color
    pub scope_label_fg: Color,
}

impl Theme {
    pub fn detect() -> Self {
        let is_light = detect_light_theme();
        if is_light {
            Self::light()
        } else {
            Self::dark()
        }
    }

    fn dark() -> Self {
        Self {
            selection_bg: Color::Rgb(50, 50, 55),
            selection_header_fg: Color::Cyan,
            selection_snippet_fg: Color::Rgb(180, 180, 180),
            snippet_fg: Color::Rgb(120, 120, 120),
            match_fg: Color::Yellow,
            search_bg: Color::Rgb(30, 30, 35),
            placeholder_fg: Color::Rgb(100, 100, 100),
            accent: Color::Cyan,
            accent_secondary: Color::Green,
            dim_fg: Color::Rgb(100, 100, 100),
            keycap_bg: Color::Rgb(60, 60, 65),
            user_bubble_bg: Color::Rgb(30, 45, 55),      // subtle cyan tint
            user_label: Color::Rgb(80, 180, 220),     // bright cyan to match bubble
            claude_bubble_bg: Color::Rgb(45, 35, 30), // subtle orange tint
            codex_bubble_bg: Color::Rgb(30, 45, 35),  // subtle green tint
            claude_source: Color::Rgb(255, 150, 50),  // Anthropic orange
            codex_source: Color::Rgb(80, 200, 120),   // OpenAI green
            scope_bg: Color::Rgb(45, 45, 50),         // slightly lighter than search_bg
            scope_key_bg: Color::Rgb(60, 60, 65),     // keycap style
            separator_fg: Color::Rgb(60, 60, 65),     // subtle separator
            scope_label_fg: Color::Rgb(140, 140, 140), // readable but not bright
        }
    }

    fn light() -> Self {
        Self {
            selection_bg: Color::Rgb(220, 220, 225),
            selection_header_fg: Color::Rgb(0, 120, 150),
            selection_snippet_fg: Color::Rgb(60, 60, 60),
            snippet_fg: Color::Rgb(120, 120, 120),
            match_fg: Color::Rgb(180, 120, 0),
            search_bg: Color::Rgb(235, 235, 240),
            placeholder_fg: Color::Rgb(150, 150, 150),
            accent: Color::Rgb(0, 150, 180),
            accent_secondary: Color::Rgb(0, 140, 80),
            dim_fg: Color::Rgb(140, 140, 140),
            keycap_bg: Color::Rgb(200, 200, 205),
            user_bubble_bg: Color::Rgb(220, 235, 245),   // subtle cyan tint
            user_label: Color::Rgb(40, 130, 180),      // darker cyan for light bg
            claude_bubble_bg: Color::Rgb(250, 235, 220), // subtle orange tint
            codex_bubble_bg: Color::Rgb(220, 245, 225),  // subtle green tint
            claude_source: Color::Rgb(200, 100, 20),   // Anthropic orange (darker for light bg)
            codex_source: Color::Rgb(30, 140, 70),    // OpenAI green (darker for light bg)
            scope_bg: Color::Rgb(215, 215, 220),      // slightly darker than search_bg
            scope_key_bg: Color::Rgb(200, 200, 205),  // keycap style
            separator_fg: Color::Rgb(195, 195, 200),  // visible on light bg
            scope_label_fg: Color::Rgb(100, 100, 100), // readable on light bg
        }
    }
}

/// Detect if terminal has a light background
fn detect_light_theme() -> bool {
    // Try to query terminal's actual background color
    if let Some(bg) = query_terminal_bg() {
        return is_light(bg);
    }

    // Fallback: Check COLORFGBG env var (format: "fg;bg" where 15=white, 0=black)
    if let Ok(val) = std::env::var("COLORFGBG") {
        if let Some(bg) = val.split(';').next_back() {
            if let Ok(bg_num) = bg.parse::<u8>() {
                return bg_num >= 7;
            }
        }
    }

    // Default to dark theme (most common for developers)
    false
}

/// Check if a color is perceptually light
fn is_light(color: (u8, u8, u8)) -> bool {
    let (r, g, b) = color;
    // Luminance formula (ITU-R BT.601)
    let y = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    y > 128.0
}

/// Query terminal background color using crossterm
#[cfg(unix)]
fn query_terminal_bg() -> Option<(u8, u8, u8)> {
    use crossterm::style::{query_background_color, Color as CtColor};
    use std::sync::OnceLock;

    // Cache the result since querying can be slow
    static CACHED: OnceLock<Option<(u8, u8, u8)>> = OnceLock::new();

    *CACHED.get_or_init(|| {
        query_background_color()
            .ok()
            .flatten()
            .and_then(|c| match c {
                CtColor::Rgb { r, g, b } => Some((r, g, b)),
                _ => None,
            })
    })
}

#[cfg(not(unix))]
fn query_terminal_bg() -> Option<(u8, u8, u8)> {
    None
}
