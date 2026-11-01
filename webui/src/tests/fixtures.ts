import type {
  DaemonStatus,
  Edge,
  MutationDiff,
  NodeDetails,
  ResourceId,
  ResourceNode,
  Topology,
} from '../lib/types';

export const revisionId: ResourceId = {
  api_version: 'serving.knative.dev/v1',
  kind: 'Revision',
  namespace: 'default',
  name: 'hello-00001',
};

export const kserviceId: ResourceId = {
  api_version: 'serving.knative.dev/v1',
  kind: 'Service',
  namespace: 'default',
  name: 'hello',
};

export const triggerId: ResourceId = {
  api_version: 'eventing.knative.dev/v1',
  kind: 'Trigger',
  namespace: 'default',
  name: 'orders-to-hello',
};

export const brokerId: ResourceId = {
  api_version: 'eventing.knative.dev/v1',
  kind: 'Broker',
  namespace: 'default',
  name: 'orders',
};

export function node(overrides: Partial<ResourceNode> & { id: ResourceId }): ResourceNode {
  return {
    category: 'revision',
    name: overrides.id.name,
    namespace: overrides.id.namespace,
    health: 'healthy',
    summary: 'Fixture resource',
    labels: {},
    annotations: {},
    badges: [],
    uid: null,
    first_seen: '2026-10-31T12:00:00Z',
    last_seen: '2026-10-31T12:05:00Z',
    ...overrides,
  };
}

export const revision = node({
  id: revisionId,
  category: 'revision',
  summary: 'Revision hello-00001 ready=1',
  labels: { app: 'hello' },
  badges: [{ label: 'ready', severity: 'info', tooltip: null }],
});

export const kservice = node({
  id: kserviceId,
  category: 'k_service',
  summary: 'Knative Service serving hello.default.example.com',
  badges: [{ label: 'scale-to-zero', severity: 'info', tooltip: 'Replicas can drop to zero' }],
});

export const trigger = node({
  id: triggerId,
  category: 'trigger',
  health: 'degraded',
  summary: 'Delivers order.created to hello',
  badges: [{ label: 'dead-letter', severity: 'medium', tooltip: null }],
});

export const broker = node({
  id: brokerId,
  category: 'broker',
  summary: 'Broker orders using MTChannelBasedBroker',
});

export const backing = node({
  id: {
    api_version: 'apps/v1',
    kind: 'Deployment',
    namespace: 'default',
    name: 'redis-master',
  },
  category: 'backing_service',
  summary: 'Redis master',
});

export const storage = node({
  id: {
    api_version: 'v1',
    kind: 'PersistentVolumeClaim',
    namespace: 'default',
    name: 'redis-data',
  },
  category: 'persistent_volume_claim',
  summary: '20Gi ceph-rbd claim',
});

export const policy = node({
  id: {
    api_version: 'kyverno.io/v1',
    kind: 'ClusterPolicy',
    namespace: null,
    name: 'require-labels',
  },
  category: 'policy',
  health: 'degraded',
  summary: 'Requires app.kubernetes.io/owner',
  badges: [{ label: 'warn', severity: 'medium', tooltip: null }],
});

export const edges: Edge[] = [
  {
    id: 'edge-kservice-revision',
    from: kserviceId,
    to: revisionId,
    kind: 'owns',
    label: null,
    health: 'healthy',
    warnings: [],
    metadata: {},
  },
  {
    id: 'edge-trigger-broker',
    from: triggerId,
    to: brokerId,
    kind: 'subscribes',
    label: 'order.created',
    health: 'healthy',
    warnings: [],
    metadata: { traffic_percent: 100 },
  },
  {
    id: 'edge-trigger-kservice',
    from: triggerId,
    to: kserviceId,
    kind: 'delivers_to',
    label: null,
    health: 'degraded',
    warnings: [],
    metadata: { traffic_percent: 100 },
  },
  {
    id: 'edge-revision-backing',
    from: revisionId,
    to: backing.id,
    kind: 'connects_to',
    label: null,
    health: 'healthy',
    warnings: [],
    metadata: {},
  },
  {
    id: 'edge-revision-storage',
    from: revisionId,
    to: storage.id,
    kind: 'mounts',
    label: null,
    health: 'healthy',
    warnings: [],
    metadata: {},
  },
  {
    id: 'edge-policy-revision',
    from: policy.id,
    to: revisionId,
    kind: 'protected_by',
    label: null,
    health: 'degraded',
    warnings: [],
    metadata: {},
  },
];

export const topology: Topology = {
  schema_version: 1,
  context: 'fixture',
  generation: 42,
  nodes: [kservice, revision, broker, trigger, backing, storage, policy],
  edges,
  warnings: [],
  stats: {
    node_count: 7,
    edge_count: 6,
    warning_count: 0,
    categories: [],
    truncated: false,
  },
};

export const status: DaemonStatus = {
  version: '0.1.0',
  schema_version: 1,
  context: 'fixture',
  generation: 42,
  watchers: [],
  uptime_secs: 120,
  actions_enabled: false,
  tap_count: 0,
  started_at: '2026-10-31T12:03:00Z',
  cache_enabled: true,
  last_resync: '2026-10-31T12:04:00Z',
};

export const details: NodeDetails = {
  id: revisionId,
  category: 'revision',
  name: 'hello-00001',
  namespace: 'default',
  health: 'healthy',
  summary: 'Revision hello-00001 ready=1',
  yaml: 'apiVersion: serving.knative.dev/v1\nkind: Revision\nmetadata:\n  name: hello-00001\n',
  yaml_redacted: true,
  mutations: [
    {
      manager: 'knative-serving-controller',
      manager_kind: 'knative',
      operation: 'update',
      subresource: null,
      time: '2026-10-31T12:01:00Z',
      paths: [
        {
          path: '/spec/timeoutSeconds',
          yaml_line: 8,
          yaml_column: 3,
          value_preview: '300',
        },
      ],
    },
  ],
  policies: [
    {
      id: 'kyverno-require-labels-hello-00001',
      name: 'require-labels',
      namespace: null,
      kind: 'kyverno',
      resource: revisionId,
      severity: 'medium',
      result: 'warn',
      message: 'Revision is missing the owner label required by policy',
      observed_at: '2026-10-31T12:04:30Z',
      details: { rule: 'check-owner-label' },
    },
  ],
  links: [
    {
      source: revisionId,
      target: backing.id,
      via: 'secret',
      confidence: 0.86,
      evidence: [
        {
          kind: 'secret_key',
          description: 'Secret key referenced by envFrom',
          path: '/spec/containers/0/env/0/valueFrom/secretKeyRef/name',
          value_redacted: '[REDACTED]',
        },
      ],
      health: 'healthy',
    },
  ],
  storage: [
    {
      pvc: storage.id,
      volume: 'redis-data',
      storage_class: 'ceph-rbd',
      access_modes: ['read_write_once'],
      capacity: '20Gi',
      phase: 'Bound',
      mounted_by: [revisionId],
      provisioned_by: null,
      backend: null,
      warnings: [
        {
          code: 'rwo_multi_mount',
          message: 'ReadWriteOnce claim is mounted by more than one replica',
          severity: 'high',
          conflict: null,
        },
      ],
    },
  ],
  warnings: [],
  metrics: {
    cpu_millicores: 12.5,
    memory_bytes: 67108864,
    replicas: 1,
    ready_replicas: 1,
    requests_per_second: 23.4,
    error_rate: 0.001,
    p95_latency_ms: 41.2,
    window_secs: 300,
    source: 'kubernetes-metrics',
  },
  logs: [],
};

export const diff: MutationDiff = {
  resource: revisionId,
  user_intent: 'apiVersion: serving.knative.dev/v1\nkind: Revision\nmetadata:\n  name: hello-00001\n',
  knative_defaults:
    'apiVersion: serving.knative.dev/v1\nkind: Revision\nmetadata:\n  name: hello-00001\nspec:\n  timeoutSeconds: 300\n',
  policy_mutators:
    'apiVersion: serving.knative.dev/v1\nkind: Revision\nmetadata:\n  name: hello-00001\n  labels:\n    app.kubernetes.io/part-of: orders\n',
  lines: [
    {
      line_number: 6,
      side: 'knative_defaults',
      owner: 'knative-serving-controller',
      text: '  timeoutSeconds: 300',
      changed: true,
      path: '/spec/timeoutSeconds',
    },
    {
      line_number: 6,
      side: 'policy_mutators',
      owner: 'kyverno',
      text: '    app.kubernetes.io/part-of: orders',
      changed: true,
      path: '/metadata/labels/app.kubernetes.io~1part-of',
    },
  ],
};
