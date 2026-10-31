//! Top-level TUI state.

use knect_proto::{ResourceId, Topology};

/// Active screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Screen {
    /// Topology tree and graph.
    #[default]
    Graph,
    /// Mutation diff.
    Diff,
    /// CloudEvent tap.
    Tap,
    /// Help overlay.
    Help,
}

/// TUI application state.
#[derive(Debug, Clone, Default)]
pub struct AppState {
    /// Active screen.
    pub screen: Screen,
    /// Most recent topology snapshot.
    pub topology: Option<Topology>,
    /// Selected resource.
    pub selected: Option<ResourceId>,
    /// Set when the event loop should exit.
    pub should_quit: bool,
    /// Last error surfaced to the status line.
    pub status: Option<String>,
}

impl AppState {
    /// Creates the initial state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Switches screens.
    pub fn switch(&mut self, screen: Screen) {
        self.screen = screen;
    }

    /// Requests exit.
    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_graph_and_running() {
        let state = AppState::new();
        assert_eq!(state.screen, Screen::Graph);
        assert!(!state.should_quit);
        assert!(state.topology.is_none());
        assert!(state.selected.is_none());
    }

    #[test]
    fn state_transitions_are_explicit() {
        let mut state = AppState::new();
        state.switch(Screen::Tap);
        assert_eq!(state.screen, Screen::Tap);
        state.quit();
        assert!(state.should_quit);
    }
}
