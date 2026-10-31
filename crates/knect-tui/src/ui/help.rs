//! Help overlay placeholder.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Paragraph};

use crate::ui::theme;

/// Renders the help placeholder.
pub fn render(frame: &mut Frame<'_>, area: Rect) {
    let theme = theme::knect_dark();
    let block = Block::bordered()
        .title("Help")
        .border_style(Style::default().fg(theme.border));
    frame.render_widget(
        Paragraph::new("q quit | r refresh | 1 graph | 2 diff | 3 tap | ? help").block(block),
        area,
    );
}
