//! UI events.

use ratatui::crossterm::event::KeyEvent;

/// Event delivered to the TUI event loop.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Keyboard input.
    Key(KeyEvent),
    /// Periodic redraw tick.
    Tick,
    /// Topology replacement from the client.
    Topology(knect_proto::Topology),
    /// Daemon event.
    Daemon(knect_proto::SseEvent),
    /// Shutdown requested by the client.
    Quit,
}
