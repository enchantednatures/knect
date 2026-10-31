#![allow(clippy::unwrap_used, clippy::expect_used)]

use knect_proto::graph::Topology;

#[test]
fn topology_schema_is_frozen() {
    let schema = schemars::schema_for!(Topology);
    let encoded = serde_json::to_string_pretty(&schema).unwrap();
    insta::assert_snapshot!("topology_schema", encoded);
}
