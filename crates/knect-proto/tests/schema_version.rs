use std::collections::HashSet;

use knect_proto::error::ERROR_CODE_NAMES;
use knect_proto::events::ALL_EVENT_NAMES;
use knect_proto::rpc::methods;

#[test]
fn schema_version_is_one() {
    assert_eq!(knect_proto::SCHEMA_VERSION, 1);
}

#[test]
fn method_names_are_unique_and_non_empty() {
    assert_eq!(methods::ALL.len(), 15);
    let unique: HashSet<&str> = methods::ALL.iter().copied().collect();
    assert_eq!(unique.len(), methods::ALL.len());
    for method in methods::ALL {
        assert!(!method.is_empty());
        assert!(method.contains('.'));
    }
}

#[test]
fn event_names_are_unique_and_frozen() {
    let expected = [
        "snapshot",
        "graph.patch",
        "node.updated",
        "policy.violation",
        "tap.event",
        "log.line",
        "action.result",
        "daemon.heartbeat",
        "daemon.error",
    ];
    assert_eq!(ALL_EVENT_NAMES, expected);
    let unique: HashSet<&str> = ALL_EVENT_NAMES.iter().copied().collect();
    assert_eq!(unique.len(), ALL_EVENT_NAMES.len());
    for event in ALL_EVENT_NAMES {
        assert!(!event.is_empty());
    }
}

#[test]
fn error_names_and_codes_are_unique_and_non_empty() {
    let unique_names: HashSet<&str> = ERROR_CODE_NAMES.iter().map(|(name, _)| *name).collect();
    assert_eq!(unique_names.len(), ERROR_CODE_NAMES.len());
    let unique_codes: HashSet<i32> = ERROR_CODE_NAMES.iter().map(|(_, code)| *code).collect();
    assert_eq!(unique_codes.len(), ERROR_CODE_NAMES.len());
    for (name, code) in ERROR_CODE_NAMES {
        assert!(!name.is_empty());
        assert!(*code < 0);
    }
    assert_eq!(
        ERROR_CODE_NAMES.len(),
        [
            "parse_error",
            "invalid_request",
            "method_not_found",
            "invalid_params",
            "internal_error",
            "not_found",
            "conflict",
            "kube_unavailable",
            "policy_blocked",
            "unsupported_gvk",
            "actions_disabled",
            "tap_limit",
            "tap_forbidden",
            "cache_corrupt",
            "redaction_violation",
            "timeout",
            "cancelled",
        ]
        .len()
    );
    assert!(ERROR_CODE_NAMES.contains(&("not_found", -32001)));
    assert!(ERROR_CODE_NAMES.contains(&("cancelled", -32012)));
}
