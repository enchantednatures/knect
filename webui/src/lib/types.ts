// Wire types mirroring `crates/knect-proto`. Field names stay snake_case: they
// are the frozen JSON contract, not a local style choice.

export type NodeCategory =
  | 'k_service'
  | 'route'
  | 'configuration'
  | 'revision'
  | 'pod_autoscaler'
  | 'broker'
  | 'trigger'
  | 'source'
  | 'sink_binding'
  | 'subscription'
  | 'channel'
  | 'deployment'
  | 'replica_set'
  | 'pod'
  | 'service'
  | 'endpoints'
  | 'backing_service'
  | 'topic'
  | 'storage_class'
  | 'persistent_volume'
  | 'persistent_volume_claim'
  | 'ceph_pool'
  | 'policy'
  | 'canary'
  | 'virtual_service'
  | 'destination_rule'
  | 'peer_authentication'
  | 'authorization_policy'
  | 'config_map'
  | 'secret'
  | 'namespace'
  | 'unknown';

export type EdgeKind =
  | 'routes_to'
  | 'owns'
  | 'scales'
  | 'runs'
  | 'subscribes'
  | 'delivers_to'
  | 'dead_letter_sink'
  | 'uses_secret'
  | 'uses_config_map'
  | 'connects_to'
  | 'backed_by'
  | 'mounts'
  | 'provisioned_by'
  | 'protected_by'
  | 'mutated_by'
  | 'manages'
  | 'references'
  | 'targets'
  | 'published_by'
  | 'consumes_from'
  | 'selects'
  | 'unknown';

export type Health = 'healthy' | 'degraded' | 'progressing' | 'blocked' | 'unknown';
export type Severity = 'critical' | 'high' | 'medium' | 'low' | 'info' | 'unknown';

export interface Gvk {
  group: string;
  version: string;
  kind: string;
}

export interface ResourceId {
  api_version: string;
  kind: string;
  namespace: string | null;
  name: string;
}

export interface Badge {
  label: string;
  severity: Severity;
  tooltip: string | null;
}

export interface Warning {
  code: string;
  message: string;
  severity: Severity;
  resource: ResourceId | null;
}

export interface ResourceNode {
  id: ResourceId;
  category: NodeCategory;
  name: string;
  namespace: string | null;
  health: Health;
  summary: string;
  labels: Record<string, string>;
  annotations: Record<string, string>;
  badges: Badge[];
  uid: string | null;
  first_seen: string;
  last_seen: string;
}

export interface Edge {
  id: string;
  from: ResourceId;
  to: ResourceId;
  kind: EdgeKind;
  label: string | null;
  health: Health;
  warnings: Warning[];
  metadata: Record<string, unknown>;
}

export interface CategoryCount {
  category: NodeCategory;
  count: number;
}

export interface TopologyStats {
  node_count: number;
  edge_count: number;
  warning_count: number;
  categories: CategoryCount[];
  truncated: boolean;
}

export interface Topology {
  schema_version: number;
  context: string;
  generation: number;
  nodes: ResourceNode[];
  edges: Edge[];
  warnings: Warning[];
  stats: TopologyStats;
}

export interface GraphPatch {
  generation: number;
  added_nodes: ResourceNode[];
  updated_nodes: ResourceNode[];
  removed_nodes: ResourceId[];
  added_edges: Edge[];
  removed_edges: string[];
  seq: number;
  ts: string;
}

export interface NodeUpdated {
  node: ResourceNode;
  changed_fields: string[];
  generation: number;
  ts: string;
}

export interface PolicyRecord {
  id: string;
  name: string;
  namespace: string | null;
  kind: 'istio' | 'kyverno' | 'flagger' | 'other';
  resource: ResourceId | null;
  severity: Severity;
  result: 'pass' | 'fail' | 'warn' | 'error' | 'skip';
  message: string;
  observed_at: string | null;
  details: Record<string, string>;
}

export interface PolicyViolation {
  policy: PolicyRecord;
  generation: number;
  ts: string;
}

export interface ActionResult {
  action: string;
  success: boolean;
  message: string;
  dry_run: boolean;
  resource: ResourceId | null;
  generation: number | null;
  warnings: Warning[];
  ts: string;
}

export interface Heartbeat {
  uptime_secs: number;
  generation: number;
  node_count: number;
  edge_count: number;
  ts: string;
}

export interface DaemonErrorEvent {
  code: number;
  message: string;
  retryable: boolean;
  ts: string;
}

export type SseEvent = { seq: number; ts: string } & (
  | { kind: 'snapshot'; data: Topology }
  | { kind: 'graph.patch'; data: GraphPatch }
  | { kind: 'node.updated'; data: NodeUpdated }
  | { kind: 'policy.violation'; data: PolicyViolation }
  | { kind: 'tap.event'; data: TapEvent }
  | { kind: 'log.line'; data: LogLine }
  | { kind: 'action.result'; data: ActionResult }
  | { kind: 'daemon.heartbeat'; data: Heartbeat }
  | { kind: 'daemon.error'; data: DaemonErrorEvent }
);

export type SseEventName = SseEvent['kind'];

export const SSE_EVENT_NAMES = [
  'snapshot',
  'graph.patch',
  'node.updated',
  'policy.violation',
  'tap.event',
  'log.line',
  'action.result',
  'daemon.heartbeat',
  'daemon.error',
] as const satisfies readonly SseEventName[];

export interface FieldOwnership {
  path: string;
  yaml_line: number | null;
  yaml_column: number | null;
  value_preview: string | null;
}

export interface MutationRecord {
  manager: string;
  manager_kind:
    | 'kubectl'
    | 'controller'
    | 'knative'
    | 'istio'
    | 'flagger'
    | 'kyverno'
    | 'helm'
    | 'operator'
    | 'other';
  operation: 'apply' | 'update' | 'create' | 'delete' | 'other';
  subresource: string | null;
  time: string | null;
  paths: FieldOwnership[];
}

export type MutationSource = 'user_intent' | 'knative_defaults' | 'policy_mutators' | 'unknown';

export interface DiffLine {
  line_number: number;
  side: MutationSource;
  owner: string | null;
  text: string;
  changed: boolean;
  path: string | null;
}

export interface MutationDiff {
  resource: ResourceId;
  user_intent: string | null;
  knative_defaults: string | null;
  policy_mutators: string | null;
  lines: DiffLine[];
}

export interface Evidence {
  kind:
    | 'env_var'
    | 'env_from'
    | 'secret_key'
    | 'config_map_key'
    | 'dns_name'
    | 'uri'
    | 'port'
    | 'owner_reference'
    | 'field'
    | 'other';
  description: string;
  path: string | null;
  value_redacted: string | null;
}

export interface LinkRecord {
  source: ResourceId;
  target: ResourceId;
  via: 'explicit_binding' | 'env' | 'secret' | 'config_map' | 'dns' | 'operator_crd' | 'storage';
  confidence: number;
  evidence: Evidence[];
  health: Health;
}

export interface StorageConflictWarning {
  pvc: ResourceId;
  access_mode: string;
  max_scale: number | null;
  reason: string;
  affected: ResourceId[];
}

export interface StorageWarning {
  code: string;
  message: string;
  severity: Severity;
  conflict: StorageConflictWarning | null;
}

export interface StorageRecord {
  pvc: ResourceId;
  volume: string | null;
  storage_class: string | null;
  access_modes: string[];
  capacity: string | null;
  phase: string | null;
  mounted_by: ResourceId[];
  provisioned_by: ResourceId | null;
  backend: ResourceId | null;
  warnings: StorageWarning[];
}

export interface MetricsSummary {
  cpu_millicores: number | null;
  memory_bytes: number | null;
  replicas: number | null;
  ready_replicas: number | null;
  requests_per_second: number | null;
  error_rate: number | null;
  p95_latency_ms: number | null;
  window_secs: number | null;
  source: string | null;
}

export interface LogLine {
  namespace: string;
  pod: string;
  container: string;
  timestamp: string;
  message: string;
  stream: 'stdout' | 'stderr';
}

export interface NodeDetails {
  id: ResourceId;
  category: NodeCategory;
  name: string;
  namespace: string | null;
  health: Health;
  summary: string;
  yaml: string | null;
  yaml_redacted: boolean;
  mutations: MutationRecord[];
  policies: PolicyRecord[];
  links: LinkRecord[];
  storage: StorageRecord[];
  warnings: Warning[];
  metrics: MetricsSummary | null;
  logs: LogLine[];
}

export interface TapTarget {
  kind: 'broker' | 'trigger' | 'unknown';
  namespace: string;
  name: string;
}

export interface TapFilter {
  attributes: Record<string, string>;
}

export interface TapHandle {
  id: string;
  target: TapTarget;
  state: 'starting' | 'active' | 'stopping' | 'stopped' | 'failed' | 'expired' | 'unknown';
  created: string;
  expires: string;
  max_events: number;
  callback_url: string;
  filter: TapFilter | null;
}

export interface TapEvent {
  id: string;
  ce_id: string;
  ce_source: string;
  ce_type: string;
  ce_subject: string | null;
  ce_time: string | null;
  headers: Record<string, string>;
  payload: unknown;
  received_at: string;
}

export interface WatcherStatus {
  gvk: Gvk;
  resource_count: number;
  healthy: boolean;
  last_event: string | null;
}

export interface DaemonStatus {
  version: string;
  schema_version: number;
  context: string;
  generation: number;
  watchers: WatcherStatus[];
  uptime_secs: number;
  actions_enabled: boolean;
  tap_count: number;
  started_at: string | null;
  cache_enabled: boolean;
  last_resync: string | null;
}

export interface ValidationResult {
  valid: boolean;
  mutated_manifest: string | null;
  violations: PolicyRecord[];
  warnings: Warning[];
}

export interface RpcErrorPayload {
  code: number;
  message: string;
  data: {
    retryable: boolean;
    details: string | null;
    resource: ResourceId | null;
  } | null;
}

export type RpcId = number | string | null;

export interface RpcRequest {
  jsonrpc: '2.0';
  id: RpcId;
  method: string;
  params?: unknown;
}

export interface RpcResponse<T> {
  jsonrpc: string;
  id: RpcId;
  result?: T | null;
  error?: RpcErrorPayload | null;
}

export interface GetTopologyParams {
  context?: string;
  min_generation?: number;
  include?: ('nodes' | 'edges' | 'warnings' | 'stats')[];
}

export interface GetNodeDetailsParams {
  id: ResourceId;
  include_yaml?: boolean;
  include_logs?: boolean;
}

export interface GetDiffParams {
  id: ResourceId;
  mode?: 'three_way' | 'managers';
}

export interface TapStartParams {
  target: TapTarget;
  filter?: TapFilter;
  max_events?: number;
  ttl_secs?: number;
}

export interface TrafficSplit {
  revision: string;
  percent: number;
  tag?: string;
}

export interface SetTrafficSplitParams {
  namespace: string;
  service: string;
  splits: TrafficSplit[];
  dry_run?: boolean;
}

export interface FlaggerControlParams {
  namespace: string;
  canary: string;
  action: 'promote' | 'abort';
  dry_run?: boolean;
}

export interface ValidateManifestParams {
  manifest: string;
  namespace?: string;
  default_kind?: string;
}

/** Stable identity string for a `ResourceId`. Never use a plain object as a key. */
export function resourceKey(id: ResourceId): string {
  return [id.api_version, id.kind, id.namespace ?? '', id.name].join('\u0000');
}

/** Human-readable identity for labels and error messages. */
export function resourceLabel(id: ResourceId): string {
  const scope = id.namespace ? `${id.namespace}/` : '';
  return `${id.kind} ${scope}${id.name}`;
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
