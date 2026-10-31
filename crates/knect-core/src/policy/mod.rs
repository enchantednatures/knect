//! Policy evaluation boundary.
//!
//! Task 10 owns Istio/Flagger/Kyverno correlation. This module freezes the
//! extension point and re-exports the protocol policy types.

pub use knect_proto::policy::{
    PolicyKind, PolicyRecord, PolicyReport, PolicyResult, PolicySummary, Severity, ValidationResult,
};

use crate::correlate::CorrelationContext;
use crate::error::CoreError;

/// Evaluates policy findings for the current store.
pub trait PolicyEvaluator: Send + Sync {
    /// Returns aggregated policy findings.
    fn evaluate(&self, ctx: &CorrelationContext) -> Result<PolicyReport, CoreError>;
}

/// Compile-time proof that the extension point is object-safe.
#[must_use]
pub fn as_dyn(
    evaluator: std::sync::Arc<dyn PolicyEvaluator>,
) -> std::sync::Arc<dyn PolicyEvaluator> {
    evaluator
}
