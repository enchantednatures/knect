//! Knative traffic-split validation and merge-patch construction.

use std::collections::BTreeSet;

use knect_proto::events::ActionResult;
use knect_proto::rpc::{SetTrafficSplitParams, TrafficSplit};
use knect_proto::{Gvk, ResourceId};
use kube::api::DynamicObject;
use serde_json::{Map, Value, json};

use crate::error::CoreError;
use crate::kube::ClusterReader;

use super::executor::action_result;

const API_VERSION: &str = "serving.knative.dev/v1";
const SERVICE_KIND: &str = "Service";
const REVISION_KIND: &str = "Revision";
const CONFIGURATION_KIND: &str = "Configuration";

pub(super) async fn set_traffic_split(
    reader: &dyn ClusterReader,
    params: SetTrafficSplitParams,
) -> Result<ActionResult, CoreError> {
    validate(&params)?;

    let service_gvk = Gvk::new("serving.knative.dev", "v1", SERVICE_KIND);
    verify_revision_ownership(reader, &params).await?;

    let dry_run = params.dry_run.unwrap_or(false);
    reader
        .patch_merge(
            &service_gvk,
            &params.namespace,
            &params.service,
            patch_for(&params.splits),
            dry_run,
        )
        .await?;

    let resource = ResourceId::namespaced(
        API_VERSION,
        SERVICE_KIND,
        &params.namespace,
        &params.service,
    );
    let message = format!(
        "traffic split for {}/{}: {}",
        params.namespace,
        params.service,
        describe(&params.splits)
    );
    let message = dry_run_prefix(message, dry_run);
    Ok(action_result(
        "actions.set_traffic_split",
        message,
        dry_run,
        Some(resource),
    ))
}

fn validate(params: &SetTrafficSplitParams) -> Result<(), CoreError> {
    if params.namespace.trim().is_empty() {
        return Err(CoreError::Config("namespace must not be empty".to_owned()));
    }
    if params.service.trim().is_empty() {
        return Err(CoreError::Config("service must not be empty".to_owned()));
    }
    if params.splits.is_empty() {
        return Err(CoreError::Config(
            "traffic split must contain at least one revision".to_owned(),
        ));
    }

    let mut seen = BTreeSet::new();
    let mut total: u32 = 0;
    for split in &params.splits {
        if split.revision.trim().is_empty() {
            return Err(CoreError::Config(
                "revision name must not be empty".to_owned(),
            ));
        }
        if split.percent > 100 {
            return Err(CoreError::Config(format!(
                "revision {} has percent {}, which exceeds 100",
                split.revision, split.percent
            )));
        }
        if !seen.insert(split.revision.as_str()) {
            return Err(CoreError::Config(format!(
                "revision {} appears more than once in the traffic split",
                split.revision
            )));
        }
        total = total.saturating_add(u32::from(split.percent));
    }
    if total != 100 {
        return Err(CoreError::Config(format!(
            "traffic split percentages must sum to 100, got {total}"
        )));
    }
    Ok(())
}

async fn verify_revision_ownership(
    reader: &dyn ClusterReader,
    params: &SetTrafficSplitParams,
) -> Result<(), CoreError> {
    for split in &params.splits {
        let revision_gvk = Gvk::new("serving.knative.dev", "v1", REVISION_KIND);
        let revision = reader
            .get(&revision_gvk, Some(&params.namespace), &split.revision)
            .await?;
        if !revision_owned_by(&revision, &params.service) {
            return Err(CoreError::Config(format!(
                "revision {}/{} is not owned by service {}/{}",
                params.namespace, split.revision, params.namespace, params.service
            )));
        }
    }
    Ok(())
}

fn revision_owned_by(revision: &DynamicObject, service: &str) -> bool {
    let Some(owners) = revision.metadata.owner_references.as_deref() else {
        return false;
    };
    owners.iter().any(|owner| {
        owner.api_version == API_VERSION
            && owner.name == service
            && matches!(owner.kind.as_str(), SERVICE_KIND | CONFIGURATION_KIND)
    })
}

fn patch_for(splits: &[TrafficSplit]) -> Value {
    let traffic: Vec<Value> = splits
        .iter()
        .map(|split| {
            let mut target = Map::new();
            target.insert(
                "revisionName".to_owned(),
                Value::String(split.revision.clone()),
            );
            target.insert("percent".to_owned(), Value::from(split.percent));
            if let Some(tag) = split.tag.as_deref().filter(|tag| !tag.is_empty()) {
                target.insert("tag".to_owned(), Value::String(tag.to_owned()));
            }
            Value::Object(target)
        })
        .collect();
    json!({"spec": {"traffic": traffic}})
}

fn describe(splits: &[TrafficSplit]) -> String {
    splits
        .iter()
        .map(|split| format!("{}={}%", split.revision, split.percent))
        .collect::<Vec<_>>()
        .join(", ")
}

fn dry_run_prefix(message: String, dry_run: bool) -> String {
    if dry_run {
        format!("dry run: {message}")
    } else {
        message
    }
}
