//! Frame layout helpers.

use std::rc::Rc;

use ratatui::layout::{Constraint, Layout, Rect};

/// Splits the frame into header, body, and status areas.
#[must_use]
pub fn main_layout(area: Rect) -> Rc<[Rect]> {
    Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area)
}
