//! Bottom log multiplexer and CloudEvent tap pane.

use std::collections::BTreeSet;

use knect_proto::tap::{LogLine, LogStream, TapEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};

use crate::app::state::AppState;
use crate::ui::theme;

/// Renders the log pane, splitting it with the tap pane when visible.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if state.tap_visible {
        let chunks = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);
        render_logs(frame, chunks[0], state);
        render_tap(frame, chunks[1], state);
    } else {
        render_logs(frame, area, state);
    }
}

fn render_logs(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let palette = theme::knect_dark();
    let focused = state.active_pane == crate::app::state::Pane::Logs;
    let labels = container_labels(&state.logs);
    let mut title = vec![Span::styled(
        format!("Logs ({})", state.logs.len()),
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD),
    )];
    if !labels.is_empty() {
        title.push(Span::styled(
            format!("  {}", labels.join(" · ")),
            Style::default().fg(palette.eventing),
        ));
    }
    if !state.log_filter.is_empty() {
        title.push(Span::styled(
            format!("  filter: {}", state.log_filter),
            Style::default().fg(palette.degraded),
        ));
    }
    let block = Block::bordered()
        .title(Line::from(title))
        .border_style(palette.pane_border(focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let filtered = filtered_logs(&state.logs, &state.log_filter);
    if filtered.is_empty() {
        let message = if state.logs.is_empty() {
            "no log lines yet — select a Revision or Pod to subscribe"
        } else {
            "no log lines match the filter"
        };
        frame.render_widget(
            Paragraph::new(message).style(Style::default().fg(palette.muted)),
            inner,
        );
        return;
    }
    let height = usize::from(inner.height);
    let offset = state.logs_offset.min(filtered.len());
    let end = filtered.len().saturating_sub(offset);
    let start = end.saturating_sub(height);
    let lines: Vec<Line<'static>> = filtered[start..end]
        .iter()
        .map(|line| log_line(line, palette))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn render_tap(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let palette = theme::knect_dark();
    let block = Block::bordered()
        .title(Line::from(Span::styled(
            format!("Tap ({})", state.tap_events.len()),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .border_style(palette.pane_border(false));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if state.tap_events.is_empty() {
        frame.render_widget(
            Paragraph::new("no tap events yet").style(Style::default().fg(palette.muted)),
            inner,
        );
        return;
    }
    let height = usize::from(inner.height);
    let start = state.tap_events.len().saturating_sub(height);
    let lines: Vec<Line<'static>> = state.tap_events[start..]
        .iter()
        .map(|event| tap_line(event, palette))
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn log_line(line: &LogLine, palette: theme::Theme) -> Line<'static> {
    let container_color = container_color(&line.container);
    let message_color = match line.stream {
        LogStream::Stdout => palette.text,
        LogStream::Stderr => palette.warning,
    };
    Line::from(vec![
        Span::styled(
            format!("{} ", line.timestamp.format("%H:%M:%S")),
            Style::default().fg(palette.muted),
        ),
        Span::styled(
            format!("{} ", line.container),
            Style::default().fg(container_color),
        ),
        Span::styled(line.message.clone(), Style::default().fg(message_color)),
    ])
}

fn tap_line(event: &TapEvent, palette: theme::Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{} ", event.ce_type),
            Style::default().fg(palette.accent),
        ),
        Span::styled(
            format!("{} ", event.ce_source),
            Style::default().fg(palette.eventing),
        ),
        Span::styled(
            compact(&event.payload.to_string(), 80),
            Style::default().fg(palette.text),
        ),
    ])
}

fn compact(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    let mut output: String = value.chars().take(limit).collect();
    output.push('…');
    output
}

fn container_labels(logs: &[LogLine]) -> Vec<String> {
    let labels: BTreeSet<String> = logs
        .iter()
        .map(|line| format!("{}:{}", line.pod, line.container))
        .collect();
    labels.into_iter().take(3).collect()
}

fn container_color(container: &str) -> Color {
    let palette = [
        Color::Cyan,
        Color::Green,
        Color::Magenta,
        Color::Yellow,
        Color::Blue,
    ];
    let index = container.bytes().fold(0_usize, |accumulator, byte| {
        accumulator.wrapping_add(usize::from(byte))
    }) % palette.len();
    palette[index]
}

/// Applies the case-insensitive log filter.
#[must_use]
pub fn filtered_logs<'a>(logs: &'a [LogLine], filter: &str) -> Vec<&'a LogLine> {
    let needle = filter.trim().to_lowercase();
    logs.iter()
        .filter(|line| {
            needle.is_empty()
                || line.message.to_lowercase().contains(&needle)
                || line.container.to_lowercase().contains(&needle)
                || line.pod.to_lowercase().contains(&needle)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn line(container: &str, message: &str) -> LogLine {
        LogLine {
            namespace: "default".to_owned(),
            pod: "pod-a".to_owned(),
            container: container.to_owned(),
            timestamp: Utc::now(),
            message: message.to_owned(),
            stream: LogStream::Stdout,
        }
    }

    #[test]
    fn filter_matches_message_container_and_pod() {
        let logs = vec![
            line("user-container", "listening on :8080"),
            line("queue", "lag 4"),
        ];
        assert_eq!(filtered_logs(&logs, "LISTENING").len(), 1);
        assert_eq!(filtered_logs(&logs, "queue").len(), 1);
        assert_eq!(filtered_logs(&logs, "pod-a").len(), 2);
        assert_eq!(filtered_logs(&logs, "").len(), 2);
        assert!(filtered_logs(&logs, "missing").is_empty());
    }

    #[test]
    fn container_labels_are_sorted_and_unique() {
        let logs = vec![line("b", "one"), line("a", "two"), line("a", "three")];
        assert_eq!(container_labels(&logs), vec!["pod-a:a", "pod-a:b"]);
    }

    #[test]
    fn compact_respects_character_boundaries() {
        assert_eq!(compact("abc", 5), "abc");
        assert_eq!(compact("abcdef", 3), "abc…");
        assert_eq!(compact("한글", 1), "한…");
    }
}
