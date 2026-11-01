// Deterministic, side-effect-free reducers for snapshots, graph patches, and
// every SSE payload. The runes store wraps these; tests drive them directly.

import type {
  ActionResult,
  CategoryCount,
  DaemonErrorEvent,
  Edge,
  GraphPatch,
  Heartbeat,
  LogLine,
  NodeCategory,
  NodeUpdated,
  PolicyRecord,
  PolicyViolation,
  ResourceNode,
  SseEvent,
  TapEvent,
  Topology,
  TopologyStats,
  Warning,
} from './types';
import { resourceKey } from './types';

export interface GraphState {
  schema_version: number;
  context: string;
  generation: number;
  nodes: ResourceNode[];
  edges: Edge[];
  warnings: Warning[];
  stats: TopologyStats;
}

export const SCHEMA_VERSION = 1;

export function emptyStats(): TopologyStats {
  return {
    node_count: 0,
    edge_count: 0,
    warning_count: 0,
    categories: [],
    truncated: false,
  };
}

export function emptyGraph(context = 'unknown', generation = 0): GraphState {
  return {
    schema_version: SCHEMA_VERSION,
    context,
    generation,
    nodes: [],
    edges: [],
    warnings: [],
    stats: emptyStats(),
  };
}

export function recomputeStats(
  nodes: readonly ResourceNode[],
  edges: readonly Edge[],
  warnings: readonly Warning[],
  truncated = false,
): TopologyStats {
  const counts = new Map<NodeCategory, number>();
  for (const node of nodes) {
    counts.set(node.category, (counts.get(node.category) ?? 0) + 1);
  }
  const categories: CategoryCount[] = [...counts.entries()]
    .map(([category, count]) => ({ category, count }))
    .sort((a, b) => (a.category < b.category ? -1 : a.category > b.category ? 1 : 0));
  return {
    node_count: nodes.length,
    edge_count: edges.length,
    warning_count: warnings.length,
    categories,
    truncated,
  };
}

function sortNodes(nodes: Iterable<ResourceNode>): ResourceNode[] {
  return [...nodes].sort((a, b) => {
    const left = resourceKey(a.id);
    const right = resourceKey(b.id);
    return left < right ? -1 : left > right ? 1 : 0;
  });
}

function sortEdges(edges: Iterable<Edge>): Edge[] {
  return [...edges].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

export function graphFromTopology(topology: Topology): GraphState {
  return {
    schema_version: topology.schema_version,
    context: topology.context,
    generation: topology.generation,
    nodes: sortNodes(topology.nodes),
    edges: sortEdges(topology.edges),
    warnings: [...topology.warnings],
    stats: recomputeStats(topology.nodes, topology.edges, topology.warnings, topology.stats.truncated),
  };
}

/**
 * Applies one `graph.patch`. Later patches win for the same identity, removed
 * identities are deleted, and edges that reference a removed node are dropped
 * so the canvas never points at a missing handle.
 */
export function applyGraphPatch(state: GraphState, patch: GraphPatch): GraphState {
  if (patch.generation < state.generation) {
    return state;
  }
  const removed = new Set(patch.removed_nodes.map(resourceKey));
  const nodes = new Map<string, ResourceNode>();
  for (const node of state.nodes) {
    if (!removed.has(resourceKey(node.id))) nodes.set(resourceKey(node.id), node);
  }
  for (const node of patch.added_nodes) nodes.set(resourceKey(node.id), node);
  for (const node of patch.updated_nodes) nodes.set(resourceKey(node.id), node);

  const removedEdges = new Set(patch.removed_edges);
  const edges = new Map<string, Edge>();
  for (const edge of state.edges) {
    if (!removedEdges.has(edge.id)) edges.set(edge.id, edge);
  }
  for (const edge of patch.added_edges) edges.set(edge.id, edge);

  const survivors = sortNodes(nodes.values());
  const liveKeys = new Set(survivors.map((node) => resourceKey(node.id)));
  const liveEdges = sortEdges(edges.values()).filter(
    (edge) => liveKeys.has(resourceKey(edge.from)) && liveKeys.has(resourceKey(edge.to)),
  );

  return {
    ...state,
    generation: Math.max(state.generation, patch.generation),
    nodes: survivors,
    edges: liveEdges,
    stats: recomputeStats(survivors, liveEdges, state.warnings, state.stats.truncated),
  };
}

export function applyNodeUpdated(state: GraphState, update: NodeUpdated): GraphState {
  const key = resourceKey(update.node.id);
  const nodes = new Map<string, ResourceNode>();
  for (const node of state.nodes) nodes.set(resourceKey(node.id), node);
  nodes.set(key, update.node);
  const survivors = sortNodes(nodes.values());
  return {
    ...state,
    generation: Math.max(state.generation, update.generation),
    nodes: survivors,
    stats: recomputeStats(survivors, state.edges, state.warnings, state.stats.truncated),
  };
}

export interface RealtimeState {
  graph: GraphState;
  policies: PolicyRecord[];
  tapEvents: TapEvent[];
  logs: LogLine[];
  actions: ActionResult[];
  heartbeat: Heartbeat | null;
  error: DaemonErrorEvent | null;
}

export const TAP_LIMIT = 200;
export const LOG_LIMIT = 500;
export const ACTION_LIMIT = 50;
export const POLICY_LIMIT = 200;

export function emptyRealtime(context = 'unknown'): RealtimeState {
  return {
    graph: emptyGraph(context),
    policies: [],
    tapEvents: [],
    logs: [],
    actions: [],
    heartbeat: null,
    error: null,
  };
}

function capped<T>(items: readonly T[], next: T, limit: number): T[] {
  return [...items, next].slice(-limit);
}

function upsertPolicy(policies: readonly PolicyRecord[], policy: PolicyRecord): PolicyRecord[] {
  const next = policies.filter((candidate) => candidate.id !== policy.id);
  next.push(policy);
  return next.slice(-POLICY_LIMIT);
}

/** Applies one SSE event. Unknown kinds are ignored by TypeScript exhaustiveness. */
export function applySseEvent(state: RealtimeState, event: SseEvent): RealtimeState {
  switch (event.kind) {
    case 'snapshot':
      return { ...state, graph: graphFromTopology(event.data) };
    case 'graph.patch':
      return { ...state, graph: applyGraphPatch(state.graph, event.data) };
    case 'node.updated':
      return { ...state, graph: applyNodeUpdated(state.graph, event.data) };
    case 'policy.violation':
      return { ...state, policies: upsertPolicy(state.policies, event.data.policy) };
    case 'tap.event':
      return { ...state, tapEvents: capped(state.tapEvents, event.data, TAP_LIMIT) };
    case 'log.line':
      return { ...state, logs: capped(state.logs, event.data, LOG_LIMIT) };
    case 'action.result':
      return { ...state, actions: capped(state.actions, event.data, ACTION_LIMIT) };
    case 'daemon.heartbeat':
      return { ...state, heartbeat: event.data };
    case 'daemon.error':
      return { ...state, error: event.data };
    default: {
      const exhaustive: never = event;
      return exhaustive;
    }
  }
}

/** Policies belonging to one resource, newest last. */
export function policiesFor(policies: readonly PolicyRecord[], key: string | null): PolicyRecord[] {
  if (key === null) return [];
  return policies.filter(
    (policy) => policy.resource !== null && resourceKey(policy.resource) === key,
  );
}

/** Policy findings that should render as node badges. */
export function violatedPolicies(policies: readonly PolicyRecord[]): PolicyRecord[] {
  return policies.filter((policy) => policy.result === 'fail' || policy.result === 'error');
}

export type PolicyViolationData = PolicyViolation;
