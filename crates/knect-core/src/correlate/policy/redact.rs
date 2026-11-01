//! Redaction and severity helpers for policy metadata.

use std::sync::OnceLock;

use knect_proto::graph::Severity;
use regex::Regex;

/// Redacts common `key=value` secret patterns from policy text.
///
/// The policy correlators only copy messages produced by Istio, Flagger,
/// Kyverno, and Kubernetes Events. A message can nevertheless echo a value that
/// looks like a credential, so every copied string passes through this filter.
#[must_use]
pub fn redact_text(value: &str) -> String {
    let Some(pattern) = secret_pattern() else {
        return value.to_owned();
    };
    pattern.replace_all(value, "${1}=<redacted>").into_owned()
}

fn secret_pattern() -> Option<&'static Regex> {
    static PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            Regex::new(
                r#"(?i)\b(password|passwd|token|secret|api[_-]?key|client[_-]?secret)\s*[:=]\s*("[^"]*"|'[^']*'|[^\s,;]+)"#,
            )
            .ok()
        })
        .as_ref()
}

/// Maps a Kyverno or policy-report result to severity.
#[must_use]
pub fn severity_from_result(result: &str, explicit: Option<&str>) -> Severity {
    explicit.and_then(severity_from_label).unwrap_or_else(|| {
        match result.to_ascii_lowercase().as_str() {
            "fail" | "error" => Severity::High,
            "warn" | "warning" => Severity::Medium,
            "skip" => Severity::Low,
            _ => Severity::Info,
        }
    })
}

/// Parses a severity label.
#[must_use]
pub fn severity_from_label(value: &str) -> Option<Severity> {
    match value.trim().to_ascii_lowercase().as_str() {
        "critical" => Some(Severity::Critical),
        "high" => Some(Severity::High),
        "medium" => Some(Severity::Medium),
        "low" => Some(Severity::Low),
        "info" | "informational" => Some(Severity::Info),
        _ => None,
    }
}

/// True for results that should surface as a non-fatal warning.
#[must_use]
pub fn is_failure(result: &str) -> bool {
    matches!(result.to_ascii_lowercase().as_str(), "fail" | "error")
}

/// Returns the stable lowercase name of a severity.
#[must_use]
pub const fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::High => "high",
        Severity::Medium => "medium",
        Severity::Low => "low",
        Severity::Info => "info",
        Severity::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_assignments_are_redacted() {
        assert_eq!(
            redact_text("login failed password=hunter2 token: abc"),
            "login failed password=<redacted> token=<redacted>"
        );
        assert_eq!(
            redact_text("no credential here"),
            "no credential here".to_owned()
        );
    }

    #[test]
    fn severity_prefers_explicit_labels() {
        assert_eq!(
            severity_from_result("fail", Some("critical")),
            Severity::Critical
        );
        assert_eq!(severity_from_result("fail", None), Severity::High);
        assert_eq!(severity_from_result("warn", None), Severity::Medium);
        assert_eq!(severity_from_result("skip", None), Severity::Low);
        assert_eq!(severity_from_result("pass", None), Severity::Info);
    }

    #[test]
    fn failure_results_are_classified() {
        assert!(is_failure("fail"));
        assert!(is_failure("ERROR"));
        assert!(!is_failure("warn"));
    }
}
