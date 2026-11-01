//! Top-level TUI state and reducers.

use std::sync::Arc;

use knect_proto::events::{GraphPatch, NodeUpdated};
use knect_proto::graph::{Health, ResourceNode};
use knect_proto::rpc::DaemonStatus;
use knect_proto::tap::{LogLine, TapEvent};
use knect_proto::{NodeDetails, ResourceId, SseEvent, SseEventKind, Topology};

use crate::ui::graph::{GraphLayout, layout};
use crate::ui::tree::{TreeRow, build_rows, selectable_indices};

/// Active pane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Pane {
    /// Resource tree.
    #[default]
    Tree,
    /// Topology DAG.
    Graph,
    /// Inspector.
    Inspector,
    /// Log multiplexer.
    Logs,
}

impl Pane {
    /// Panes in cycle order.
    pub const ALL: [Self; 4] = [Self::Tree, Self::Graph, Self::Inspector, Self::Logs];

    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tree => "tree",
            Self::Graph => "topology",
            Self::Inspector => "inspector",
            Self::Logs => "logs",
        }
    }

    /// Next pane in cycle order.
    #[must_use]
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|pane| *pane == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// Previous pane in cycle order.
    #[must_use]
    pub fn previous(self) -> Self {
        let index = Self::ALL.iter().position(|pane| *pane == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Inspector tab.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InspectorTab {
    /// Mutation attribution.
    #[default]
    Mutations,
    /// Policy results.
    Policies,
    /// Discovered links.
    Links,
    /// Metrics and storage.
    Metrics,
}

impl InspectorTab {
    /// Tabs in display order.
    pub const ALL: [Self; 4] = [Self::Mutations, Self::Policies, Self::Links, Self::Metrics];

    /// Tab label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mutations => "Mutations",
            Self::Policies => "Policies",
            Self::Links => "Links",
            Self::Metrics => "Metrics",
        }
    }

    /// One-based index.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::Mutations => 1,
            Self::Policies => 2,
            Self::Links => 3,
            Self::Metrics => 4,
        }
    }

    /// Tab from a one-based index.
    #[must_use]
    pub const fn from_number(number: u8) -> Option<Self> {
        match number {
            1 => Some(Self::Mutations),
            2 => Some(Self::Policies),
            3 => Some(Self::Links),
            4 => Some(Self::Metrics),
            _ => None,
        }
    }

    /// Next tab, wrapping.
    #[must_use]
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// Previous tab, wrapping.
    #[must_use]
    pub fn previous(self) -> Self {
        let index = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Input mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InputMode {
    /// Normal navigation.
    #[default]
    Normal,
    /// Editing the tree filter.
    Filter,
}

/// Connection state shown in the status bar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// No connection attempt has completed.
    #[default]
    Connecting,
    /// The daemon is reachable and events are flowing.
    Connected,
    /// Reconnecting after a failure.
    Reconnecting {
        /// Consecutive failed attempts.
        attempt: u32,
    },
    /// No usable endpoint.
    Disconnected(String),
}

impl ConnectionStatus {
    /// Short status text.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Connecting => "connecting".to_owned(),
            Self::Connected => "connected".to_owned(),
            Self::Reconnecting { attempt } => format!("reconnecting ({attempt})"),
            Self::Disconnected(reason) => format!("disconnected: {reason}"),
        }
    }

    /// True when connected.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }
}

/// TUI application state.
#[derive(Debug, Clone, Default)]
pub struct AppState {
    /// Most recent topology snapshot.
    pub topology: Option<Topology>,
    /// Derived layered DAG layout for the current topology.
    pub graph_layout: Option<Arc<GraphLayout>>,
    /// Derived tree rows for the current topology and filter.
    pub tree_rows: Vec<TreeRow>,
    /// Selected resource.
    pub selected: Option<ResourceId>,
    /// Row index of the selected resource.
    pub selected_row: usize,
    /// Tree filter text.
    pub filter: String,
    /// Filter prompt or normal mode.
    pub input_mode: InputMode,
    /// Active pane.
    pub active_pane: Pane,
    /// Active inspector tab.
    pub inspector_tab: InspectorTab,
    /// Tree scroll offset.
    pub tree_offset: usize,
    /// Graph pan offset.
    pub graph_offset: (i32, i32),
    /// Inspector scroll offset.
    pub inspector_offset: usize,
    /// Log scroll offset; zero means the newest line.
    pub logs_offset: usize,
    /// Log filter show in the pane title.
    pub log_filter: String,
    /// Active log subscription ID.
    pub log_subscription: Option<String>,
    /// Recent log lines.
    pub logs: Vec<LogLine>,
    /// Recent CloudEvent tap events.
    pub tap_events: Vec<TapEvent>,
    /// Whether the tap pane is visible.
    pub tap_visible: bool,
    /// Connection state.
    pub connection: ConnectionStatus,
    /// Latest daemon status.
    pub daemon: Option<DaemonStatus>,
    /// Inspector payload for the selection.
    pub details: Option<NodeDetails>,
    /// Inspector error for the selection.
    pub details_error: Option<String>,
    /// Transient status message.
    pub status: Option<String>,
    /// Persistent error banner.
    pub error_banner: Option<String>,
    /// Terminal endpoint label.
    pub endpoint: Option<String>,
    /// ASCII-only rendering.
    pub ascii: bool,
    /// Last received event sequence.
    pub last_event_seq: Option<u64>,
    /// Set when the event loop should exit.
    pub should_quit: bool,
    /// Whether the help overlay is open.
    pub help_visible: bool,
    /// `g` was pressed and a follow-up key is pending.
    pub pending_g: bool,
    /// Set when the next draw is needed.
    pub dirty: bool,
}

impl AppState {
    /// Creates the initial state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            dirty: true,
            ..Self::default()
        }
    }

    /// Creates state with ASCII rendering selected.
    #[must_use]
    pub fn with_ascii(ascii: bool) -> Self {
        let mut state = Self::new();
        state.ascii = ascii;
        state
    }

    /// Marks the frame dirty.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Consumes the dirty flag.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Selects the initial pane focus for a CLI-launched selection.
    pub fn focus_details(&mut self) {
        self.active_pane = Pane::Inspector;
        self.mark_dirty();
    }

    /// Requests exit.
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Sets the endpoint label.
    pub fn set_endpoint(&mut self, endpoint: impl Into<String>) {
        self.endpoint = Some(endpoint.into());
        self.mark_dirty();
    }

    /// Sets a transient status message.
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some(message.into());
        self.mark_dirty();
    }

    /// Sets the persistent error banner.
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error_banner = Some(message.into());
        self.mark_dirty();
    }

    /// Replaces the connection state.
    pub fn set_connection(&mut self, connection: ConnectionStatus) {
        self.connection = connection;
        self.mark_dirty();
    }

    /// Replaces the current topology and all derived state.
    pub fn set_topology(&mut self, mut topology: Topology) {
        if topology.stats.node_count as usize != topology.nodes.len()
            || topology.stats.edge_count as usize != topology.edges.len()
        {
            topology.stats.node_count = topology.nodes.len() as u64;
            topology.stats.edge_count = topology.edges.len() as u64;
        }
        if topology.schema_version != knect_proto::SCHEMA_VERSION {
            self.set_error(format!(
                "daemon schema_version {} does not match client {}",
                topology.schema_version,
                knect_proto::SCHEMA_VERSION
            ));
        }
        self.graph_layout = Some(Arc::new(layout(&topology)));
        self.topology = Some(topology);
        self.refresh_tree();
        self.dirty = true;
    }

    /// Applies an incremental graph patch.
    pub fn apply_graph_patch(&mut self, patch: GraphPatch) {
        let Some(mut topology) = self.topology.take() else {
            return;
        };
        for removed in &patch.removed_nodes {
            topology.nodes.retain(|node| &node.id != removed);
            topology
                .edges
                .retain(|edge| &edge.from != removed && &edge.to != removed);
        }
        for added in patch.added_nodes {
            if let Some(existing) = topology.nodes.iter_mut().find(|node| node.id == added.id) {
                *existing = added;
            } else {
                topology.nodes.push(added);
            }
        }
        for updated in patch.updated_nodes {
            if let Some(existing) = topology.nodes.iter_mut().find(|node| node.id == updated.id) {
                *existing = updated;
            } else {
                topology.nodes.push(updated);
            }
        }
        for added in patch.added_edges {
            if let Some(existing) = topology.edges.iter_mut().find(|edge| edge.id == added.id) {
                *existing = added;
            } else {
                topology.edges.push(added);
            }
        }
        for removed in &patch.removed_edges {
            topology.edges.retain(|edge| &edge.id != removed);
        }
        topology.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        topology.edges.sort_by(|left, right| left.id.cmp(&right.id));
        topology.generation = patch.generation;
        topology.stats.node_count = topology.nodes.len() as u64;
        topology.stats.edge_count = topology.edges.len() as u64;
        self.last_event_seq = Some(self.last_event_seq.unwrap_or(0).max(patch.seq));
        self.set_topology(topology);
    }

    /// Applies a single-node update.
    pub fn apply_node_updated(&mut self, update: NodeUpdated) {
        let Some(mut topology) = self.topology.take() else {
            return;
        };
        if let Some(existing) = topology
            .nodes
            .iter_mut()
            .find(|node| node.id == update.node.id)
        {
            *existing = update.node;
        } else {
            topology.nodes.push(update.node);
        }
        topology.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        topology.generation = update.generation;
        topology.stats.node_count = topology.nodes.len() as u64;
        self.set_topology(topology);
    }

    /// Applies one daemon event.
    pub fn apply_event(&mut self, event: SseEvent) {
        self.last_event_seq = Some(self.last_event_seq.unwrap_or(0).max(event.seq));
        match event.kind {
            SseEventKind::Snapshot { data } => self.set_topology(data),
            SseEventKind::GraphPatch { data } => self.apply_graph_patch(data),
            SseEventKind::NodeUpdated { data } => self.apply_node_updated(data),
            SseEventKind::LogLine { data } => {
                self.logs.push(data);
                if self.logs.len() > 5_000 {
                    let excess = self.logs.len() - 5_000;
                    self.logs.drain(..excess);
                }
                self.dirty = true;
            }
            SseEventKind::TapEvent { data } => {
                self.tap_events.push(data);
                if self.tap_events.len() > 500 {
                    let excess = self.tap_events.len() - 500;
                    self.tap_events.drain(..excess);
                }
                self.dirty = true;
            }
            SseEventKind::Heartbeat { data } => {
                if let Some(daemon) = self.daemon.as_mut() {
                    daemon.generation = data.generation;
                    daemon.uptime_secs = data.uptime_secs;
                }
                self.dirty = true;
            }
            SseEventKind::PolicyViolation { data } => {
                self.set_status(format!(
                    "policy violation: {} ({:?})",
                    data.policy.name, data.policy.severity
                ));
            }
            SseEventKind::ActionResult { data } => {
                self.set_status(format!(
                    "action {}: {}",
                    data.action,
                    if data.success { "ok" } else { "failed" }
                ));
            }
            SseEventKind::DaemonError { data } => {
                if data.retryable {
                    self.set_status(format!("daemon warning: {}", data.message));
                } else {
                    self.set_error(format!("daemon error: {}", data.message));
                }
            }
        }
    }

    /// Rebuilds tree rows and keeps selection valid.
    pub fn refresh_tree(&mut self) {
        self.tree_rows = build_rows(self.topology.as_ref(), &self.filter);
        let selected = self.selected.clone();
        if let Some(id) = selected
            && let Some(index) = self
                .tree_rows
                .iter()
                .position(|row| row.resource.as_ref() == Some(&id))
        {
            self.selected_row = index;
            return;
        }
        self.select_first();
    }

    /// Selects the first resource.
    pub fn select_first(&mut self) {
        match self.tree_rows.iter().position(|row| row.selectable()) {
            Some(first) => self.set_selected_row(first),
            None => {
                self.selected = None;
                self.selected_row = 0;
                self.details = None;
                self.details_error = None;
                self.dirty = true;
            }
        }
    }

    /// Selects the last resource.
    pub fn select_last(&mut self) {
        let last = self
            .tree_rows
            .iter()
            .rposition(|row| row.selectable())
            .unwrap_or(0);
        self.set_selected_row(last);
    }

    /// Moves selection by `delta` selectable rows.
    pub fn move_selection(&mut self, delta: i32) {
        let indices = selectable_indices(&self.tree_rows);
        if indices.is_empty() {
            return;
        }
        let current = indices.iter().position(|index| *index == self.selected_row);
        let target = match (delta, current) {
            (delta, Some(position)) if delta < 0 => position.saturating_sub(1),
            (_, Some(position)) => (position + 1).min(indices.len() - 1),
            (delta, None) if delta < 0 => indices.len() - 1,
            (_, None) => 0,
        };
        self.set_selected_row(indices[target]);
    }

    fn set_selected_row(&mut self, row_index: usize) {
        let Some(row) = self.tree_rows.get(row_index) else {
            return;
        };
        let Some(resource) = row.resource.clone() else {
            return;
        };
        if self.selected.as_ref() != Some(&resource) {
            self.selected = Some(resource);
            self.selected_row = row_index;
            self.details = None;
            self.details_error = None;
            self.inspector_offset = 0;
            self.dirty = true;
        } else {
            self.selected_row = row_index;
        }
    }

    /// Selects a resource by identity when it is present in the tree.
    pub fn select_resource(&mut self, id: &ResourceId) -> bool {
        let Some(index) = self
            .tree_rows
            .iter()
            .position(|row| row.resource.as_ref() == Some(id))
        else {
            return false;
        };
        self.set_selected_row(index);
        true
    }

    /// Applies a filter and revalidates selection.
    pub fn set_filter(&mut self, filter: impl Into<String>) {
        self.filter = filter.into();
        self.refresh_tree();
        self.dirty = true;
    }

    /// Selected node projection.
    #[must_use]
    pub fn selected_node(&self) -> Option<&ResourceNode> {
        let id = self.selected.as_ref()?;
        self.topology
            .as_ref()?
            .nodes
            .iter()
            .find(|node| &node.id == id)
    }

    /// Selected node health.
    #[must_use]
    pub fn selected_health(&self) -> Option<Health> {
        self.selected_node().map(|node| node.health)
    }

    /// Cycles the active pane.
    pub fn cycle_pane(&mut self, forward: bool) {
        self.active_pane = if forward {
            self.active_pane.next()
        } else {
            self.active_pane.previous()
        };
        self.dirty = true;
    }

    /// Selects an inspector tab by one-based number.
    pub fn select_tab(&mut self, number: u8) -> bool {
        let Some(tab) = InspectorTab::from_number(number) else {
            return false;
        };
        self.inspector_tab = tab;
        self.active_pane = Pane::Inspector;
        self.inspector_offset = 0;
        self.dirty = true;
        true
    }

    /// Moves the graph pan offset.
    pub fn pan_graph(&mut self, dx: i32, dy: i32) {
        self.graph_offset.0 = self.graph_offset.0.saturating_add(dx).max(0);
        self.graph_offset.1 = self.graph_offset.1.saturating_add(dy).max(0);
        self.dirty = true;
    }

    /// Scrolls the inspector.
    pub fn scroll_inspector(&mut self, delta: i32) {
        self.inspector_offset = if delta < 0 {
            self.inspector_offset
                .saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.inspector_offset.saturating_add(delta as usize)
        };
        self.dirty = true;
    }

    /// Scrolls the logs; zero is the newest line.
    pub fn scroll_logs(&mut self, delta: i32) {
        self.logs_offset = if delta < 0 {
            self.logs_offset
                .saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.logs_offset.saturating_add(delta as usize)
        };
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use knect_proto::graph::{NodeCategory, TopologyStats};

    fn node(name: &str) -> ResourceNode {
        ResourceNode {
            id: ResourceId::namespaced("serving.knative.dev/v1", "Service", "default", name),
            category: NodeCategory::KService,
            name: name.to_owned(),
            namespace: Some("default".to_owned()),
            health: Health::Healthy,
            summary: String::new(),
            labels: Default::default(),
            annotations: Default::default(),
            badges: Vec::new(),
            uid: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        }
    }

    fn topology(nodes: Vec<ResourceNode>) -> Topology {
        Topology {
            schema_version: 1,
            context: "test".to_owned(),
            generation: 1,
            stats: TopologyStats::default(),
            nodes,
            edges: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn initial_state_is_running_and_dirty() {
        let mut state = AppState::new();
        assert!(state.dirty);
        assert!(!state.should_quit);
        assert!(state.topology.is_none());
        assert!(state.selected.is_none());
        assert!(state.take_dirty());
        assert!(!state.take_dirty());
    }

    #[test]
    fn topology_sets_selection_and_layout() {
        let mut state = AppState::new();
        state.set_topology(topology(vec![node("b"), node("a")]));
        assert_eq!(
            state.selected.as_ref().map(|id| id.name.as_str()),
            Some("a")
        );
        assert_eq!(state.tree_rows.len(), 4);
        assert!(state.graph_layout.is_some());
        assert!(state.dirty);
    }

    #[test]
    fn selection_moves_through_selectable_rows_only() {
        let mut state = AppState::new();
        state.set_topology(topology(vec![node("a"), node("b")]));
        assert_eq!(
            state.selected.as_ref().map(|id| id.name.as_str()),
            Some("a")
        );
        state.move_selection(1);
        assert_eq!(
            state.selected.as_ref().map(|id| id.name.as_str()),
            Some("b")
        );
        state.move_selection(1);
        assert_eq!(
            state.selected.as_ref().map(|id| id.name.as_str()),
            Some("b")
        );
        state.move_selection(-1);
        assert_eq!(
            state.selected.as_ref().map(|id| id.name.as_str()),
            Some("a")
        );
    }

    #[test]
    fn filter_narrows_the_tree_and_keeps_selection_valid() {
        let mut state = AppState::new();
        state.set_topology(topology(vec![node("checkout"), node("events")]));
        state.set_filter("check");
        assert_eq!(
            state.selected.as_ref().map(|id| id.name.as_str()),
            Some("checkout")
        );
        state.set_filter("missing");
        assert!(state.selected.is_none());
    }

    #[test]
    fn pane_and_tab_cycles_wrap() {
        let mut state = AppState::new();
        state.cycle_pane(true);
        assert_eq!(state.active_pane, Pane::Graph);
        state.cycle_pane(false);
        assert_eq!(state.active_pane, Pane::Tree);
        state.cycle_pane(false);
        assert_eq!(state.active_pane, Pane::Logs);
        assert!(state.select_tab(3));
        assert_eq!(state.inspector_tab, InspectorTab::Links);
        assert_eq!(state.active_pane, Pane::Inspector);
        assert!(!state.select_tab(9));
    }

    #[test]
    fn graph_patches_add_update_and_remove_nodes() {
        let mut state = AppState::new();
        let first = node("a");
        state.set_topology(topology(vec![first.clone()]));
        let mut updated = first.clone();
        updated.health = Health::Degraded;
        state.apply_graph_patch(GraphPatch {
            generation: 2,
            added_nodes: vec![node("b")],
            updated_nodes: vec![updated],
            removed_nodes: Vec::new(),
            added_edges: Vec::new(),
            removed_edges: Vec::new(),
            seq: 1,
            ts: Utc::now(),
        });
        let topology = state.topology.as_ref().expect("topology");
        assert_eq!(topology.generation, 2);
        assert_eq!(topology.nodes.len(), 2);
        assert_eq!(state.last_event_seq, Some(1));
        state.apply_graph_patch(GraphPatch {
            generation: 3,
            added_nodes: Vec::new(),
            updated_nodes: Vec::new(),
            removed_nodes: vec![first.id.clone()],
            added_edges: Vec::new(),
            removed_edges: Vec::new(),
            seq: 2,
            ts: Utc::now(),
        });
        let topology = state.topology.as_ref().expect("topology");
        assert_eq!(topology.nodes.len(), 1);
    }

    #[test]
    fn log_events_are_capped_and_mark_dirty() {
        let mut state = AppState::new();
        for index in 0..5_010 {
            state.apply_event(SseEvent::new(
                index as u64,
                Utc::now(),
                SseEventKind::LogLine {
                    data: LogLine {
                        namespace: "default".to_owned(),
                        pod: "p".to_owned(),
                        container: "c".to_owned(),
                        timestamp: Utc::now(),
                        message: index.to_string(),
                        stream: knect_proto::tap::LogStream::Stdout,
                    },
                },
            ));
        }
        assert_eq!(state.logs.len(), 5_000);
        assert!(state.dirty);
    }
}
