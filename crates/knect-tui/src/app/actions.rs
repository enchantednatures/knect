//! User actions and the effects they request from the event loop.

use knect_proto::ResourceId;
use knect_proto::graph::NodeCategory;
use knect_proto::rpc::{SetTrafficSplitParams, TapStartParams};
use knect_proto::tap::PodRef;

use crate::app::keys::ModalKey;
use crate::app::modal::Modal;
use crate::app::state::{AppState, InputMode, Pane};

/// High-level UI action produced by key handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Exit the TUI.
    Quit,
    /// Force a topology refresh.
    Refresh,
    /// Toggle the help overlay.
    ToggleHelp,
    /// Close the help overlay or cancel the filter prompt.
    CloseOverlay,
    /// Enter filter editing mode.
    BeginFilter,
    /// Append one character to the filter.
    FilterChar(char),
    /// Remove the last filter character.
    FilterBackspace,
    /// Apply the filter and leave edit mode.
    FilterCommit,
    /// Discard the filter and leave edit mode.
    FilterCancel,
    /// Move selection or scroll up.
    Up,
    /// Move selection or scroll down.
    Down,
    /// Pan left or switch to the previous inspector tab.
    Left,
    /// Pan right or switch to the next inspector tab.
    Right,
    /// Jump to the top.
    Top,
    /// Jump to the bottom.
    Bottom,
    /// Cycle to the next pane.
    PaneNext,
    /// Cycle to the previous pane.
    PanePrevious,
    /// Select an inspector tab by one-based index.
    InspectorTab(u8),
    /// Focus the inspector.
    FocusDetails,
    /// `g` was pressed and a follow-up key is pending.
    PendingG,
    /// Open the mutation diff modal.
    OpenDiff,
    /// Open the traffic split editor.
    OpenTraffic,
    /// Open the CloudEvent tap modal.
    OpenTap,
}

/// Work the event loop must launch asynchronously.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Refresh daemon status and topology.
    Refresh,
    /// Fetch inspector details for one resource.
    FetchDetails(ResourceId),
    /// Start a log subscription for the selected workload's pods.
    SubscribeLogs(Vec<PodRef>),
    /// Fetch the three-way mutation diff for one resource.
    FetchDiff(ResourceId),
    /// Validate or write a traffic split.
    SetTrafficSplit(SetTrafficSplitParams),
    /// Start a CloudEvent tap.
    StartTap(TapStartParams),
    /// Stop a CloudEvent tap.
    StopTap(String),
}

/// Applies an action to state and returns follow-up effects.
pub fn apply_action(state: &mut AppState, action: Action) -> Vec<Effect> {
    match action {
        Action::Quit => {
            state.quit();
            Vec::new()
        }
        Action::Refresh => {
            state.set_status("refreshing topology…");
            vec![Effect::Refresh]
        }
        Action::ToggleHelp => {
            state.help_visible = !state.help_visible;
            state.mark_dirty();
            Vec::new()
        }
        Action::CloseOverlay => {
            if state.help_visible {
                state.help_visible = false;
            } else if state.input_mode == InputMode::Filter {
                state.input_mode = InputMode::Normal;
                state.set_filter(String::new());
            } else {
                state.status = None;
            }
            state.mark_dirty();
            Vec::new()
        }
        Action::BeginFilter => {
            state.input_mode = InputMode::Filter;
            state.set_status("filter: type to narrow, Enter applies, Esc cancels");
            Vec::new()
        }
        Action::FilterChar(character) => {
            let mut filter = state.filter.clone();
            filter.push(character);
            state.set_filter(filter);
            Vec::new()
        }
        Action::FilterBackspace => {
            let mut filter = state.filter.clone();
            filter.pop();
            state.set_filter(filter);
            Vec::new()
        }
        Action::FilterCommit => {
            state.input_mode = InputMode::Normal;
            state.mark_dirty();
            selection_effects(state)
        }
        Action::FilterCancel => {
            state.input_mode = InputMode::Normal;
            state.set_filter(String::new());
            Vec::new()
        }
        Action::Up => match state.active_pane {
            Pane::Tree => {
                state.move_selection(-1);
                selection_effects(state)
            }
            Pane::Graph => {
                state.pan_graph(0, -1);
                Vec::new()
            }
            Pane::Inspector => {
                state.scroll_inspector(-1);
                Vec::new()
            }
            Pane::Logs => {
                state.scroll_logs(-1);
                Vec::new()
            }
        },
        Action::Down => match state.active_pane {
            Pane::Tree => {
                state.move_selection(1);
                selection_effects(state)
            }
            Pane::Graph => {
                state.pan_graph(0, 1);
                Vec::new()
            }
            Pane::Inspector => {
                state.scroll_inspector(1);
                Vec::new()
            }
            Pane::Logs => {
                state.scroll_logs(1);
                Vec::new()
            }
        },
        Action::Left => match state.active_pane {
            Pane::Graph => {
                state.pan_graph(-1, 0);
                Vec::new()
            }
            Pane::Inspector => {
                state.inspector_tab = state.inspector_tab.previous();
                state.inspector_offset = 0;
                state.mark_dirty();
                Vec::new()
            }
            Pane::Tree | Pane::Logs => Vec::new(),
        },
        Action::Right => match state.active_pane {
            Pane::Graph => {
                state.pan_graph(1, 0);
                Vec::new()
            }
            Pane::Inspector => {
                state.inspector_tab = state.inspector_tab.next();
                state.inspector_offset = 0;
                state.mark_dirty();
                Vec::new()
            }
            Pane::Tree | Pane::Logs => Vec::new(),
        },
        Action::Top => {
            match state.active_pane {
                Pane::Tree => state.select_first(),
                Pane::Graph => state.graph_offset = (0, 0),
                Pane::Inspector => state.inspector_offset = 0,
                Pane::Logs => {
                    state.logs_offset = state.logs.len();
                }
            }
            state.mark_dirty();
            selection_effects(state)
        }
        Action::Bottom => {
            match state.active_pane {
                Pane::Tree => state.select_last(),
                Pane::Graph => {
                    let height = state
                        .graph_layout
                        .as_ref()
                        .map(|layout| layout.height.saturating_sub(4))
                        .unwrap_or(0);
                    state.graph_offset = (state.graph_offset.0, height.max(0));
                }
                Pane::Inspector => state.inspector_offset = 10_000,
                Pane::Logs => state.logs_offset = 0,
            }
            state.mark_dirty();
            selection_effects(state)
        }
        Action::PaneNext => {
            state.cycle_pane(true);
            Vec::new()
        }
        Action::PanePrevious => {
            state.cycle_pane(false);
            Vec::new()
        }
        Action::InspectorTab(number) => {
            state.select_tab(number);
            selection_effects(state)
        }
        Action::FocusDetails => {
            state.focus_details();
            selection_effects(state)
        }
        Action::PendingG => Vec::new(),
        Action::OpenDiff => {
            let Some(id) = state.selected.clone() else {
                state.set_status("select a resource before opening its mutation diff");
                return Vec::new();
            };
            if state.open_diff() {
                vec![Effect::FetchDiff(id)]
            } else {
                Vec::new()
            }
        }
        Action::OpenTraffic => {
            state.open_traffic();
            Vec::new()
        }
        Action::OpenTap => open_tap_effects(state),
    }
}

/// Applies one key press inside the active modal.
pub fn apply_modal_key(state: &mut AppState, key: ModalKey) -> Vec<Effect> {
    match key {
        ModalKey::Close => close_modal(state),
        ModalKey::Refresh => match &mut state.modal {
            Modal::Diff(modal) => {
                modal.loading = true;
                modal.error = None;
                vec![Effect::FetchDiff(modal.resource.clone())]
            }
            Modal::Traffic(_) => {
                state.refresh_traffic();
                Vec::new()
            }
            Modal::Tap(_) | Modal::None => Vec::new(),
        },
        ModalKey::Up => {
            scroll_modal(state, -1);
            Vec::new()
        }
        ModalKey::Down => {
            scroll_modal(state, 1);
            Vec::new()
        }
        ModalKey::Left => {
            if let Modal::Traffic(modal) = &mut state.modal {
                modal.adjust(-5);
                state.mark_dirty();
            }
            Vec::new()
        }
        ModalKey::Right => {
            if let Modal::Traffic(modal) = &mut state.modal {
                modal.adjust(5);
                state.mark_dirty();
            }
            Vec::new()
        }
        ModalKey::Top => {
            jump_modal(state, true);
            Vec::new()
        }
        ModalKey::Bottom => {
            jump_modal(state, false);
            Vec::new()
        }
        ModalKey::Text(character) => {
            if let Modal::Traffic(modal) = &mut state.modal {
                if modal.edit.is_some() {
                    modal.push_char(character);
                } else if character.is_ascii_digit() {
                    modal.edit_percent(character);
                }
                state.mark_dirty();
            }
            Vec::new()
        }
        ModalKey::Backspace => {
            if let Modal::Traffic(modal) = &mut state.modal {
                modal.backspace();
                state.mark_dirty();
            }
            Vec::new()
        }
        ModalKey::Commit => {
            if let Modal::Traffic(modal) = &mut state.modal {
                if let Err(error) = modal.commit_edit() {
                    modal.error = Some(error);
                }
                state.mark_dirty();
            }
            Vec::new()
        }
        ModalKey::CancelEdit => {
            if let Modal::Traffic(modal) = &mut state.modal {
                modal.cancel_edit();
                state.mark_dirty();
            }
            Vec::new()
        }
        ModalKey::BeginTagEdit => {
            if let Modal::Traffic(modal) = &mut state.modal {
                modal.edit_tag();
                state.mark_dirty();
            }
            Vec::new()
        }
        ModalKey::Preview => submit_traffic(state, true),
        ModalKey::Submit => {
            if let Modal::Traffic(modal) = &mut state.modal
                && !modal.confirm_pending
            {
                modal.confirm_pending = true;
                state.set_status("press s again to apply the traffic split");
                state.mark_dirty();
                return Vec::new();
            }
            submit_traffic(state, false)
        }
    }
}

fn open_tap_effects(state: &mut AppState) -> Vec<Effect> {
    if !state.open_tap() {
        return Vec::new();
    }
    let Modal::Tap(modal) = &state.modal else {
        return Vec::new();
    };
    vec![Effect::StartTap(TapStartParams {
        target: modal.target.clone(),
        filter: None,
        max_events: Some(200),
        ttl_secs: None,
    })]
}

fn close_modal(state: &mut AppState) -> Vec<Effect> {
    let tap = match &state.modal {
        Modal::Tap(modal) => modal.tap.as_ref().map(|handle| handle.id.clone()),
        _ => None,
    };
    state.close_modal();
    tap.map_or_else(Vec::new, |id| vec![Effect::StopTap(id)])
}

fn scroll_modal(state: &mut AppState, delta: i32) {
    match &mut state.modal {
        Modal::Diff(modal) => {
            let max = modal.max_lines().saturating_sub(1);
            modal.scroll = shift(modal.scroll, delta, max);
        }
        Modal::Traffic(modal) => modal.select_next(delta),
        Modal::Tap(modal) => modal.select_next(delta),
        Modal::None => {}
    }
    state.mark_dirty();
}

fn jump_modal(state: &mut AppState, top: bool) {
    match &mut state.modal {
        Modal::Diff(modal) => {
            modal.scroll = if top {
                0
            } else {
                modal.max_lines().saturating_sub(1)
            };
        }
        Modal::Traffic(modal) => {
            modal.selected = if top {
                0
            } else {
                modal.rows.len().saturating_sub(1)
            };
        }
        Modal::Tap(modal) => {
            modal.selected = if top {
                0
            } else {
                modal.events.len().saturating_sub(1)
            };
            modal.scroll = 0;
        }
        Modal::None => {}
    }
    state.mark_dirty();
}

fn shift(value: usize, delta: i32, max: usize) -> usize {
    if delta < 0 {
        value.saturating_sub(delta.unsigned_abs() as usize)
    } else {
        value.saturating_add(delta as usize).min(max)
    }
}

fn submit_traffic(state: &mut AppState, dry_run: bool) -> Vec<Effect> {
    let Modal::Traffic(modal) = &mut state.modal else {
        return Vec::new();
    };
    if modal.loading {
        return Vec::new();
    }
    if let Err(error) = modal.commit_edit() {
        modal.error = Some(error);
        modal.confirm_pending = false;
        state.mark_dirty();
        return Vec::new();
    }
    if let Err(error) = modal.validate() {
        modal.error = Some(error);
        modal.confirm_pending = false;
        state.mark_dirty();
        return Vec::new();
    }
    let Some(params) = modal.params(dry_run) else {
        modal.error = Some("traffic edits require a namespaced Service".to_owned());
        state.mark_dirty();
        return Vec::new();
    };
    modal.loading = true;
    modal.error = None;
    modal.result = None;
    modal.confirm_pending = false;
    modal.edit = None;
    state.mark_dirty();
    vec![Effect::SetTrafficSplit(params)]
}

pub(crate) fn selection_effects(state: &AppState) -> Vec<Effect> {
    let Some(id) = state.selected.clone() else {
        return Vec::new();
    };
    let mut effects = Vec::new();
    if state.details.as_ref().map(|details| &details.id) != Some(&id) {
        effects.push(Effect::FetchDetails(id.clone()));
    }
    let pods = log_pods(state);
    if !pods.is_empty() && state.log_subscription.is_none() {
        effects.push(Effect::SubscribeLogs(pods));
    }
    effects
}

fn log_pods(state: &AppState) -> Vec<PodRef> {
    let Some(node) = state.selected_node() else {
        return Vec::new();
    };
    let Some(namespace) = node.namespace.clone() else {
        return Vec::new();
    };
    match node.category {
        NodeCategory::Pod => vec![PodRef {
            namespace,
            name: node.name.clone(),
            container: None,
        }],
        NodeCategory::Revision => {
            let revision = node.name.as_str();
            state
                .topology
                .as_ref()
                .map(|topology| {
                    topology
                        .nodes
                        .iter()
                        .filter(|candidate| candidate.category == NodeCategory::Pod)
                        .filter(|candidate| {
                            candidate
                                .labels
                                .get("serving.knative.dev/revision")
                                .is_some_and(|value| value == revision)
                        })
                        .filter_map(|pod| {
                            pod.namespace.clone().map(|namespace| PodRef {
                                namespace,
                                name: pod.name.clone(),
                                container: None,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{AppState, InspectorTab, Pane};
    use chrono::Utc;
    use knect_proto::graph::{Health, ResourceNode, Topology, TopologyStats};
    use std::collections::BTreeMap;

    fn node(category: NodeCategory, kind: &str, name: &str) -> ResourceNode {
        ResourceNode {
            id: ResourceId::namespaced("serving.knative.dev/v1", kind, "default", name),
            category,
            name: name.to_owned(),
            namespace: Some("default".to_owned()),
            health: Health::Healthy,
            summary: String::new(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            badges: Vec::new(),
            uid: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        }
    }

    fn topology(nodes: Vec<ResourceNode>, edges: Vec<knect_proto::graph::Edge>) -> Topology {
        Topology {
            schema_version: 1,
            context: "test".to_owned(),
            generation: 1,
            nodes,
            edges,
            warnings: Vec::new(),
            stats: TopologyStats::default(),
        }
    }

    fn broker_topology() -> Topology {
        topology(
            vec![node(NodeCategory::Broker, "Broker", "orders")],
            Vec::new(),
        )
    }

    fn traffic_topology() -> Topology {
        let service = node(NodeCategory::KService, "Service", "hello");
        let route = node(NodeCategory::Route, "Route", "hello");
        let revision = node(NodeCategory::Revision, "Revision", "hello-00001");
        let mut owns = knect_proto::graph::Edge {
            id: "e1".to_owned(),
            from: service.id.clone(),
            to: route.id.clone(),
            kind: knect_proto::graph::EdgeKind::Owns,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        };
        owns.metadata.insert("route".to_owned(), "hello".into());
        let mut traffic = knect_proto::graph::Edge {
            id: "e2".to_owned(),
            from: route.id.clone(),
            to: revision.id.clone(),
            kind: knect_proto::graph::EdgeKind::RoutesTo,
            label: None,
            health: Health::Healthy,
            warnings: Vec::new(),
            metadata: BTreeMap::new(),
        };
        traffic.metadata.insert("percent".to_owned(), 100.into());
        topology(vec![service, route, revision], vec![owns, traffic])
    }

    #[test]
    fn opening_a_diff_requires_a_selection() {
        let mut state = AppState::new();
        let effects = apply_action(&mut state, Action::OpenDiff);
        assert!(effects.is_empty());
        assert!(state.status.is_some());
    }

    #[test]
    fn opening_a_tap_starts_the_selected_broker() {
        let mut state = AppState::new();
        state.set_topology(broker_topology());
        assert!(state.selected.is_some());
        let effects = apply_action(&mut state, Action::OpenTap);
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::StartTap(_)));
    }

    #[test]
    fn traffic_modal_requires_a_valid_split_before_submitting() {
        let mut state = AppState::new();
        state.set_topology(traffic_topology());
        assert!(state.selected.is_some());
        assert!(apply_action(&mut state, Action::OpenTraffic).is_empty());
        let effects = apply_modal_key(&mut state, crate::app::keys::ModalKey::Preview);
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::SetTrafficSplit(_)));
    }

    #[test]
    fn help_and_filter_overlays_close_cleanly() {
        let mut state = AppState::new();
        assert!(!state.help_visible);
        apply_action(&mut state, Action::ToggleHelp);
        assert!(state.help_visible);
        apply_action(&mut state, Action::CloseOverlay);
        assert!(!state.help_visible);

        apply_action(&mut state, Action::BeginFilter);
        apply_action(&mut state, Action::FilterChar('a'));
        apply_action(&mut state, Action::FilterChar('b'));
        assert_eq!(state.filter, "ab");
        apply_action(&mut state, Action::FilterBackspace);
        assert_eq!(state.filter, "a");
        apply_action(&mut state, Action::FilterCancel);
        assert_eq!(state.filter, "");
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn inspector_tab_actions_focus_the_inspector() {
        let mut state = AppState::new();
        apply_action(&mut state, Action::InspectorTab(3));
        assert_eq!(state.active_pane, Pane::Inspector);
        assert_eq!(state.inspector_tab, InspectorTab::Links);
    }

    #[test]
    fn graph_arrows_pan_without_changing_selection() {
        let mut state = AppState::new();
        state.active_pane = Pane::Graph;
        apply_action(&mut state, Action::Down);
        apply_action(&mut state, Action::Right);
        assert_eq!(state.graph_offset, (1, 1));
        apply_action(&mut state, Action::Top);
        assert_eq!(state.graph_offset, (0, 0));
    }

    #[test]
    fn refresh_requests_a_daemon_refresh() {
        let mut state = AppState::new();
        let effects = apply_action(&mut state, Action::Refresh);
        assert_eq!(effects, vec![Effect::Refresh]);
    }
}
