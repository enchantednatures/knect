//! Placeholder Logs pane.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Paragraph};

use crate::ui::theme;

/// Renders the placeholder pane.
pub fn render(frame: &mut Frame<'_>, area: Rect) {
    let theme = theme::knect_dark();
    let block = Block::bordered()
        .title("Logs")
        .border_style(Style::default().fg(theme.border));
    frame.render_widget(Paragraph::new("pending").block(block), area);
}
