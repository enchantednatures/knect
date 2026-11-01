//! User actions and the effects they request from the event loop.

use knect_proto::ResourceId;
use knect_proto::graph::NodeCategory;
use knect_proto::tap::PodRef;

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
    /// A key reserved for a later task.
    Reserved(&'static str),
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
        Action::Reserved(feature) => {
            state.set_status(format!("{feature} arrives in Task 14"));
            Vec::new()
        }
    }
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

    #[test]
    fn reserved_keys_leave_a_status_hook() {
        let mut state = AppState::new();
        let effects = apply_action(&mut state, Action::Reserved("mutation diff"));
        assert!(effects.is_empty());
        assert!(
            state
                .status
                .as_deref()
                .is_some_and(|status| status.contains("Task 14"))
        );
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
