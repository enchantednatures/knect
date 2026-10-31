//! `tui` scenario.
//!
//! Foundation skeleton. The owning task replaces this body with real
//! cluster/UI assertions.

use crate::harness::Harness;
use crate::scenarios::{ScenarioError, pending};

/// Runs the tui scenario skeleton.
pub async fn run(_harness: &Harness) -> Result<(), ScenarioError> {
    pending("tui");
    Ok(())
}
