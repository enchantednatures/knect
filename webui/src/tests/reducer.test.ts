import { describe, expect, it } from 'vitest';
import {
  applyGraphPatch,
  applyNodeUpdated,
  applySseEvent,
  emptyGraph,
  emptyRealtime,
  graphFromTopology,
  recomputeStats,
} from '../lib/reducer';
import type { GraphPatch, SseEvent } from '../lib/types';
import { broker, brokerId, kservice, revision, topology, trigger } from './fixtures';

const payments = {
  ...broker,
  id: { ...brokerId, name: 'payments' },
  name: 'payments',
  summary: 'Broker payments',
};

const patch: GraphPatch = {
  generation: 43,
  added_nodes: [payments],
  updated_nodes: [],
  removed_nodes: [],
  added_edges: [],
  removed_edges: [],
  seq: 43,
  ts: '2026-10-31T12:06:00Z',
};

describe('graph reducers', () => {
  it('replaces the graph from a snapshot and recomputes stats', () => {
    const state = graphFromTopology(topology);
    expect(state.generation).toBe(42);
    expect(state.nodes).toHaveLength(7);
    expect(state.stats.node_count).toBe(7);
    expect(state.stats.edge_count).toBe(6);
    expect(state.stats.categories.some((entry) => entry.category === 'k_service')).toBe(true);
  });

  it('applies additions and keeps deterministic ordering', () => {
    const state = applyGraphPatch(emptyGraph('fixture', 42), patch);
    expect(state.nodes.map((node) => node.name)).toContain('payments');
    expect(state.generation).toBe(43);
    const keys = state.nodes.map((node) => node.id.api_version + node.id.kind + node.id.name);
    expect(keys).toEqual([...keys].sort());
  });

  it('updates by identity, removes nodes, and drops dangling edges', () => {
    const base = graphFromTopology(topology);
    const updated = { ...revision, health: 'blocked' as const, summary: 'Blocked revision' };
    const next = applyGraphPatch(base, {
      generation: 44,
      added_nodes: [],
      updated_nodes: [updated],
      removed_nodes: [brokerId],
      added_edges: [],
      removed_edges: [],
      seq: 44,
      ts: '2026-10-31T12:07:00Z',
    });

    expect(next.nodes.find((node) => node.name === 'hello-00001')?.health).toBe('blocked');
    expect(next.nodes.some((node) => node.name === 'orders')).toBe(false);
    expect(next.edges.some((edge) => edge.id === 'edge-trigger-broker')).toBe(false);
    expect(next.stats.node_count).toBe(next.nodes.length);
    expect(next.stats.edge_count).toBe(next.edges.length);
  });

  it('ignores a stale patch generation', () => {
    const base = graphFromTopology(topology);
    const stale = applyGraphPatch(base, { ...patch, generation: 1 });
    expect(stale).toBe(base);
  });

  it('applies node.updated without touching other nodes', () => {
    const base = graphFromTopology(topology);
    const next = applyNodeUpdated(base, {
      node: { ...kservice, summary: 'Updated summary' },
      changed_fields: ['summary'],
      generation: 45,
      ts: '2026-10-31T12:08:00Z',
    });
    expect(next.generation).toBe(45);
    expect(next.nodes.find((node) => node.name === 'hello')?.summary).toBe('Updated summary');
    expect(next.nodes).toHaveLength(base.nodes.length);
  });

  it('recomputes category counts deterministically', () => {
    const stats = recomputeStats([trigger, kservice, broker], [], []);
    expect(stats.categories).toEqual([
      { category: 'broker', count: 1 },
      { category: 'k_service', count: 1 },
      { category: 'trigger', count: 1 },
    ]);
  });
});

describe('SSE reducer', () => {
  it('handles every non-graph event into the realtime state', () => {
    let state = emptyRealtime('fixture');
    const events: SseEvent[] = [
      {
        seq: 1,
        ts: '2026-10-31T12:00:00Z',
        kind: 'tap.event',
        data: {
          id: 'tap-1',
          ce_id: 'ce-1',
          ce_source: 'broker/orders',
          ce_type: 'order.created',
          ce_subject: 'orders/123',
          ce_time: null,
          headers: {},
          payload: { id: 123 },
          received_at: '2026-10-31T12:00:00Z',
        },
      },
      {
        seq: 2,
        ts: '2026-10-31T12:00:01Z',
        kind: 'log.line',
        data: {
          namespace: 'default',
          pod: 'hello-00001',
          container: 'user-container',
          timestamp: '2026-10-31T12:00:01Z',
          message: 'ready',
          stream: 'stdout',
        },
      },
      {
        seq: 3,
        ts: '2026-10-31T12:00:02Z',
        kind: 'action.result',
        data: {
          action: 'actions.set_traffic_split',
          success: true,
          message: 'applied',
          dry_run: false,
          resource: null,
          generation: 43,
          warnings: [],
          ts: '2026-10-31T12:00:02Z',
        },
      },
      {
        seq: 4,
        ts: '2026-10-31T12:00:03Z',
        kind: 'policy.violation',
        data: {
          policy: {
            id: 'policy-1',
            name: 'require-labels',
            namespace: null,
            kind: 'kyverno',
            resource: revision.id,
            severity: 'medium',
            result: 'warn',
            message: 'missing label',
            observed_at: null,
            details: {},
          },
          generation: 43,
          ts: '2026-10-31T12:00:03Z',
        },
      },
      {
        seq: 5,
        ts: '2026-10-31T12:00:04Z',
        kind: 'daemon.heartbeat',
        data: {
          uptime_secs: 10,
          generation: 43,
          node_count: 7,
          edge_count: 6,
          ts: '2026-10-31T12:00:04Z',
        },
      },
      {
        seq: 6,
        ts: '2026-10-31T12:00:05Z',
        kind: 'daemon.error',
        data: {
          code: -32003,
          message: 'kube unavailable',
          retryable: true,
          ts: '2026-10-31T12:00:05Z',
        },
      },
    ];
    for (const event of events) state = applySseEvent(state, event);

    expect(state.tapEvents).toHaveLength(1);
    expect(state.logs).toHaveLength(1);
    expect(state.actions).toHaveLength(1);
    expect(state.policies).toHaveLength(1);
    expect(state.heartbeat?.generation).toBe(43);
    expect(state.error?.retryable).toBe(true);
  });

  it('applies snapshot and patch events through the same entrypoint', () => {
    const snapshot: SseEvent = {
      seq: 1,
      ts: '2026-10-31T12:00:00Z',
      kind: 'snapshot',
      data: topology,
    };
    const patchEvent: SseEvent = {
      seq: 2,
      ts: '2026-10-31T12:00:01Z',
      kind: 'graph.patch',
      data: patch,
    };
    let state = applySseEvent(emptyRealtime(), snapshot);
    state = applySseEvent(state, patchEvent);
    expect(state.graph.nodes).toHaveLength(8);
    expect(state.graph.generation).toBe(43);
  });

  it('upserts policy findings by id', () => {
    const base = emptyRealtime();
    const policy = {
      id: 'p1',
      name: 'require-labels',
      namespace: null,
      kind: 'kyverno' as const,
      resource: revision.id,
      severity: 'medium' as const,
      result: 'warn' as const,
      message: 'first',
      observed_at: null,
      details: {},
    };
    let state = applySseEvent(base, {
      seq: 1,
      ts: '2026-10-31T12:00:00Z',
      kind: 'policy.violation',
      data: { policy, generation: 1, ts: '2026-10-31T12:00:00Z' },
    });
    state = applySseEvent(state, {
      seq: 2,
      ts: '2026-10-31T12:00:01Z',
      kind: 'policy.violation',
      data: {
        policy: { ...policy, message: 'second' },
        generation: 2,
        ts: '2026-10-31T12:00:01Z',
      },
    });
    expect(state.policies).toHaveLength(1);
    expect(state.policies[0]?.message).toBe('second');
  });
});
