//! Dashboard rendering entry point.

pub mod graph;
pub mod help;
pub mod inspector;
pub mod layout;
pub mod logs;
pub mod theme;
pub mod tree;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::state::{AppState, InputMode};
use crate::ui::theme::knect_dark;

/// Renders one frame.
pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
    if layout::too_small(area) {
        render_too_small(frame, area);
        return;
    }
    let regions = layout::dashboard(area);
    render_header(frame, regions.header, state);
    tree::render(frame, regions.tree, state);
    graph::render(frame, regions.graph, state);
    inspector::render(frame, regions.inspector, state);
    logs::render(frame, regions.logs, state);
    render_status(frame, regions.status, state);
    if state.help_visible {
        help::render(frame, area);
    }
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect) {
    let palette = knect_dark();
    let vertical = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Fill(1),
    ])
    .split(area);
    let block = Block::bordered()
        .title("knect")
        .border_style(Style::default().fg(palette.warning));
    frame.render_widget(
        Paragraph::new(format!(
            "terminal too small — need at least {}x{}",
            layout::MIN_WIDTH,
            layout::MIN_HEIGHT
        ))
        .style(Style::default().fg(palette.warning))
        .block(block),
        vertical[1],
    );
}

fn render_header(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let palette = knect_dark();
    let context = state
        .daemon
        .as_ref()
        .map(|status| status.context.clone())
        .or_else(|| {
            state
                .topology
                .as_ref()
                .map(|topology| topology.context.clone())
        })
        .unwrap_or_else(|| "no context".to_owned());
    let generation = state
        .topology
        .as_ref()
        .map(|topology| topology.generation)
        .or_else(|| state.daemon.as_ref().map(|status| status.generation))
        .unwrap_or(0);
    let endpoints = state.endpoint.as_deref().unwrap_or("unresolved");
    let connection = &state.connection;
    let connection_color = if connection.is_connected() {
        palette.healthy
    } else {
        palette.warning
    };
    let watchers = state
        .daemon
        .as_ref()
        .map(|status| {
            let total = status.watchers.len();
            let healthy = status
                .watchers
                .iter()
                .filter(|watcher| watcher.healthy)
                .count();
            format!("{healthy}/{total} watchers")
        })
        .unwrap_or_else(|| "watchers --".to_owned());
    let actions = state.daemon.as_ref().map_or("actions ?", |status| {
        if status.actions_enabled {
            "actions ON"
        } else {
            "actions off"
        }
    });
    let line = Line::from(vec![
        Span::styled(
            "knect",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {context}  gen {generation}  "),
            Style::default().fg(palette.text),
        ),
        Span::styled(endpoints.to_owned(), Style::default().fg(palette.muted)),
        Span::styled("  ● ", Style::default().fg(connection_color)),
        Span::styled(
            connection.label(),
            Style::default()
                .fg(connection_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {watchers}  {actions}"),
            Style::default().fg(palette.muted),
        ),
    ]);
    let block = Block::bordered()
        .title(Line::from(Span::styled(
            "knect dashboard",
            Style::default().fg(palette.accent),
        )))
        .border_style(Style::default().fg(palette.border));
    frame.render_widget(Paragraph::new(line).block(block), area);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let palette = knect_dark();
    let mut spans = vec![Span::styled(
        format!(" {} ", state.active_pane.label()),
        Style::default()
            .bg(palette.selection_bg)
            .fg(palette.selection_fg)
            .add_modifier(Modifier::BOLD),
    )];
    if state.input_mode == InputMode::Filter {
        spans.push(Span::styled(
            " filter> ",
            Style::default().fg(palette.accent),
        ));
        spans.push(Span::styled(
            state.filter.clone(),
            Style::default().fg(palette.text),
        ));
        spans.push(Span::styled(
            "  Enter apply · Esc cancel",
            Style::default().fg(palette.muted),
        ));
        let cursor_x = area.x
            + 1
            + u16::try_from(state.active_pane.label().len() + 2).unwrap_or(0)
            + u16::try_from(state.filter.width()).unwrap_or(0)
            + 9;
        frame.set_cursor_position((cursor_x.min(area.right().saturating_sub(1)), area.y));
    } else if let Some(error) = &state.error_banner {
        spans.push(Span::styled(
            format!(" {error}"),
            Style::default().fg(palette.error),
        ));
    } else if let Some(status) = &state.status {
        spans.push(Span::styled(
            format!(" {status}"),
            Style::default().fg(palette.warning),
        ));
    } else {
        spans.push(Span::styled(
            " hjkl move · / filter · gd details · Tab pane · 1-4 tabs · ? help · q quit",
            Style::default().fg(palette.muted),
        ));
    }
    if state.tap_visible {
        spans.push(Span::styled(
            "  tap:pane",
            Style::default().fg(palette.eventing),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn tiny_terminal_renders_a_hint_instead_of_panicking() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let state = AppState::new();
        terminal.draw(|frame| render(frame, &state)).expect("draw");
        let content = buffer_text(terminal.backend().buffer());
        assert!(content.contains("terminal too small"));
    }

    #[test]
    fn full_layout_renders_every_pane_title() {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let state = AppState::new();
        terminal.draw(|frame| render(frame, &state)).expect("draw");
        let content = buffer_text(terminal.backend().buffer());
        for title in ["Resources", "Topology", "Inspector", "Logs"] {
            assert!(content.contains(title), "missing pane {title}");
        }
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }
}
