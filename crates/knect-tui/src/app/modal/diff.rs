//! Three-way mutation diff modal state.

use knect_proto::ResourceId;
use knect_proto::mutation::{DiffLine, MutationDiff, MutationDiffMode, MutationSource};

/// Every source pane in display order.
pub const SOURCES: [MutationSource; 3] = [
    MutationSource::UserIntent,
    MutationSource::KnativeDefaults,
    MutationSource::PolicyMutators,
];

/// Three-way diff modal.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffModal {
    /// Resource being attributed.
    pub resource: ResourceId,
    /// Requested rendering mode.
    pub mode: MutationDiffMode,
    /// Decoded diff.
    pub diff: Option<MutationDiff>,
    /// Request failure.
    pub error: Option<String>,
    /// True while the request is in flight.
    pub loading: bool,
    /// Shared vertical scroll offset.
    pub scroll: usize,
}

impl DiffModal {
    /// Creates a loading modal for a resource.
    #[must_use]
    pub fn loading(resource: ResourceId) -> Self {
        Self {
            resource,
            mode: MutationDiffMode::ThreeWay,
            diff: None,
            error: None,
            loading: true,
            scroll: 0,
        }
    }

    /// Longest pane length, used to clamp scrolling.
    #[must_use]
    pub fn max_lines(&self) -> usize {
        let Some(diff) = self.diff.as_ref() else {
            return 0;
        };
        SOURCES
            .iter()
            .map(|source| pane_line_count(diff, *source))
            .max()
            .unwrap_or(0)
    }
}

/// Returns pane lines for one diff source, sorted by line number.
#[must_use]
pub fn pane_lines(diff: &MutationDiff, source: MutationSource) -> Vec<&DiffLine> {
    let mut lines: Vec<&DiffLine> = diff
        .lines
        .iter()
        .filter(|line| line.side == source)
        .collect();
    lines.sort_by_key(|line| line.line_number);
    lines
}

/// Rendered document for one diff source.
#[must_use]
pub fn pane_document(diff: &MutationDiff, source: MutationSource) -> Option<&str> {
    match source {
        MutationSource::UserIntent => diff.user_intent.as_deref(),
        MutationSource::KnativeDefaults => diff.knative_defaults.as_deref(),
        MutationSource::PolicyMutators => diff.policy_mutators.as_deref(),
        MutationSource::Unknown => None,
    }
}

fn pane_line_count(diff: &MutationDiff, source: MutationSource) -> usize {
    let lines = pane_lines(diff, source).len();
    if lines > 0 {
        return lines;
    }
    pane_document(diff, source)
        .map(|document| document.lines().count())
        .unwrap_or(0)
}
