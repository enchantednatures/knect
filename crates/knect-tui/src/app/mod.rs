//! Application state shell.
//!
//! Task 13 builds the event loop on top of these types. The shape is frozen so
//! later UI work does not have to rewrite the client or CLI boundary.

pub mod actions;
pub mod event;
pub mod keys;
pub mod state;

pub use actions::Action;
pub use event::UiEvent;
pub use state::{AppState, Screen};
