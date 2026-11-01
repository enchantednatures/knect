//! Three-pane mutation diff modal.

use knect_proto::mutation::MutationSource;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::modal::{DiffModal, SOURCES, pane_document, pane_lines};
use crate::ui::modal::{modal_block, owner_color, source_style};

/// Renders the three-way diff.
pub fn render(frame: &mut Frame<'_>, area: Rect, modal: &DiffModal) {
    let title = format!(
        "Mutation diff — {} {}/{}  (r refresh · j/k scroll · Esc close)",
        modal.resource.kind,
        modal
            .resource
            .namespace
            .as_deref()
            .unwrap_or("cluster-scoped"),
        modal.resource.name
    );
    let block = modal_block(title, ratatui::style::Color::Cyan);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let panes = Layout::horizontal([
        Constraint::Percentage(34),
        Constraint::Percentage(33),
        Constraint::Percentage(33),
    ])
    .split(inner);
    for (index, source) in SOURCES.iter().enumerate() {
        render_pane(frame, panes[index], modal, *source);
    }
}

fn render_pane(frame: &mut Frame<'_>, area: Rect, modal: &DiffModal, source: MutationSource) {
    let (label, color) = source_style(source);
    let count = modal
        .diff
        .as_ref()
        .map(|diff| pane_lines(diff, source).len())
        .unwrap_or(0);
    let block = modal_block(format!("{label} ({count})"), color);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if let Some(error) = &modal.error {
        frame.render_widget(
            Paragraph::new(error.clone())
                .style(Style::default().fg(ratatui::style::Color::LightRed))
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let Some(diff) = modal.diff.as_ref() else {
        if modal.loading {
            frame.render_widget(
                Paragraph::new("loading mutation attribution…")
                    .style(crate::ui::modal::muted_style()),
                inner,
            );
        }
        return;
    };
    let lines = pane_lines(diff, source);
    let rendered: Vec<Line<'static>> = if lines.is_empty() {
        pane_document(diff, source)
            .map(|document| {
                document
                    .lines()
                    .enumerate()
                    .map(|(index, line)| document_line(index, line, color))
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![Line::from(Span::styled(
                    "no attribution lines",
                    crate::ui::modal::muted_style(),
                ))]
            })
    } else {
        lines.iter().map(|line| diff_line(line, color)).collect()
    };
    frame.render_widget(
        Paragraph::new(Text::from(rendered)).scroll((modal.scroll as u16, 0)),
        inner,
    );
}

fn document_line(index: usize, line: &str, color: ratatui::style::Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{:>4} ", index + 1),
            Style::default().fg(ratatui::style::Color::DarkGray),
        ),
        Span::styled(line.to_owned(), Style::default().fg(color)),
    ])
}

fn diff_line(
    line: &knect_proto::mutation::DiffLine,
    color: ratatui::style::Color,
) -> Line<'static> {
    let marker = if line.changed { "*" } else { " " };
    let mut text = Style::default().fg(color);
    if line.changed {
        text = text.add_modifier(Modifier::BOLD);
    }
    let owner = line.owner.clone().unwrap_or_default();
    let owner_style = if owner.is_empty() {
        crate::ui::modal::muted_style()
    } else {
        Style::default().fg(owner_color(&owner))
    };
    Line::from(vec![
        Span::styled(
            format!("{:>4} ", line.line_number),
            Style::default().fg(ratatui::style::Color::DarkGray),
        ),
        Span::styled(
            format!("{marker} "),
            Style::default().fg(ratatui::style::Color::LightYellow),
        ),
        Span::styled(owner, owner_style),
        Span::styled(format!("  {}", line.text), text),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::modal::Modal;
    use crate::app::state::AppState;
    use knect_proto::ResourceId;
    use knect_proto::mutation::{DiffLine, MutationDiff, MutationDiffMode, MutationSource};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn diff() -> MutationDiff {
        MutationDiff {
            resource: ResourceId::namespaced(
                "serving.knative.dev/v1",
                "Service",
                "default",
                "hello",
            ),
            user_intent: None,
            knative_defaults: None,
            policy_mutators: None,
            lines: vec![
                DiffLine {
                    line_number: 8,
                    side: MutationSource::UserIntent,
                    owner: Some("kubectl-client-side-apply".to_owned()),
                    text: "image: hello:1.0.0".to_owned(),
                    changed: false,
                    path: None,
                },
                DiffLine {
                    line_number: 12,
                    side: MutationSource::KnativeDefaults,
                    owner: Some("knative-serving-controller".to_owned()),
                    text: "cpu: 100m".to_owned(),
                    changed: true,
                    path: None,
                },
                DiffLine {
                    line_number: 6,
                    side: MutationSource::PolicyMutators,
                    owner: Some("istio-sidecar-injector".to_owned()),
                    text: "sidecar.istio.io/inject: \"true\"".to_owned(),
                    changed: true,
                    path: None,
                },
            ],
        }
    }

    #[test]
    fn renders_three_colour_coded_panes_with_owners_and_markers() {
        let mut state = AppState::new();
        state.modal = Modal::Diff(DiffModal {
            resource: diff().resource.clone(),
            mode: MutationDiffMode::ThreeWay,
            diff: Some(diff()),
            error: None,
            loading: false,
            scroll: 0,
        });
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::ui::render(frame, &state))
            .expect("draw");
        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        for expected in [
            "Mutation diff",
            "User intent",
            "Knative defaults",
            "Policy mutators",
            "kubectl-client-side-apply",
            "knative-serving-controller",
            "istio-sidecar-injector",
            "8",
            "12",
        ] {
            assert!(content.contains(expected), "missing {expected}:\n{content}");
        }
    }
}
