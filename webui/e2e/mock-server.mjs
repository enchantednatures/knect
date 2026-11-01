#!/usr/bin/env node
// Deterministic fixture daemon for Playwright. It serves the real built bundle
// and speaks the frozen `/rpc` + `/events` contract; no production code path is
// replaced by mocks.

import { createReadStream } from 'node:fs';
import { mkdir, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { extname, resolve, sep } from 'node:path';

const PORT = Number(process.env.PORT ?? 7429);
const DIST = resolve(process.cwd(), 'dist');
const START = Date.now();

const id = (apiVersion, kind, namespace, name) => ({ api_version: apiVersion, kind, namespace, name });
const revisionId = id('serving.knative.dev/v1', 'Revision', 'default', 'hello-00001');
const kserviceId = id('serving.knative.dev/v1', 'Service', 'default', 'hello');
const brokerId = id('eventing.knative.dev/v1', 'Broker', 'default', 'orders');
const triggerId = id('eventing.knative.dev/v1', 'Trigger', 'default', 'orders-to-hello');
const backingId = id('apps/v1', 'Deployment', 'default', 'redis-master');
const storageId = id('v1', 'PersistentVolumeClaim', 'default', 'redis-data');
const policyId = id('kyverno.io/v1', 'ClusterPolicy', null, 'require-labels');

const stamp = '2026-10-31T12:00:00Z';
const later = '2026-10-31T12:05:00Z';

const node = (resourceId, category, health, summary, badges = [], labels = {}) => ({
  id: resourceId,
  category,
  name: resourceId.name,
  namespace: resourceId.namespace,
  health,
  summary,
  labels,
  annotations: {},
  badges,
  uid: null,
  first_seen: stamp,
  last_seen: later,
});

const nodes = [
  node(kserviceId, 'k_service', 'healthy', 'Knative Service serving hello.default.example.com', [
    { label: 'scale-to-zero', severity: 'info', tooltip: 'Replicas can drop to zero' },
  ], { app: 'hello' }),
  node(revisionId, 'revision', 'healthy', 'Revision hello-00001 ready=1', [
    { label: 'ready', severity: 'info', tooltip: null },
  ], { app: 'hello' }),
  node(brokerId, 'broker', 'healthy', 'Broker orders using MTChannelBasedBroker'),
  node(triggerId, 'trigger', 'degraded', 'Delivers order.created to hello', [
    { label: 'dead-letter', severity: 'medium', tooltip: 'Dead-letter sink configured' },
  ]),
  node(backingId, 'backing_service', 'healthy', 'Redis master backing service'),
  node(storageId, 'persistent_volume_claim', 'healthy', '20Gi ceph-rbd claim'),
  node(policyId, 'policy', 'degraded', 'Requires app.kubernetes.io/owner', [
    { label: 'warn', severity: 'medium', tooltip: null },
  ]),
];

const edge = (edgeId, from, to, kind, health = 'healthy', label = null, metadata = {}) => ({
  id: edgeId,
  from,
  to,
  kind,
  label,
  health,
  warnings: [],
  metadata,
});

const edges = [
  edge('edge-kservice-revision', kserviceId, revisionId, 'owns'),
  edge('edge-trigger-broker', triggerId, brokerId, 'subscribes', 'healthy', 'order.created', {
    traffic_percent: 100,
  }),
  edge('edge-trigger-kservice', triggerId, kserviceId, 'delivers_to', 'degraded', null, {
    traffic_percent: 100,
  }),
  edge('edge-revision-backing', revisionId, backingId, 'connects_to'),
  edge('edge-revision-storage', revisionId, storageId, 'mounts'),
  edge('edge-policy-revision', policyId, revisionId, 'protected_by', 'degraded'),
];

const counts = new Map();
for (const item of nodes) counts.set(item.category, (counts.get(item.category) ?? 0) + 1);

const topology = {
  schema_version: 1,
  context: 'fixture',
  generation: 42,
  nodes,
  edges,
  warnings: [],
  stats: {
    node_count: nodes.length,
    edge_count: edges.length,
    warning_count: 0,
    categories: [...counts.entries()]
      .map(([category, count]) => ({ category, count }))
      .sort((a, b) => a.category.localeCompare(b.category)),
    truncated: false,
  },
};

const status = {
  version: '0.1.0',
  schema_version: 1,
  context: 'fixture',
  generation: 42,
  watchers: [
    {
      gvk: { group: 'serving.knative.dev', version: 'v1', kind: 'Service' },
      resource_count: 1,
      healthy: true,
      last_event: later,
    },
  ],
  uptime_secs: Math.floor((Date.now() - START) / 1000),
  actions_enabled: false,
  tap_count: 0,
  started_at: stamp,
  cache_enabled: true,
  last_resync: later,
};

function detailsFor(resourceId) {
  const summary = nodes.find((item) => item.name === resourceId.name)?.summary ?? 'Fixture resource';
  return {
    id: resourceId,
    category: nodes.find((item) => item.name === resourceId.name)?.category ?? 'revision',
    name: resourceId.name,
    namespace: resourceId.namespace,
    health: nodes.find((item) => item.name === resourceId.name)?.health ?? 'healthy',
    summary,
    yaml: `apiVersion: ${resourceId.api_version}\nkind: ${resourceId.kind}\nmetadata:\n  name: ${resourceId.name}\n  namespace: ${resourceId.namespace ?? 'null'}\nspec:\n  timeoutSeconds: 300\n`,
    yaml_redacted: true,
    mutations: [
      {
        manager: 'knative-serving-controller',
        manager_kind: 'knative',
        operation: 'update',
        subresource: null,
        time: later,
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
        id: 'kyverno-require-labels',
        name: 'require-labels',
        namespace: null,
        kind: 'kyverno',
        resource: resourceId,
        severity: 'medium',
        result: 'warn',
        message: 'Resource is missing the owner label required by policy',
        observed_at: later,
        details: { rule: 'check-owner-label' },
      },
    ],
    links: [
      {
        source: resourceId,
        target: backingId,
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
        pvc: storageId,
        volume: 'redis-data',
        storage_class: 'ceph-rbd',
        access_modes: ['read_write_once'],
        capacity: '20Gi',
        phase: 'Bound',
        mounted_by: [resourceId],
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
}

const diff = {
  resource: revisionId,
  user_intent:
    'apiVersion: serving.knative.dev/v1\nkind: Revision\nmetadata:\n  name: hello-00001\nspec:\n  timeoutSeconds: 300\n',
  knative_defaults:
    'apiVersion: serving.knative.dev/v1\nkind: Revision\nmetadata:\n  name: hello-00001\nspec:\n  timeoutSeconds: 300\n  containers:\n    - image: ghcr.io/example/hello:1.0.0\n',
  policy_mutators:
    'apiVersion: serving.knative.dev/v1\nkind: Revision\nmetadata:\n  name: hello-00001\n  labels:\n    app.kubernetes.io/part-of: orders\nspec:\n  timeoutSeconds: 300\n',
  lines: [
    {
      line_number: 6,
      side: 'knative_defaults',
      owner: 'knative-serving-controller',
      text: '  containers:',
      changed: true,
      path: '/spec/containers',
    },
    {
      line_number: 7,
      side: 'policy_mutators',
      owner: 'kyverno',
      text: '    app.kubernetes.io/part-of: orders',
      changed: true,
      path: '/metadata/labels/app.kubernetes.io~1part-of',
    },
  ],
};

const taps = [];
let sequence = 100;
const sseClients = new Set();

function sseFrame(event) {
  return `id: ${event.seq}\nevent: ${event.kind}\ndata: ${JSON.stringify(event)}\n\n`;
}

function broadcast(kind, data) {
  sequence += 1;
  const event = { seq: sequence, ts: new Date().toISOString(), kind, data };
  for (const response of sseClients) response.write(sseFrame(event));
}

function sendJson(response, body, statusCode = 200) {
  const payload = JSON.stringify(body);
  response.writeHead(statusCode, {
    'content-type': 'application/json',
    'content-length': Buffer.byteLength(payload),
  });
  response.end(payload);
}

function rpcResult(request) {
  const { method, params } = request;
  switch (method) {
    case 'daemon.status':
      return { ok: status };
    case 'graph.get_topology':
      return { ok: topology };
    case 'graph.get_node_details': {
      const resourceId = params?.id;
      if (!resourceId) return { error: { code: -32602, message: 'id is required' } };
      return { ok: detailsFor(resourceId) };
    }
    case 'mutations.get_diff':
      return { ok: diff };
    case 'graph.refresh':
      return { ok: { generation: topology.generation } };
    case 'tap.list':
      return { ok: taps };
    case 'tap.start': {
      const target = params?.target;
      if (!target) return { error: { code: -32602, message: 'target is required' } };
      const handle = {
        id: `tap-${taps.length + 1}`,
        target,
        state: 'active',
        created: new Date().toISOString(),
        expires: new Date(Date.now() + 900_000).toISOString(),
        max_events: 100,
        callback_url: 'http://127.0.0.1:7421/tap/callback',
        filter: params?.filter ?? null,
      };
      taps.push(handle);
      setTimeout(() => {
        broadcast('tap.event', {
          id: `event-${sequence + 1}`,
          ce_id: `ce-${sequence + 1}`,
          ce_source: `/default/${target.name}`,
          ce_type: params?.filter?.attributes?.type ?? 'order.created',
          ce_subject: 'orders/42',
          ce_time: new Date().toISOString(),
          headers: { 'content-type': 'application/json' },
          payload: { orderId: 42, total: 19.99 },
          received_at: new Date().toISOString(),
        });
      }, 150);
      return { ok: handle };
    }
    case 'tap.stop': {
      const tapId = params?.tap_id;
      const index = taps.findIndex((tap) => tap.id === tapId);
      if (index >= 0) taps.splice(index, 1);
      return { ok: { removed: index >= 0 } };
    }
    case 'actions.set_traffic_split':
    case 'actions.flagger_control':
    case 'policy.validate_manifest':
      return { error: { code: -32006, message: 'actions are disabled in the fixture daemon' } };
    default:
      return { error: { code: -32601, message: `method not found: ${method}` } };
  }
}

async function serveStatic(pathname, response) {
  const safe = resolve(DIST, `.${decodeURIComponent(pathname)}`);
  if (!safe.startsWith(DIST + sep) && safe !== DIST) {
    response.writeHead(403);
    response.end('forbidden');
    return;
  }
  let file = safe;
  try {
    const info = await stat(file);
    if (info.isDirectory()) file = resolve(file, 'index.html');
  } catch {
    if (pathname.includes('.')) {
      response.writeHead(404);
      response.end('not found');
      return;
    }
    file = resolve(DIST, 'index.html');
  }
  try {
    await stat(file);
  } catch {
    response.writeHead(404);
    response.end('not found');
    return;
  }
  const type =
    {
      '.html': 'text/html; charset=utf-8',
      '.js': 'text/javascript; charset=utf-8',
      '.css': 'text/css; charset=utf-8',
      '.svg': 'image/svg+xml',
      '.json': 'application/json',
      '.woff2': 'font/woff2',
    }[extname(file)] ?? 'application/octet-stream';
  response.writeHead(200, {
    'content-type': type,
    'cache-control': pathname.includes('.') ? 'public, max-age=31536000, immutable' : 'no-cache',
    'x-content-type-options': 'nosniff',
  });
  createReadStream(file).pipe(response);
}

const server = createServer((request, response) => {
  const url = new URL(request.url ?? '/', `http://127.0.0.1:${PORT}`);

  if (url.pathname === '/healthz') {
    sendJson(response, { status: 'ok' });
    return;
  }

  if (url.pathname === '/events') {
    response.writeHead(200, {
      'content-type': 'text/event-stream',
      'cache-control': 'no-cache',
      connection: 'keep-alive',
      'x-accel-buffering': 'no',
    });
    sseClients.add(response);
    sequence += 1;
    response.write(
      sseFrame({
        seq: sequence,
        ts: new Date().toISOString(),
        kind: 'snapshot',
        data: topology,
      }),
    );
    const heartbeat = setInterval(() => {
      sequence += 1;
      response.write(
        sseFrame({
          seq: sequence,
          ts: new Date().toISOString(),
          kind: 'daemon.heartbeat',
          data: {
            uptime_secs: Math.floor((Date.now() - START) / 1000),
            generation: 42,
            node_count: nodes.length,
            edge_count: edges.length,
            ts: new Date().toISOString(),
          },
        }),
      );
    }, 5_000);
    request.on('close', () => {
      clearInterval(heartbeat);
      sseClients.delete(response);
    });
    return;
  }

  if (url.pathname === '/rpc' && request.method === 'POST') {
    let body = '';
    request.on('data', (chunk) => {
      body += chunk;
    });
    request.on('end', () => {
      let envelope;
      try {
        envelope = JSON.parse(body);
      } catch {
        sendJson(response, {
          jsonrpc: '2.0',
          id: null,
          error: { code: -32700, message: 'invalid JSON' },
        });
        return;
      }
      const result = rpcResult(envelope);
      if (result.error) {
        sendJson(response, {
          jsonrpc: '2.0',
          id: envelope.id,
          result: null,
          error: result.error,
        });
        return;
      }
      sendJson(response, {
        jsonrpc: '2.0',
        id: envelope.id,
        result: result.ok,
        error: null,
      });
    });
    return;
  }

  serveStatic(url.pathname === '/' ? '/index.html' : url.pathname, response);
});

await mkdir(resolve(process.cwd(), '../artifacts'), { recursive: true });
server.listen(PORT, '127.0.0.1', () => {
  console.log(`knect fixture WebUI listening on http://127.0.0.1:${PORT}`);
});
