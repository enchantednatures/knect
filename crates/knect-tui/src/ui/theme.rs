//! Colour theme for the dashboard.
//!
//! Every colour is paired with a text label or badge in the UI, so health and
//! scale state remain readable on monochrome terminals.

use knect_proto::graph::Health;
use ratatui::style::{Color, Modifier, Style};

/// Theme palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Accent colour for titles.
    pub accent: Color,
    /// Border colour for unfocused panes.
    pub border: Color,
    /// Border colour for the focused pane.
    pub focus: Color,
    /// Muted status text.
    pub muted: Color,
    /// Primary text.
    pub text: Color,
    /// Selection background.
    pub selection_bg: Color,
    /// Selection foreground.
    pub selection_fg: Color,
    /// Healthy state.
    pub healthy: Color,
    /// Degraded state.
    pub degraded: Color,
    /// Progressing state.
    pub progressing: Color,
    /// Blocked state.
    pub blocked: Color,
    /// Unknown state.
    pub unknown: Color,
    /// Eventing resources.
    pub eventing: Color,
    /// Warning banner.
    pub warning: Color,
    /// Error banner.
    pub error: Color,
}

/// Default dark, terminal-friendly theme.
#[must_use]
pub const fn knect_dark() -> Theme {
    Theme {
        accent: Color::Cyan,
        border: Color::DarkGray,
        focus: Color::LightCyan,
        muted: Color::Gray,
        text: Color::White,
        selection_bg: Color::DarkGray,
        selection_fg: Color::White,
        healthy: Color::Green,
        degraded: Color::Yellow,
        progressing: Color::LightCyan,
        blocked: Color::LightRed,
        unknown: Color::Gray,
        eventing: Color::Magenta,
        warning: Color::Yellow,
        error: Color::LightRed,
    }
}

impl Theme {
    /// Colour for a health state.
    #[must_use]
    pub const fn health(&self, health: Health) -> Color {
        match health {
            Health::Healthy => self.healthy,
            Health::Degraded => self.degraded,
            Health::Progressing => self.progressing,
            Health::Blocked => self.blocked,
            Health::Unknown => self.unknown,
        }
    }

    /// Style for a focused pane border.
    #[must_use]
    pub fn pane_border(&self, focused: bool) -> Style {
        Style::default().fg(if focused { self.focus } else { self.border })
    }

    /// Style for the selected row.
    #[must_use]
    pub fn selection(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .fg(self.selection_fg)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for group headers.
    #[must_use]
    pub fn group(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_is_deterministic_and_health_is_distinct() {
        assert_eq!(knect_dark(), knect_dark());
        let theme = knect_dark();
        assert_ne!(theme.health(Health::Healthy), theme.health(Health::Blocked));
        assert_ne!(
            theme.health(Health::Degraded),
            theme.health(Health::Unknown)
        );
    }
}
