//! Modal overlays for mutation diffs, traffic splits, and CloudEvent taps.

use knect_proto::mutation::MutationSource;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Clear};

use crate::app::modal::Modal;
use crate::app::state::AppState;
use crate::ui::theme;

/// Renders the active modal, if any.
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if matches!(state.modal, Modal::None) {
        return;
    }
    let area = overlay(area);
    frame.render_widget(Clear, area);
    match &state.modal {
        Modal::None => {}
        Modal::Diff(modal) => crate::ui::diff_modal::render(frame, area, modal),
        Modal::Traffic(modal) => crate::ui::traffic_modal::render(frame, area, modal),
        Modal::Tap(modal) => crate::ui::tap_modal::render(frame, area, modal),
    }
}

/// Centred overlay rect with a small terminal margin.
#[must_use]
pub fn overlay(area: Rect) -> Rect {
    let width = (u32::from(area.width) * 94 / 100)
        .max(40)
        .min(u32::from(area.width));
    let height = (u32::from(area.height) * 86 / 100)
        .max(12)
        .min(u32::from(area.height));
    let width = u16::try_from(width).unwrap_or(area.width);
    let height = u16::try_from(height).unwrap_or(area.height);
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
    horizontal[1]
}

/// Label and accent colour for a diff pane.
#[must_use]
pub const fn source_style(source: MutationSource) -> (&'static str, Color) {
    match source {
        MutationSource::UserIntent => ("User intent", Color::Cyan),
        MutationSource::KnativeDefaults => ("Knative defaults", Color::Magenta),
        MutationSource::PolicyMutators => ("Policy mutators", Color::Yellow),
        MutationSource::Unknown => ("Unknown", Color::Gray),
    }
}

/// Colour for a diff line owner.
#[must_use]
pub fn owner_color(owner: &str) -> Color {
    let owner = owner.to_ascii_lowercase();
    if owner.contains("kyverno") {
        Color::Yellow
    } else if owner.contains("istio") {
        Color::Green
    } else if owner.contains("knative") {
        Color::Magenta
    } else if owner.contains("flagger") {
        Color::LightYellow
    } else if owner.contains("helm") {
        Color::LightCyan
    } else if owner.contains("kubectl") {
        Color::Cyan
    } else {
        Color::Gray
    }
}

/// Shared bordered modal block.
#[must_use]
pub fn modal_block(title: String, accent: Color) -> Block<'static> {
    let palette = theme::knect_dark();
    Block::bordered()
        .title(ratatui::text::Line::from(ratatui::text::Span::styled(
            title,
            Style::default().fg(accent),
        )))
        .border_style(Style::default().fg(palette.focus))
}

/// Diagnostics for a failed diff pane.
#[must_use]
pub fn muted_style() -> Style {
    Style::default().fg(Color::Gray)
}
