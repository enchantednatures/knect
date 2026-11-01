//! UI events delivered to the event loop.

use knect_proto::rpc::DaemonStatus;
use knect_proto::{NodeDetails, ResourceId, SseEvent, Topology};
use ratatui::crossterm::event::KeyEvent;

use crate::app::state::ConnectionStatus;

/// Event delivered to the TUI event loop.
#[derive(Debug)]
pub enum UiEvent {
    /// Keyboard input.
    Key(KeyEvent),
    /// Periodic redraw tick.
    Tick,
    /// Terminal resize.
    Resize(u16, u16),
    /// Topology replacement from an RPC response.
    Topology(Box<Topology>),
    /// Daemon status response.
    Status(Box<DaemonStatus>),
    /// Inspector payload for one resource.
    Details {
        /// Resource that was requested.
        id: ResourceId,
        /// Redacted details, or a displayable error.
        result: Result<Box<NodeDetails>, String>,
    },
    /// Daemon event from JSONL or SSE.
    Daemon(Box<SseEvent>),
    /// Connection state change.
    Connection {
        /// New state.
        status: ConnectionStatus,
        /// Optional diagnostic message.
        message: Option<String>,
    },
    /// Log subscription acknowledgement.
    LogsSubscribed {
        /// Subscription ID from the daemon.
        subscription: String,
    },
    /// Non-fatal notice for the status bar.
    Notice {
        /// Message text.
        message: String,
        /// Render as an error banner when true.
        error: bool,
    },
    /// Shutdown requested by the event loop.
    Quit,
}
