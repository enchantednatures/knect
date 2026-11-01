//! Interactive traffic-split editor modal.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;

use crate::app::modal::{TrafficEdit, TrafficModal};
use crate::ui::modal::modal_block;
use crate::ui::theme;

/// Renders the traffic editor.
pub fn render(frame: &mut Frame<'_>, area: Rect, modal: &TrafficModal) {
    let title = format!(
        "Traffic split — {} {}  (j/k select · +/- 5% · digits % · t tag · p dry-run · s apply · Esc close)",
        modal
            .service
            .namespace
            .as_deref()
            .unwrap_or("cluster-scoped"),
        modal.service.name
    );
    let block = modal_block(title, Color::Cyan);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = header_lines(modal);
    lines.push(Line::from(""));
    for (index, row) in modal.rows.iter().enumerate() {
        lines.push(row_line(modal, index, row));
    }
    if modal.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "no revisions to edit",
            crate::ui::modal::muted_style(),
        )));
    }
    lines.push(Line::from(""));
    lines.push(sum_line(modal));
    if let Some(error) = &modal.error {
        lines.push(Line::from(Span::styled(
            error.clone(),
            Style::default().fg(Color::LightRed),
        )));
    }
    if let Some(result) = &modal.result {
        lines.push(Line::from(Span::styled(
            result.clone(),
            Style::default().fg(Color::Green),
        )));
    }
    if modal.confirm_pending {
        lines.push(Line::from(Span::styled(
            "press s again to apply this split",
            Style::default().fg(Color::LightYellow),
        )));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn header_lines(modal: &TrafficModal) -> Vec<Line<'static>> {
    let palette = theme::knect_dark();
    let route = modal
        .route
        .as_ref()
        .map(|route| {
            format!(
                "Route {}/{}",
                route.namespace.as_deref().unwrap_or("-"),
                route.name
            )
        })
        .unwrap_or_else(|| "no Route discovered".to_owned());
    let mut lines = vec![
        Line::from(Span::styled(route, Style::default().fg(palette.muted))),
        Line::from(vec![
            Span::styled("revision", Style::default().fg(palette.accent)),
            Span::styled(
                "          percent   tag",
                Style::default().fg(palette.muted),
            ),
        ]),
    ];
    if modal.loading {
        lines.push(Line::from(Span::styled(
            "action in flight…",
            Style::default().fg(palette.warning),
        )));
    }
    lines
}

fn row_line(
    modal: &TrafficModal,
    index: usize,
    row: &crate::app::modal::TrafficRow,
) -> Line<'static> {
    let palette = theme::knect_dark();
    let selected = index == modal.selected;
    let cursor = if selected { "▶ " } else { "  " };
    let percent = match &modal.edit {
        Some(TrafficEdit::Percent(buffer)) if selected => format!("{buffer:>3}"),
        _ => format!("{:>3}", row.percent),
    };
    let tag = match &modal.edit {
        Some(TrafficEdit::Tag(buffer)) if selected => format!("{buffer}▏"),
        _ => row.tag.clone(),
    };
    let marker = if selected && modal.edit.is_some() {
        "  edit"
    } else {
        ""
    };
    let mut spans = vec![
        Span::styled(
            format!("{cursor}{:<24}", row.revision),
            if selected {
                Style::default()
                    .fg(palette.selection_fg)
                    .bg(palette.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            },
        ),
        Span::styled(
            format!("{percent:>8}%  "),
            if selected {
                Style::default()
                    .fg(palette.selection_fg)
                    .bg(palette.selection_bg)
            } else {
                Style::default().fg(palette.healthy)
            },
        ),
        Span::styled(tag, Style::default().fg(palette.eventing)),
    ];
    if !marker.is_empty() {
        spans.push(Span::styled(
            marker.to_owned(),
            Style::default().fg(palette.warning),
        ));
    }
    Line::from(spans)
}

fn sum_line(modal: &TrafficModal) -> Line<'static> {
    let palette = theme::knect_dark();
    let total = modal.sum();
    let (label, color) = match modal.validate() {
        Ok(()) => (format!("total {total}% — valid"), palette.healthy),
        Err(error) => (format!("total {total}% — {error}"), palette.error),
    };
    Line::from(Span::styled(
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}
