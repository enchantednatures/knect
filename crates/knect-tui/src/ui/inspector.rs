//! Inspector pane with Mutations, Policies, Links, and Metrics tabs.

use knect_proto::graph::{Health, NodeDetails};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};

use crate::app::state::{AppState, InspectorTab};
use crate::ui::{graph, theme};

/// Renders the inspector pane.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let palette = theme::knect_dark();
    let focused = state.active_pane == crate::app::state::Pane::Inspector;
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled(
            "Inspector",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]))
        .border_style(palette.pane_border(focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = detail_lines_for_width(state, usize::from(inner.width));
    let offset = (state.inspector_offset as u16).min(lines.len().saturating_sub(1) as u16);
    frame.render_widget(Paragraph::new(Text::from(lines)).scroll((offset, 0)), inner);
}

/// Builds the inspector's text for the current selection and tab.
#[must_use]
pub fn detail_lines(state: &AppState) -> Vec<Line<'static>> {
    detail_lines_for_width(state, 80)
}

fn detail_lines_for_width(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let palette = theme::knect_dark();
    let mut lines = tabs_lines(state.inspector_tab, palette, width);
    if state.selected.is_none() {
        lines.push(Line::from(Span::styled(
            "Select a resource to inspect its mutations, policies, links, and metrics.",
            Style::default().fg(palette.muted),
        )));
        return lines;
    }
    if let Some(error) = &state.details_error {
        lines.push(Line::from(Span::styled(
            format!("details unavailable: {error}"),
            Style::default().fg(palette.error),
        )));
    }
    let Some(details) = state.details.as_ref() else {
        lines.push(Line::from(Span::styled(
            "loading details…",
            Style::default().fg(palette.muted),
        )));
        return lines;
    };
    header_lines(details, palette, &mut lines);
    match state.inspector_tab {
        InspectorTab::Mutations => mutations_lines(details, palette, &mut lines),
        InspectorTab::Policies => policies_lines(details, palette, &mut lines),
        InspectorTab::Links => links_lines(details, palette, &mut lines),
        InspectorTab::Metrics => metrics_lines(details, palette, &mut lines),
    }
    lines
}

fn tabs_lines(active: InspectorTab, palette: theme::Theme, width: usize) -> Vec<Line<'static>> {
    let per_line = if width >= 44 {
        4
    } else if width >= 22 {
        2
    } else {
        1
    };
    InspectorTab::ALL
        .chunks(per_line)
        .map(|tabs| {
            let mut spans = Vec::new();
            for tab in tabs {
                let style = if *tab == active {
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                } else {
                    Style::default().fg(palette.muted)
                };
                spans.push(Span::styled(format!("[{}]", tab.label()), style));
                spans.push(Span::raw(" "));
            }
            Line::from(spans)
        })
        .collect()
}

fn header_lines(details: &NodeDetails, palette: theme::Theme, lines: &mut Vec<Line<'static>>) {
    lines.push(Line::from(vec![
        Span::styled(
            details.name.clone(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  {}/{}",
                graph::short_category(details.category),
                details.namespace.as_deref().unwrap_or("cluster")
            ),
            Style::default().fg(palette.muted),
        ),
        Span::styled(
            format!("  {}", health_label(details.health)),
            Style::default().fg(palette.health(details.health)),
        ),
    ]));
    if !details.summary.is_empty() {
        lines.push(Line::from(Span::styled(
            details.summary.clone(),
            Style::default().fg(palette.muted),
        )));
    }
    lines.push(Line::from(""));
}

fn mutations_lines(details: &NodeDetails, palette: theme::Theme, lines: &mut Vec<Line<'static>>) {
    if details.mutations.is_empty() {
        lines.push(empty("no mutation data yet"));
    } else {
        for record in &details.mutations {
            lines.push(Line::from(vec![
                Span::styled(record.manager.clone(), Style::default().fg(palette.accent)),
                Span::styled(
                    format!("  {:?}/{:?}", record.manager_kind, record.operation),
                    Style::default().fg(palette.muted),
                ),
            ]));
            for path in &record.paths {
                let preview = path
                    .value_preview
                    .as_deref()
                    .map(|value| format!(" = {value}"))
                    .unwrap_or_default();
                lines.push(Line::from(Span::styled(
                    format!("  {}{}", path.path, preview),
                    Style::default().fg(palette.text),
                )));
            }
        }
    }
    if let Some(yaml) = &details.yaml {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Manifest (redacted)",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )));
        for yaml_line in yaml.lines().take(120) {
            lines.push(Line::from(Span::styled(
                yaml_line.to_owned(),
                Style::default().fg(palette.text),
            )));
        }
    }
}

fn policies_lines(details: &NodeDetails, palette: theme::Theme, lines: &mut Vec<Line<'static>>) {
    if details.policies.is_empty() {
        lines.push(empty("no policy data yet"));
        return;
    }
    for policy in &details.policies {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:?}", policy.severity),
                Style::default().fg(palette.warning),
            ),
            Span::styled(
                format!("  {:?}  {}", policy.result, policy.name),
                Style::default().fg(palette.text),
            ),
        ]));
        if !policy.message.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  {}", policy.message),
                Style::default().fg(palette.muted),
            )));
        }
        for (key, value) in &policy.details {
            lines.push(Line::from(Span::styled(
                format!("  {key}: {value}"),
                Style::default().fg(palette.muted),
            )));
        }
    }
}

fn links_lines(details: &NodeDetails, palette: theme::Theme, lines: &mut Vec<Line<'static>>) {
    if details.links.is_empty() {
        lines.push(empty("no link data yet"));
        return;
    }
    for link in &details.links {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} → {}", link.source.name, link.target.name),
                Style::default().fg(palette.text),
            ),
            Span::styled(
                format!("  {:?} ({:.2})", link.via, link.confidence),
                Style::default().fg(palette.eventing),
            ),
        ]));
        for evidence in &link.evidence {
            lines.push(Line::from(Span::styled(
                format!("  {:?}: {}", evidence.kind, evidence.description),
                Style::default().fg(palette.muted),
            )));
        }
    }
}

fn metrics_lines(details: &NodeDetails, palette: theme::Theme, lines: &mut Vec<Line<'static>>) {
    let Some(metrics) = details.metrics.as_ref() else {
        lines.push(empty("no metrics data yet"));
        return;
    };
    let rows = [
        (
            "cpu_millicores",
            metrics.cpu_millicores.map(|value| format!("{value:.1} m")),
        ),
        (
            "memory_bytes",
            metrics.memory_bytes.map(|value| format!("{value} B")),
        ),
        ("replicas", metrics.replicas.map(|value| value.to_string())),
        (
            "ready_replicas",
            metrics.ready_replicas.map(|value| value.to_string()),
        ),
        (
            "requests_per_second",
            metrics
                .requests_per_second
                .map(|value| format!("{value:.2}")),
        ),
        (
            "error_rate",
            metrics.error_rate.map(|value| format!("{value:.4}")),
        ),
        (
            "p95_latency_ms",
            metrics.p95_latency_ms.map(|value| format!("{value:.1} ms")),
        ),
        (
            "window_secs",
            metrics.window_secs.map(|value| value.to_string()),
        ),
        ("source", metrics.source.clone()),
    ];
    for (label, value) in rows {
        if let Some(value) = value {
            lines.push(Line::from(vec![
                Span::styled(format!("{label:<20}"), Style::default().fg(palette.muted)),
                Span::styled(value, Style::default().fg(palette.text)),
            ]));
        }
    }
    if details.storage.is_empty() {
        lines.push(empty("no storage data yet"));
    } else {
        for storage in &details.storage {
            lines.push(Line::from(Span::styled(
                format!(
                    "pvc {}  {}  {}",
                    storage.pvc.name,
                    storage.phase.as_deref().unwrap_or("unknown"),
                    storage.capacity.as_deref().unwrap_or("-")
                ),
                Style::default().fg(palette.text),
            )));
            for warning in &storage.warnings {
                lines.push(Line::from(Span::styled(
                    format!("  {:?}: {}", warning.severity, warning.message),
                    Style::default().fg(palette.warning),
                )));
            }
        }
    }
}

fn empty(message: &str) -> Line<'static> {
    Line::from(Span::styled(
        message.to_owned(),
        Style::default().fg(theme::knect_dark().muted),
    ))
}

fn health_label(health: Health) -> &'static str {
    match health {
        Health::Healthy => "healthy",
        Health::Degraded => "degraded",
        Health::Progressing => "progressing",
        Health::Blocked => "blocked",
        Health::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_are_stable_and_labelled() {
        assert_eq!(InspectorTab::ALL.len(), 4);
        assert_eq!(InspectorTab::Mutations.label(), "Mutations");
        assert_eq!(InspectorTab::Policies.label(), "Policies");
        assert_eq!(InspectorTab::Links.label(), "Links");
        assert_eq!(InspectorTab::Metrics.label(), "Metrics");
    }

    #[test]
    fn no_selection_explains_the_pane() {
        let state = AppState::new();
        let text = render_text(detail_lines(&state));
        assert!(text.contains("Select a resource"));
        assert!(text.contains("[Mutations]"));
    }

    #[test]
    fn missing_details_show_a_loading_state() {
        let mut state = AppState::new();
        state.selected = Some(knect_proto::ResourceId::namespaced(
            "serving.knative.dev/v1",
            "Service",
            "default",
            "hello",
        ));
        assert!(render_text(detail_lines(&state)).contains("loading details"));
    }

    fn render_text(lines: Vec<Line<'static>>) -> String {
        lines
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
