//! User actions produced by key handling.

use crate::app::state::Screen;

/// High-level UI action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Exit the TUI.
    Quit,
    /// Force a topology refresh.
    Refresh,
    /// Switch to a screen.
    Switch(Screen),
    /// Move selection up.
    SelectNext,
    /// Move selection down.
    SelectPrevious,
    /// Open the inspector.
    OpenDetails,
}
