#![expect(
    clippy::result_large_err,
    reason = "RpcError is the frozen protocol error object; boxing it at every internal call site would obscure the contract"
)]

//! JSON-RPC dispatcher with every frozen method registered.
//!
//! Foundation behavior:
//! - `daemon.status`, `graph.get_topology`, `graph.refresh`, `config.get` work.
//! - Every later method validates params, then returns a typed
//!   `INTERNAL_ERROR` (or `ACTIONS_DISABLED` for write actions) that names the
//!   pending task. Unknown methods and malformed envelopes use the standard
//!   JSON-RPC codes.

use knect_proto::error::{
    ACTIONS_DISABLED, INTERNAL_ERROR, INVALID_PARAMS, INVALID_REQUEST, METHOD_NOT_FOUND,
};
use knect_proto::rpc::JSONRPC_VERSION;
use knect_proto::rpc::{
    DaemonStatusParams, EmptyParams, EventsSubscribeParams, FlaggerControlParams, GetDiffParams,
    GetNodeDetailsParams, GetTopologyParams, LogsStopParams, LogsSubscribeParams, RefreshParams,
    SetTrafficSplitParams, TapStartParams, TapStopParams, ValidateManifestParams, methods,
};
use knect_proto::{ErrorData, RpcError, RpcRequest, RpcResponse};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub mod daemon;
pub mod graph;

mod actions;
mod logs;
mod mutations;
mod tap;
mod validate;

use crate::app::AppState;

/// Dispatches protocol requests against shared daemon state.
#[derive(Clone)]
pub struct RpcDispatcher {
    state: AppState,
}

impl RpcDispatcher {
    /// Creates a dispatcher for the given state.
    #[must_use]
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Returns every registered method name in contract order.
    #[must_use]
    pub const fn method_names() -> &'static [&'static str] {
        methods::ALL
    }

    /// Dispatches one request.
    ///
    /// Returns `None` for notifications (a `null` request ID). Invalid
    /// envelopes always produce a response, even with a `null` ID, matching the
    /// JSON-RPC error rule.
    #[must_use]
    pub fn dispatch(&self, request: RpcRequest) -> Option<RpcResponse> {
        futures::executor::block_on(self.dispatch_async(request))
    }

    /// Dispatches one request without blocking the caller's async worker.
    #[must_use]
    pub async fn dispatch_async(&self, request: RpcRequest) -> Option<RpcResponse> {
        let id = request.id.clone();
        if request.jsonrpc != JSONRPC_VERSION {
            return Some(RpcResponse::failure(
                id,
                RpcError::new(
                    INVALID_REQUEST,
                    format!(
                        "jsonrpc must be \"{JSONRPC_VERSION}\", got \"{}\"",
                        request.jsonrpc
                    ),
                ),
            ));
        }
        let outcome = self.route(&request).await;
        if request.id.is_null() {
            return None;
        }
        Some(match outcome {
            Ok(result) => RpcResponse::success(id, result),
            Err(error) => RpcResponse::failure(id, error),
        })
    }

    async fn route(&self, request: &RpcRequest) -> Result<Value, RpcError> {
        match request.method.as_str() {
            methods::DAEMON_STATUS => {
                let _: DaemonStatusParams = params_or_default(request)?;
                encode(daemon::status(&self.state).await)
            }
            methods::GRAPH_GET_TOPOLOGY => {
                let params: GetTopologyParams = params_or_default(request)?;
                encode(graph::get_topology(&self.state, &params).await?)
            }
            methods::GRAPH_GET_NODE_DETAILS => {
                let params: GetNodeDetailsParams = required_params(request)?;
                encode(graph::get_node_details(&self.state, &params).await?)
            }
            methods::GRAPH_REFRESH => {
                let _: RefreshParams = params_or_default(request)?;
                encode(daemon::refresh(&self.state))
            }
            methods::MUTATIONS_GET_DIFF => {
                let params: GetDiffParams = required_params(request)?;
                encode(mutations::get_diff(&self.state, params).await?)
            }
            methods::ACTIONS_SET_TRAFFIC_SPLIT => {
                let params: SetTrafficSplitParams = required_params(request)?;
                self.actions_guard(&request.method)?;
                encode(actions::set_traffic_split(&self.state, params).await?)
            }
            methods::ACTIONS_FLAGGER_CONTROL => {
                let params: FlaggerControlParams = required_params(request)?;
                self.actions_guard(&request.method)?;
                encode(actions::flagger_control(&self.state, params).await?)
            }
            methods::POLICY_VALIDATE_MANIFEST => {
                let params: ValidateManifestParams = required_params(request)?;
                encode(validate::validate_manifest(&self.state, params).await?)
            }
            methods::TAP_START => {
                let params: TapStartParams = required_params(request)?;
                encode(tap::start(&self.state, params).await?)
            }
            methods::TAP_STOP => {
                let params: TapStopParams = required_params(request)?;
                encode(tap::stop(&self.state, params).await?)
            }
            methods::TAP_LIST => {
                let _: EmptyParams = params_or_default(request)?;
                encode(tap::list(&self.state).await?)
            }
            methods::LOGS_SUBSCRIBE => {
                let params: LogsSubscribeParams = required_params(request)?;
                encode(logs::subscribe(&self.state, params).await?)
            }
            methods::LOGS_STOP => {
                let params: LogsStopParams = required_params(request)?;
                encode(logs::stop(&self.state, params).await?)
            }
            methods::EVENTS_SUBSCRIBE => {
                let _: EventsSubscribeParams = params_or_default(request)?;
                Err(RpcError::with_data(
                    INTERNAL_ERROR,
                    "events.subscribe is not available on this transport",
                    ErrorData {
                        retryable: false,
                        details: Some(
                            "HTTP clients use GET /events; UDS JSONL subscriptions land with Task 5"
                                .to_owned(),
                        ),
                        resource: None,
                    },
                ))
            }
            methods::CONFIG_GET => {
                let _: EmptyParams = params_or_default(request)?;
                encode(self.state.config.safe())
            }
            other => Err(RpcError::new(
                METHOD_NOT_FOUND,
                format!("unknown method '{other}'"),
            )),
        }
    }

    fn actions_guard(&self, method: &str) -> Result<(), RpcError> {
        if !self.state.actions_enabled() {
            return Err(RpcError::with_data(
                ACTIONS_DISABLED,
                "guarded write actions are disabled",
                ErrorData {
                    retryable: false,
                    details: Some(format!(
                        "{method} requires daemon.allow_actions = true and actions.enabled = true"
                    )),
                    resource: None,
                },
            ));
        }
        Ok(())
    }
}

fn encode<T: Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|error| {
        RpcError::new(
            INTERNAL_ERROR,
            format!("cannot serialize method result: {error}"),
        )
    })
}

fn params_or_default<T: DeserializeOwned + Default>(request: &RpcRequest) -> Result<T, RpcError> {
    match request.params.as_ref() {
        None | Some(Value::Null) => Ok(T::default()),
        Some(value) => serde_json::from_value(value.clone()).map_err(invalid_params),
    }
}

fn required_params<T: DeserializeOwned>(request: &RpcRequest) -> Result<T, RpcError> {
    match request.params.as_ref() {
        None | Some(Value::Null) => Err(RpcError::new(
            INVALID_PARAMS,
            format!("method '{}' requires params", request.method),
        )),
        Some(value) => serde_json::from_value(value.clone()).map_err(invalid_params),
    }
}

fn invalid_params(error: serde_json::Error) -> RpcError {
    RpcError::new(INVALID_PARAMS, format!("invalid params: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VERSION;
    use knect_core::config::Config;
    use knect_proto::error::NOT_FOUND;
    use knect_proto::rpc::{DaemonStatus, RefreshResult};
    use serde_json::json;

    fn state() -> AppState {
        AppState::new(Config::default(), true)
    }

    fn request(method: &str, params: Option<Value>) -> RpcRequest {
        RpcRequest::new(1_i64, method, params)
    }

    #[test]
    fn every_method_is_registered() {
        let dispatcher = RpcDispatcher::new(state());
        for method in methods::ALL {
            let response = dispatcher
                .dispatch(request(method, None))
                .expect("registered methods reply");
            let code = response.error.as_ref().map(|error| error.code);
            assert_ne!(
                code,
                Some(METHOD_NOT_FOUND),
                "method {method} is not registered"
            );
            assert_ne!(code, Some(INVALID_REQUEST), "method {method} envelope");
        }
        assert_eq!(RpcDispatcher::method_names().len(), 15);
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let dispatcher = RpcDispatcher::new(state());
        let response = dispatcher
            .dispatch(request("bogus.method", None))
            .expect("response");
        assert_eq!(
            response.error.map(|error| error.code),
            Some(METHOD_NOT_FOUND)
        );
    }

    #[test]
    fn invalid_envelope_is_invalid_request() {
        let dispatcher = RpcDispatcher::new(state());
        let mut request = request(methods::DAEMON_STATUS, None);
        request.jsonrpc = "1.0".to_owned();
        let response = dispatcher.dispatch(request).expect("response");
        assert_eq!(
            response.error.map(|error| error.code),
            Some(INVALID_REQUEST)
        );
    }

    #[test]
    fn malformed_params_are_invalid_params() {
        let dispatcher = RpcDispatcher::new(state());
        let response = dispatcher
            .dispatch(request(
                methods::GRAPH_GET_NODE_DETAILS,
                Some(json!({"unexpected": true})),
            ))
            .expect("response");
        assert_eq!(response.error.map(|error| error.code), Some(INVALID_PARAMS));
    }

    #[test]
    fn daemon_status_reports_fixture_state() {
        let dispatcher = RpcDispatcher::new(state());
        let response = dispatcher
            .dispatch(request(methods::DAEMON_STATUS, Some(json!({}))))
            .expect("response");
        let status: DaemonStatus =
            serde_json::from_value(response.result.expect("result")).expect("status decodes");
        assert_eq!(status.context, "fixture");
        assert_eq!(status.generation, 0);
        assert_eq!(status.schema_version, knect_proto::SCHEMA_VERSION);
        assert_eq!(status.version, VERSION);
        assert!(!status.actions_enabled);
        assert!(status.watchers.is_empty());
        assert!(status.cache_enabled);
    }

    #[test]
    fn empty_topology_is_valid() {
        let dispatcher = RpcDispatcher::new(state());
        let response = dispatcher
            .dispatch(request(methods::GRAPH_GET_TOPOLOGY, None))
            .expect("response");
        let topology: knect_proto::Topology =
            serde_json::from_value(response.result.expect("result")).expect("topology decodes");
        assert_eq!(topology.schema_version, knect_proto::SCHEMA_VERSION);
        assert_eq!(topology.context, "fixture");
        assert!(topology.nodes.is_empty());
        assert!(topology.edges.is_empty());
        assert_eq!(topology.stats.node_count, 0);
        assert_eq!(topology.stats.edge_count, 0);
    }

    #[test]
    fn include_filters_sections() {
        let state = state();
        let mut topology = crate::app::empty_topology("fixture", 0);
        topology.stats.node_count = 3;
        topology.stats.edge_count = 2;
        state.set_topology(topology);
        let dispatcher = RpcDispatcher::new(state);
        let response = dispatcher
            .dispatch(request(
                methods::GRAPH_GET_TOPOLOGY,
                Some(json!({"include": ["stats"]})),
            ))
            .expect("response");
        let topology: knect_proto::Topology =
            serde_json::from_value(response.result.expect("result")).expect("topology decodes");
        assert!(topology.nodes.is_empty());
        assert!(topology.edges.is_empty());
        assert!(topology.warnings.is_empty());
        assert_eq!(topology.stats.node_count, 3);
    }

    #[test]
    fn wrong_context_is_not_found() {
        let dispatcher = RpcDispatcher::new(state());
        let response = dispatcher
            .dispatch(request(
                methods::GRAPH_GET_TOPOLOGY,
                Some(json!({"context": "kind-other"})),
            ))
            .expect("response");
        assert_eq!(response.error.map(|error| error.code), Some(NOT_FOUND));
    }

    #[test]
    fn refresh_signals_without_faking_a_rebuild() {
        let dispatcher = RpcDispatcher::new(state());
        let response = dispatcher
            .dispatch(request(methods::GRAPH_REFRESH, None))
            .expect("response");
        let refresh: RefreshResult =
            serde_json::from_value(response.result.expect("result")).expect("refresh decodes");
        assert_eq!(refresh.generation, dispatcher.state.generation());
        assert_eq!(dispatcher.state.generation(), 0);
    }

    #[test]
    fn config_get_returns_safe_config() {
        let dispatcher = RpcDispatcher::new(state());
        let response = dispatcher
            .dispatch(request(methods::CONFIG_GET, None))
            .expect("response");
        let config: knect_proto::rpc::SafeConfig =
            serde_json::from_value(response.result.expect("result")).expect("config decodes");
        assert_eq!(config.daemon.http_addr, "127.0.0.1:7420");
        assert_eq!(config.daemon.context, "fixture");
        assert!(!config.actions.enabled);
        let json = serde_json::to_string(&config).expect("serialize");
        assert!(!json.contains("kubeconfig"));
    }

    #[test]
    fn actions_are_disabled_by_default() {
        let dispatcher = RpcDispatcher::new(state());
        let response = dispatcher
            .dispatch(request(
                methods::ACTIONS_SET_TRAFFIC_SPLIT,
                Some(json!({
                    "namespace": "default",
                    "service": "hello",
                    "splits": []
                })),
            ))
            .expect("response");
        assert_eq!(
            response.error.map(|error| error.code),
            Some(ACTIONS_DISABLED)
        );
    }

    #[test]
    fn notifications_have_no_response() {
        let dispatcher = RpcDispatcher::new(state());
        let request = RpcRequest::notification(methods::DAEMON_STATUS, None);
        assert!(dispatcher.dispatch(request).is_none());
    }

    #[tokio::test]
    async fn node_details_returns_redacted_yaml_and_empty_sections() {
        use knect_proto::graph::{Health, NodeCategory};
        use knect_proto::{ResourceId, ResourceNode};
        use kube::api::{ApiResource, DynamicObject};

        let state = state();
        let object = DynamicObject::new(
            "hello",
            &ApiResource {
                group: "serving.knative.dev".to_owned(),
                version: "v1".to_owned(),
                api_version: "serving.knative.dev/v1".to_owned(),
                kind: "Service".to_owned(),
                plural: "services".to_owned(),
            },
        );
        let id = knect_core::store::resource_id(&object);
        state.store.write().await.upsert(object);
        let mut topology = crate::app::empty_topology("fixture", 1);
        topology.nodes.push(ResourceNode {
            id: id.clone(),
            category: NodeCategory::KService,
            name: "hello".to_owned(),
            namespace: Some("default".to_owned()),
            health: Health::Healthy,
            summary: "ready".to_owned(),
            labels: Default::default(),
            annotations: Default::default(),
            badges: Vec::new(),
            uid: None,
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
        });
        topology.stats.node_count = 1;
        state.set_topology(topology);
        let dispatcher = RpcDispatcher::new(state);

        let response = dispatcher
            .dispatch_async(request(
                methods::GRAPH_GET_NODE_DETAILS,
                Some(json!({"id": id, "include_yaml": true})),
            ))
            .await
            .expect("response");
        assert!(response.error.is_none());
        let details: knect_proto::graph::NodeDetails =
            serde_json::from_value(response.result.expect("result")).expect("details decode");
        assert!(details.yaml_redacted);
        assert!(
            details
                .yaml
                .as_deref()
                .is_some_and(|yaml| yaml.contains("kind: Service"))
        );
        assert!(details.links.is_empty());
        assert!(details.policies.is_empty());
        assert!(details.storage.is_empty());
        assert!(details.mutations.is_empty());

        let missing =
            ResourceId::namespaced("serving.knative.dev/v1", "Service", "default", "absent");
        let response = dispatcher
            .dispatch_async(request(
                methods::GRAPH_GET_NODE_DETAILS,
                Some(json!({"id": missing})),
            ))
            .await
            .expect("response");
        assert_eq!(response.error.map(|error| error.code), Some(NOT_FOUND));
    }

    #[test]
    fn mutations_get_diff_missing_resource_is_not_found() {
        let dispatcher = RpcDispatcher::new(state());
        let response = dispatcher
            .dispatch(request(
                methods::MUTATIONS_GET_DIFF,
                Some(json!({
                    "id": {
                        "api_version": "v1",
                        "kind": "Pod",
                        "namespace": "default",
                        "name": "web"
                    }
                })),
            ))
            .expect("response");
        let error = response.error.expect("error");
        assert_eq!(error.code, NOT_FOUND);
        assert!(error.message.contains("not present"));
    }
}
