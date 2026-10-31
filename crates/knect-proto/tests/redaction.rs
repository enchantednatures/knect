#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;

use knect_proto::redaction::{REDACTED, SecretValue};

const SENTINEL: &str = "hunter2-knect-sentinel";

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/contracts")
}

#[test]
fn secret_value_debug_and_display_are_redacted() {
    let secret = SecretValue::new(SENTINEL);
    let debug = format!("{secret:?}");
    let display = format!("{secret}");
    assert!(!debug.contains(SENTINEL));
    assert!(!display.contains(SENTINEL));
    assert!(debug.contains(REDACTED));
    assert_eq!(display, REDACTED);
    assert_eq!(secret.expose(), SENTINEL);
    assert_eq!(secret.len(), SENTINEL.len());
    assert!(!secret.is_empty());
}

#[test]
fn contract_fixtures_contain_no_secret_sentinel() {
    for entry in fs::read_dir(contracts_dir()).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let contents = fs::read_to_string(&path).unwrap();
        assert!(
            !contents.contains(SENTINEL),
            "sentinel leaked into {}",
            path.display()
        );
    }
}
