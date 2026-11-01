//! Help overlay.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::ui::theme;

const HELP_ROWS: &[(&str, &str)] = &[
    ("j / k", "move selection or scroll"),
    ("h / l", "pan the graph or switch tabs"),
    ("g / G", "jump to top or bottom"),
    ("gd", "focus the inspector details"),
    ("/", "filter the resource tree"),
    ("Tab", "cycle panes (Shift-Tab reverses)"),
    (
        "1..4",
        "inspector tabs: Mutations, Policies, Links, Metrics",
    ),
    ("r", "refresh topology from the daemon"),
    ("?", "toggle this help"),
    ("Esc", "close overlays and prompts"),
    ("q", "quit"),
    (
        "d / t / w",
        "mutation diff · traffic split · CloudEvent tap",
    ),
];

/// Renders the help overlay centred over the dashboard.
pub fn render(frame: &mut Frame<'_>, area: Rect) {
    let palette = theme::knect_dark();
    let width = area.width.saturating_sub(4).clamp(40, 72);
    let height = (HELP_ROWS.len() as u16 + 4).min(area.height.saturating_sub(2));
    let vertical = Layout::vertical([
        Constraint::Length(area.height.saturating_sub(height) / 2),
        Constraint::Length(height),
        Constraint::Min(0),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Length(area.width.saturating_sub(width) / 2),
        Constraint::Length(width),
        Constraint::Min(0),
    ])
    .split(vertical[1]);
    let popup = horizontal[1];
    frame.render_widget(Clear, popup);
    let lines: Vec<Line<'_>> = HELP_ROWS
        .iter()
        .map(|(keys, description)| {
            Line::from(vec![
                Span::styled(
                    format!("{keys:<12}"),
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(*description, Style::default().fg(palette.text)),
            ])
        })
        .collect();
    let block = Block::bordered()
        .title(Line::from(Span::styled(
            "knect help",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .border_style(Style::default().fg(palette.focus));
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

/// Number of documented shortcuts, used by tests.
#[must_use]
pub const fn shortcut_count() -> usize {
    HELP_ROWS.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_documents_required_keys() {
        let text: String = HELP_ROWS
            .iter()
            .map(|(keys, description)| format!("{keys} {description}"))
            .collect();
        for required in [
            "j / k",
            "h / l",
            "gd",
            "/",
            "Tab",
            "1..4",
            "?",
            "Esc",
            "q",
            "d / t / w",
        ] {
            assert!(text.contains(required), "help is missing {required}");
        }
        assert_eq!(shortcut_count(), HELP_ROWS.len());
    }
}
