//! Four-pane dashboard geometry.

use ratatui::layout::{Constraint, Layout, Rect};

/// Minimum terminal width that keeps every pane legible.
pub const MIN_WIDTH: u16 = 60;
/// Minimum terminal height that keeps every pane legible.
pub const MIN_HEIGHT: u16 = 16;

/// Fixed regions of the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashboardLayout {
    /// Application header.
    pub header: Rect,
    /// Filterable resource tree.
    pub tree: Rect,
    /// Topology DAG.
    pub graph: Rect,
    /// Inspector tabs.
    pub inspector: Rect,
    /// Log multiplexer and optional tap pane.
    pub logs: Rect,
    /// Connection and key-hint status bar.
    pub status: Rect,
}

/// True when the terminal cannot show the four panes legibly.
#[must_use]
pub fn too_small(area: Rect) -> bool {
    area.width < MIN_WIDTH || area.height < MIN_HEIGHT
}

/// Splits a terminal into header, tree, graph, inspector, logs, and status.
#[must_use]
pub fn dashboard(area: Rect) -> DashboardLayout {
    let outer = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(1),
    ])
    .split(area);
    let rows = Layout::vertical([Constraint::Fill(1), Constraint::Percentage(30)]).split(outer[1]);
    let panes = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Fill(2),
        Constraint::Percentage(20),
    ])
    .split(rows[0]);
    DashboardLayout {
        header: outer[0],
        tree: panes[0],
        graph: panes[1],
        inspector: panes[2],
        logs: rows[1],
        status: outer[2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(left: Rect, right: Rect) -> bool {
        left.x < right.x.saturating_add(right.width)
            && right.x < left.x.saturating_add(left.width)
            && left.y < right.y.saturating_add(right.height)
            && right.y < left.y.saturating_add(left.height)
    }

    #[test]
    fn four_panes_do_not_overlap_at_eighty_by_twenty_four() {
        let layout = dashboard(Rect::new(0, 0, 80, 24));
        let regions = [
            layout.header,
            layout.tree,
            layout.graph,
            layout.inspector,
            layout.logs,
            layout.status,
        ];
        for (index, left) in regions.iter().enumerate() {
            assert!(left.width > 0 && left.height > 0);
            for right in regions.iter().skip(index + 1) {
                assert!(!overlaps(*left, *right), "{left:?} overlaps {right:?}");
            }
        }
        assert!(layout.graph.width > layout.tree.width);
        assert!(layout.graph.width > layout.inspector.width);
        assert_eq!(layout.header.y, 0);
        assert_eq!(layout.header.height, 3);
        assert_eq!(layout.status.height, 1);
        assert_eq!(layout.status.y + layout.status.height, 24);
    }

    #[test]
    fn layout_scales_to_a_wide_terminal() {
        let narrow = dashboard(Rect::new(0, 0, 80, 24));
        let wide = dashboard(Rect::new(0, 0, 160, 50));
        assert!(wide.graph.width > narrow.graph.width);
        assert!(wide.tree.width > narrow.tree.width);
        assert!(wide.logs.height > narrow.logs.height);
    }

    #[test]
    fn small_terminals_are_refused() {
        assert!(too_small(Rect::new(0, 0, 59, 24)));
        assert!(too_small(Rect::new(0, 0, 80, 15)));
        assert!(!too_small(Rect::new(0, 0, 60, 16)));
    }
}
