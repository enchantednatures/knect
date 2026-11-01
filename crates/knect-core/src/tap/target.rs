//! Target/Broker extraction and filter merging for tap Triggers.

use std::collections::BTreeMap;

use knect_proto::tap::{TapFilter, TapTargetKind};
use kube::api::DynamicObject;
use serde_json::{Value, json};

use super::trigger::{BROKER_GROUP, BROKER_VERSION};
use crate::error::CoreError;

pub(crate) fn broker_reference(
    kind: TapTargetKind,
    target: &DynamicObject,
) -> Result<(Value, BTreeMap<String, String>), CoreError> {
    match kind {
        TapTargetKind::Broker => Ok((
            json!({
                "apiVersion": format!("{BROKER_GROUP}/{BROKER_VERSION}"),
                "kind": "Broker",
                "name": target.metadata.name.clone().unwrap_or_default(),
            }),
            BTreeMap::new(),
        )),
        TapTargetKind::Trigger => {
            let spec = target
                .data
                .get("spec")
                .ok_or_else(|| CoreError::Tap("tap target Trigger has no spec".to_owned()))?;
            let broker = spec
                .get("brokerRef")
                .ok_or_else(|| CoreError::Tap("tap target Trigger has no brokerRef".to_owned()))?;
            let name = broker.get("name").and_then(Value::as_str).ok_or_else(|| {
                CoreError::Tap("tap target Trigger brokerRef has no name".to_owned())
            })?;
            let api_version = broker
                .get("apiVersion")
                .and_then(Value::as_str)
                .unwrap_or(BROKER_GROUP);
            let kind = broker
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("Broker");
            let broker_ref = json!({
                "apiVersion": api_version,
                "kind": kind,
                "name": name,
            });
            Ok((broker_ref, target_filter(spec)?))
        }
        TapTargetKind::Unknown => Err(CoreError::Tap(
            "unsupported tap target kind; expected broker or trigger".to_owned(),
        )),
    }
}

fn target_filter(spec: &Value) -> Result<BTreeMap<String, String>, CoreError> {
    let Some(attributes) = spec
        .get("filter")
        .and_then(|filter| filter.get("attributes"))
        .and_then(Value::as_object)
    else {
        return Ok(BTreeMap::new());
    };
    let mut result = BTreeMap::new();
    for (key, value) in attributes {
        let Some(text) = value.as_str() else {
            return Err(CoreError::Tap(format!(
                "tap target Trigger filter attribute '{key}' is not a string"
            )));
        };
        result.insert(key.clone(), text.to_owned());
    }
    Ok(result)
}

pub(crate) fn merge_filters(
    mut base: BTreeMap<String, String>,
    requested: Option<&TapFilter>,
) -> Result<BTreeMap<String, String>, CoreError> {
    let Some(requested) = requested else {
        return Ok(base);
    };
    for (key, value) in &requested.attributes {
        if let Some(existing) = base.get(key)
            && existing != value
        {
            return Err(CoreError::Tap(format!(
                "tap filter attribute '{key}' conflicts with the target Trigger"
            )));
        }
        base.insert(key.clone(), value.clone());
    }
    Ok(base)
}
