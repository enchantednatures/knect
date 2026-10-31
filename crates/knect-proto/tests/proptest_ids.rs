#![allow(clippy::unwrap_used, clippy::expect_used)]

use knect_proto::ids::{FieldPath, Gvk, ResourceId};
use proptest::prelude::*;

proptest! {
    #[test]
    fn resource_id_json_roundtrips(
        api_version in ".*",
        kind in ".*",
        namespace in proptest::option::of(".*"),
        name in ".*",
    ) {
        let id = ResourceId::new(api_version, kind, namespace, name);
        let encoded = serde_json::to_value(&id).unwrap();
        let decoded: ResourceId = serde_json::from_value(encoded.clone()).unwrap();
        prop_assert_eq!(&decoded, &id);
        prop_assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded);
    }

    #[test]
    fn field_path_json_roundtrips(path in ".*") {
        let field_path = FieldPath::new(path);
        let encoded = serde_json::to_value(&field_path).unwrap();
        let decoded: FieldPath = serde_json::from_value(encoded.clone()).unwrap();
        prop_assert_eq!(&decoded, &field_path);
        prop_assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded);
    }

    #[test]
    fn gvk_json_roundtrips(group in ".*", version in ".*", kind in ".*") {
        let gvk = Gvk::new(group, version, kind);
        let encoded = serde_json::to_value(&gvk).unwrap();
        let decoded: Gvk = serde_json::from_value(encoded.clone()).unwrap();
        prop_assert_eq!(&decoded, &gvk);
        prop_assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded);
    }
}
