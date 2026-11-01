//! Ephemeral Knative Trigger and Secret provisioning for taps.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use knect_proto::Gvk;
use knect_proto::tap::{TapFilter, TapTarget, TapTargetKind};
use kube::api::{ApiResource, DynamicObject};
use serde_json::json;

use super::config::TapManagerConfig;
use super::target::{broker_reference, merge_filters};
use crate::error::CoreError;
use crate::kube::ClusterReader;

/// Label marking objects owned by the daemon.
pub(crate) const MANAGED_BY_LABEL: &str = "knect.dev/managed-by";
/// Label carrying a tap ID.
pub(crate) const TAP_ID_LABEL: &str = "knect.dev/tap-id";
/// Label value marking daemon-managed objects.
pub(crate) const MANAGED_BY_VALUE: &str = "knect";

pub(super) const BROKER_GROUP: &str = "eventing.knative.dev";
pub(super) const BROKER_VERSION: &str = "v1";

/// Coordinates of the resources created for one tap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TriggerCoordinates {
    /// Namespace shared by the Trigger and its Secret.
    pub namespace: String,
    /// Trigger object name.
    pub trigger_name: String,
    /// Secret object name.
    pub secret_name: String,
}

/// Everything needed to create one tap Trigger.
pub(crate) struct TriggerRequest<'a> {
    /// Tap ID recorded on both objects.
    pub id: &'a str,
    /// Broker or Trigger being tapped.
    pub target: &'a TapTarget,
    /// Target namespace.
    pub namespace: &'a str,
    /// Trigger object name.
    pub trigger_name: &'a str,
    /// Secret object name.
    pub secret_name: &'a str,
    /// Callback bearer token stored in the Secret.
    pub token: &'a str,
    /// Token-guarded callback URL.
    pub callback_url: &'a str,
    /// Additional attribute filter.
    pub filter: Option<&'a TapFilter>,
}

/// Tap identity used to build the callback URL.
pub(crate) struct CallbackRef<'a> {
    /// Tap ID.
    pub id: &'a str,
    /// Callback token.
    pub token: &'a str,
}

/// Returns the discovered-shaped API resource for Knative Triggers.
#[must_use]
pub(crate) fn trigger_resource() -> ApiResource {
    ApiResource {
        group: BROKER_GROUP.to_owned(),
        version: BROKER_VERSION.to_owned(),
        api_version: format!("{BROKER_GROUP}/{BROKER_VERSION}"),
        kind: "Trigger".to_owned(),
        plural: "triggers".to_owned(),
    }
}

/// Returns the core API resource for Secrets.
#[must_use]
pub(crate) fn secret_resource() -> ApiResource {
    ApiResource {
        group: String::new(),
        version: "v1".to_owned(),
        api_version: "v1".to_owned(),
        kind: "Secret".to_owned(),
        plural: "secrets".to_owned(),
    }
}

/// Returns the GVK for a tap target kind.
pub(crate) fn target_gvk(kind: TapTargetKind) -> Result<Gvk, CoreError> {
    match kind {
        TapTargetKind::Broker => Ok(Gvk::new(BROKER_GROUP, BROKER_VERSION, "Broker")),
        TapTargetKind::Trigger => Ok(Gvk::new(BROKER_GROUP, BROKER_VERSION, "Trigger")),
        TapTargetKind::Unknown => Err(CoreError::Tap(
            "unsupported tap target kind; expected broker or trigger".to_owned(),
        )),
    }
}

/// Validates target coordinates before any object is created.
pub(crate) fn validate_target(target: &TapTarget) -> Result<(), CoreError> {
    if target.namespace.trim().is_empty() || target.name.trim().is_empty() {
        return Err(CoreError::Tap(
            "tap target namespace and name are required".to_owned(),
        ));
    }
    target_gvk(target.kind).map(|_| ())
}

/// Builds the advertised callback URL for a tap.
pub(crate) fn callback_url(
    config: &TapManagerConfig,
    addr: Option<SocketAddr>,
    tap: &CallbackRef<'_>,
) -> Result<String, CoreError> {
    let addr = addr
        .or_else(|| config.callback_bind.parse::<SocketAddr>().ok())
        .ok_or_else(|| {
            CoreError::Tap("tap callback listener has not bound an address".to_owned())
        })?;
    let host = advertised_host(&config.callback_host, addr);
    Ok(format!("http://{host}/tap/{}/{}", tap.id, tap.token))
}

fn detected_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("10.255.255.255:1").ok()?;
    let address = socket.local_addr().ok()?;
    (!address.ip().is_unspecified()).then(|| address.ip().to_string())
}

fn advertised_host(configured: &str, addr: SocketAddr) -> String {
    let configured = configured.trim();
    if configured.is_empty() {
        let host = if addr.ip().is_unspecified() {
            detected_local_ip().unwrap_or_else(|| "127.0.0.1".to_owned())
        } else {
            addr.ip().to_string()
        };
        return format!("{host}:{}", addr.port());
    }
    if configured.contains("://") {
        return configured.to_owned();
    }
    if configured
        .rsplit_once(':')
        .is_some_and(|(_, port)| port.parse::<u16>().is_ok())
    {
        return configured.to_owned();
    }
    format!("{configured}:{}", addr.port())
}

/// Creates the Secret and Trigger for one tap.
///
/// The Secret is deleted again when Trigger creation fails, so a failed start
/// never leaves half of the pair behind.
pub(crate) async fn create_trigger(
    reader: &std::sync::Arc<dyn ClusterReader>,
    request: TriggerRequest<'_>,
) -> Result<(), CoreError> {
    let gvk = target_gvk(request.target.kind)?;
    let target = reader
        .get(&gvk, Some(request.namespace), request.target.name.as_str())
        .await?;
    let (broker_ref, base_filter) = broker_reference(request.target.kind, &target)?;
    let merged = merge_filters(base_filter, request.filter)?;

    let mut secret = DynamicObject::new(request.secret_name, &secret_resource());
    secret.metadata.namespace = Some(request.namespace.to_owned());
    secret.metadata.labels = Some(object_labels(request.id));
    secret.data = json!({
        "type": "Opaque",
        "data": {
            "token": STANDARD.encode(request.token),
        },
    });
    reader.create(secret).await?;

    let mut trigger = DynamicObject::new(request.trigger_name, &trigger_resource());
    trigger.metadata.namespace = Some(request.namespace.to_owned());
    trigger.metadata.labels = Some(object_labels(request.id));
    trigger.metadata.annotations = Some(BTreeMap::from([(
        TAP_ID_LABEL.to_owned(),
        request.id.to_owned(),
    )]));
    let mut spec = json!({
        "brokerRef": broker_ref,
        "subscriber": {"uri": request.callback_url},
    });
    if !merged.is_empty()
        && let Some(object) = spec.as_object_mut()
    {
        object.insert("filter".to_owned(), json!({"attributes": merged}));
    }
    trigger.data = json!({ "spec": spec });

    if let Err(error) = reader.create(trigger).await {
        let coordinates = TriggerCoordinates {
            namespace: request.namespace.to_owned(),
            trigger_name: request.trigger_name.to_owned(),
            secret_name: request.secret_name.to_owned(),
        };
        let _ = delete_secret(reader, &coordinates).await;
        return Err(error);
    }
    Ok(())
}

/// Deletes both objects for a tap. Missing objects are treated as already gone.
pub(crate) async fn delete_trigger_resources(
    reader: &std::sync::Arc<dyn ClusterReader>,
    coordinates: &TriggerCoordinates,
) -> Result<(), CoreError> {
    let trigger_gvk = Gvk::new(BROKER_GROUP, BROKER_VERSION, "Trigger");
    let secret_gvk = Gvk::new("", "v1", "Secret");
    let mut first_error = None;
    if let Err(error) = reader
        .delete(
            &trigger_gvk,
            Some(coordinates.namespace.as_str()),
            coordinates.trigger_name.as_str(),
        )
        .await
    {
        first_error = Some(error);
    }
    if let Err(error) = reader
        .delete(
            &secret_gvk,
            Some(coordinates.namespace.as_str()),
            coordinates.secret_name.as_str(),
        )
        .await
        && first_error.is_none()
    {
        first_error = Some(error);
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn delete_secret(
    reader: &std::sync::Arc<dyn ClusterReader>,
    coordinates: &TriggerCoordinates,
) -> Result<(), CoreError> {
    reader
        .delete(
            &Gvk::new("", "v1", "Secret"),
            Some(coordinates.namespace.as_str()),
            coordinates.secret_name.as_str(),
        )
        .await
}

/// Builds the Secret name paired with a tap ID.
#[must_use]
pub(crate) fn secret_name_for(id: &str) -> String {
    format!("{id}-token")
}

fn object_labels(id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (MANAGED_BY_LABEL.to_owned(), MANAGED_BY_VALUE.to_owned()),
        (TAP_ID_LABEL.to_owned(), id.to_owned()),
    ])
}
