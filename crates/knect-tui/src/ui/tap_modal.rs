//! Live CloudEvent tap modal with a JSON scratch pane.

use knect_proto::tap::{TapState, TapTargetKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::modal::TapModal;
use crate::ui::modal::modal_block;
use crate::ui::theme;

/// Renders the tap event stream.
pub fn render(frame: &mut Frame<'_>, area: Rect, modal: &TapModal) {
    let kind = match modal.target.kind {
        TapTargetKind::Broker => "Broker",
        TapTargetKind::Trigger => "Trigger",
        TapTargetKind::Unknown => "target",
    };
    let state = modal
        .tap
        .as_ref()
        .map(|handle| tap_state(handle.state))
        .unwrap_or_else(|| {
            if modal.starting {
                "starting"
            } else {
                "inactive"
            }
        });
    let title = format!(
        "Tap — {kind} {}/{} [{state}]  (j/k select · Esc close · {} events)",
        modal.target.namespace,
        modal.target.name,
        modal.events.len()
    );
    let block = modal_block(title, Color::Magenta);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let layout =
        Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).split(inner);
    render_list(frame, layout[0], modal);
    render_json(frame, layout[1], modal);
}

fn render_list(frame: &mut Frame<'_>, area: Rect, modal: &TapModal) {
    let palette = theme::knect_dark();
    if let Some(error) = &modal.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::default().fg(palette.error))
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    if modal.events.is_empty() {
        let message = if modal.starting {
            "starting tap…"
        } else {
            "waiting for events…"
        };
        frame.render_widget(
            Paragraph::new(message).style(Style::default().fg(palette.muted)),
            area,
        );
        return;
    }
    let mut lines = Vec::with_capacity(modal.events.len() * 2);
    for (index, event) in modal.events.iter().enumerate() {
        let selected = index == modal.selected;
        let style = if selected {
            Style::default()
                .fg(palette.selection_fg)
                .bg(palette.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.text)
        };
        lines.push(Line::from(vec![
            Span::styled(
                event.ce_type.clone(),
                style.fg(if selected {
                    palette.selection_fg
                } else {
                    palette.accent
                }),
            ),
            Span::styled(
                format!(
                    "  {}",
                    event
                        .ce_time
                        .map(|time| time.format("%H:%M:%S").to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ),
                style.fg(palette.muted),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {}", event.ce_id),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_json(frame: &mut Frame<'_>, area: Rect, modal: &TapModal) {
    let palette = theme::knect_dark();
    let Some(event) = modal.selected_event() else {
        frame.render_widget(
            Paragraph::new("select an event to inspect its CloudEvent JSON")
                .style(Style::default().fg(palette.muted)),
            area,
        );
        return;
    };
    let payload = serde_json::json!({
        "id": &event.ce_id,
        "type": &event.ce_type,
        "source": &event.ce_source,
        "subject": &event.ce_subject,
        "time": event.ce_time.map(|time| time.to_rfc3339()),
        "received_at": event.received_at.to_rfc3339(),
        "headers": &event.headers,
        "data": &event.payload,
    });
    let pretty = serde_json::to_string_pretty(&payload)
        .unwrap_or_else(|error| format!("cannot render payload: {error}"));
    let mut lines: Vec<Line<'static>> = vec![Line::from(vec![
        Span::styled(
            format!("{} ", event.ce_id),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            event
                .ce_time
                .map(|time| time.to_rfc3339())
                .unwrap_or_else(|| "-".to_owned()),
            Style::default().fg(palette.muted),
        ),
    ])];
    lines.extend(pretty.lines().map(|line| {
        Line::from(Span::styled(
            line.to_owned(),
            Style::default().fg(palette.text),
        ))
    }));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).scroll((modal.scroll as u16, 0)),
        area,
    );
}

fn tap_state(state: TapState) -> &'static str {
    match state {
        TapState::Starting => "starting",
        TapState::Active => "active",
        TapState::Stopping => "stopping",
        TapState::Stopped => "stopped",
        TapState::Failed => "failed",
        TapState::Expired => "expired",
        TapState::Unknown => "unknown",
    }
}
