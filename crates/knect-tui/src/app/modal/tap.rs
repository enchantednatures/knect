//! Live CloudEvent tap modal state.

use knect_proto::tap::{TapEvent, TapHandle, TapTarget};

/// Maximum events retained by a tap modal.
pub const TAP_EVENT_LIMIT: usize = 200;

/// Started tap modal.
#[derive(Debug, Clone, PartialEq)]
pub struct TapModal {
    /// Broker or Trigger being tapped.
    pub target: TapTarget,
    /// Handle returned by `tap.start`.
    pub tap: Option<TapHandle>,
    /// Start failure.
    pub error: Option<String>,
    /// Bounded event buffer.
    pub events: Vec<TapEvent>,
    /// Selected event index.
    pub selected: usize,
    /// Scroll offset in the JSON scratch pane.
    pub scroll: usize,
    /// True while the tap is starting.
    pub starting: bool,
}

impl TapModal {
    /// Creates a starting modal.
    #[must_use]
    pub fn starting(target: TapTarget) -> Self {
        Self {
            target,
            tap: None,
            error: None,
            events: Vec::new(),
            selected: 0,
            scroll: 0,
            starting: true,
        }
    }

    /// Appends an event, dropping the oldest beyond the limit.
    pub fn push_event(&mut self, event: TapEvent) {
        self.events.push(event);
        if self.events.len() > TAP_EVENT_LIMIT {
            let excess = self.events.len() - TAP_EVENT_LIMIT;
            self.events.drain(..excess);
        }
        self.selected = self.events.len().saturating_sub(1);
        self.scroll = 0;
    }

    /// Selected event, if any.
    #[must_use]
    pub fn selected_event(&self) -> Option<&TapEvent> {
        self.events.get(self.selected)
    }

    /// Moves the selected event.
    pub fn select_next(&mut self, delta: i32) {
        if self.events.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = if delta < 0 {
            self.selected.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.selected
                .saturating_add(delta as usize)
                .min(self.events.len() - 1)
        };
        self.scroll = 0;
    }
}
