//! Placeholder UI rendering.
//!
//! Task 13 replaces these modules with the four-pane product. The module names
//! and the `render` entry point are frozen.

pub mod graph;
pub mod help;
pub mod inspector;
pub mod layout;
pub mod logs;
pub mod theme;
pub mod tree;

use ratatui::Frame;
use ratatui::style::Style;
use ratatui::widgets::{Block, Paragraph};

use crate::app::state::AppState;

/// Renders one frame.
pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let areas = layout::main_layout(frame.area());
    let theme = theme::knect_dark();
    let header = Paragraph::new("knect")
        .style(Style::default().fg(theme.accent))
        .block(
            Block::bordered()
                .title("knect")
                .border_style(Style::default().fg(theme.border)),
        );
    frame.render_widget(header, areas[0]);
    graph::render(frame, areas[1]);
    let status = Paragraph::new(format!(
        "TUI implementation pending | screen: {:?} | q quit",
        state.screen
    ))
    .style(Style::default().fg(theme.muted));
    frame.render_widget(status, areas[2]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn placeholder_renders_a_knect_frame() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let state = AppState::new();
        terminal.draw(|frame| render(frame, &state)).expect("draw");
        let buffer = terminal.backend().buffer();
        let content: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(content.contains("knect"));
        assert!(content.contains("TUI implementation pending"));
        assert!(content.contains("Graph"));
    }
}
