//! Colour theme.

use ratatui::style::Color;

/// Theme palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Accent colour for titles.
    pub accent: Color,
    /// Border colour.
    pub border: Color,
    /// Muted status text.
    pub muted: Color,
}

/// Default dark theme.
#[must_use]
pub const fn knect_dark() -> Theme {
    Theme {
        accent: Color::Cyan,
        border: Color::DarkGray,
        muted: Color::Gray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_is_deterministic() {
        assert_eq!(knect_dark(), knect_dark());
        assert_eq!(knect_dark().accent, Color::Cyan);
    }
}
