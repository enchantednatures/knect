#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use knect_core::actions::KubeActions;
use knect_core::kube::FakeReader;
use knect_proto::rpc::{
    FlaggerAction, FlaggerControlParams, SetTrafficSplitParams, TrafficSplit,
    ValidateManifestParams,
};
use kube::api::{DynamicObject, TypeMeta};
use serde_json::json;

pub const NS: &str = "default";
pub const SERVICE: &str = "hello";

pub fn object(api_version: &str, kind: &str, namespace: &str, name: &str) -> DynamicObject {
    DynamicObject {
        types: Some(TypeMeta {
            api_version: api_version.to_owned(),
            kind: kind.to_owned(),
        }),
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(namespace.to_owned()),
            uid: Some(format!("uid-{name}")),
            ..ObjectMeta::default()
        },
        data: json!({}),
    }
}

fn owned(mut object: DynamicObject, kind: &str, name: &str, uid: &str) -> DynamicObject {
    object.metadata.owner_references = Some(vec![OwnerReference {
        api_version: "serving.knative.dev/v1".to_owned(),
        kind: kind.to_owned(),
        name: name.to_owned(),
        uid: uid.to_owned(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    }]);
    object
}

pub fn service() -> DynamicObject {
    object("serving.knative.dev/v1", "Service", NS, SERVICE)
}

pub fn revision(name: &str) -> DynamicObject {
    owned(
        object("serving.knative.dev/v1", "Revision", NS, name),
        "Configuration",
        SERVICE,
        "uid-config-hello",
    )
}

fn foreign_revision(name: &str) -> DynamicObject {
    owned(
        object("serving.knative.dev/v1", "Revision", NS, name),
        "Configuration",
        "other",
        "uid-config-other",
    )
}

pub fn cluster() -> FakeReader {
    FakeReader::new()
        .with_object(service())
        .with_object(revision("hello-00001"))
        .with_object(revision("hello-00002"))
        .with_object(foreign_revision("other-00001"))
}

pub fn actions(reader: &FakeReader) -> KubeActions {
    KubeActions::new(Arc::new(reader.clone()))
}

pub fn traffic_params(splits: Vec<TrafficSplit>, dry_run: Option<bool>) -> SetTrafficSplitParams {
    SetTrafficSplitParams {
        namespace: NS.to_owned(),
        service: SERVICE.to_owned(),
        splits,
        dry_run,
    }
}

pub fn split(revision: &str, percent: u8, tag: Option<&str>) -> TrafficSplit {
    TrafficSplit {
        revision: revision.to_owned(),
        percent,
        tag: tag.map(ToOwned::to_owned),
    }
}

pub fn canary() -> DynamicObject {
    object("flagger.app/v1beta1", "Canary", NS, SERVICE)
}

pub fn flagger_params(action: FlaggerAction, dry_run: Option<bool>) -> FlaggerControlParams {
    FlaggerControlParams {
        namespace: NS.to_owned(),
        canary: SERVICE.to_owned(),
        action,
        dry_run,
    }
}

pub fn manifest(contents: &str) -> ValidateManifestParams {
    ValidateManifestParams {
        manifest: contents.to_owned(),
        namespace: None,
        default_kind: None,
    }
}

pub fn mutated_service() -> DynamicObject {
    let mut object = service();
    object.metadata.labels = Some(BTreeMap::from([("served".to_owned(), "true".to_owned())]));
    object.data = json!({
        "spec": {
            "traffic": [{"latestRevision": true, "percent": 100}]
        }
    });
    object
}
