// Pure filter/selection helpers shared by the filter rail and the canvas.

import { categoryMeta } from './theme';
import type { Edge, Health, NodeCategory, ResourceNode } from './types';
import { resourceKey } from './types';

export type TopologyScope = 'knative' | 'all';

export interface FilterState {
  /** `knative` keeps Knative resources plus their connected dependencies. */
  scope: TopologyScope;
  search: string;
  hiddenCategories: NodeCategory[];
  health: Health | 'all';
  namespace: string;
  problemsOnly: boolean;
  showPolicies: boolean;
  showBacking: boolean;
  showStorage: boolean;
}

export function defaultFilters(): FilterState {
  return {
    scope: 'knative',
    search: '',
    hiddenCategories: [],
    health: 'all',
    namespace: '',
    problemsOnly: false,
    showPolicies: true,
    showBacking: true,
    showStorage: true,
  };
}

export function isFiltered(filters: FilterState): boolean {
  return (
    filters.search.trim() !== '' ||
    filters.hiddenCategories.length > 0 ||
    filters.health !== 'all' ||
    filters.namespace !== '' ||
    filters.problemsOnly ||
    !filters.showPolicies ||
    !filters.showBacking ||
    !filters.showStorage
  );
}

function categoryVisible(category: NodeCategory, filters: FilterState): boolean {
  if (filters.hiddenCategories.includes(category)) return false;
  const group = categoryMeta(category).group;
  if (group === 'policy' && !filters.showPolicies) return false;
  if (group === 'backing' && !filters.showBacking) return false;
  if (group === 'storage' && !filters.showStorage) return false;
  return true;
}

export function matchesSearch(node: ResourceNode, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (needle === '') return true;
  const haystack = [
    node.name,
    node.namespace ?? '',
    node.id.api_version,
    node.id.kind,
    node.summary,
    categoryMeta(node.category).label,
    ...Object.keys(node.labels),
    ...Object.values(node.labels),
    ...node.badges.map((badge) => badge.label),
  ]
    .join('\n')
    .toLowerCase();
  return haystack.includes(needle);
}

export function filterNodes(
  nodes: readonly ResourceNode[],
  filters: FilterState,
): ResourceNode[] {
  return nodes.filter((node) => {
    if (!categoryVisible(node.category, filters)) return false;
    if (filters.health !== 'all' && node.health !== filters.health) return false;
    if (filters.namespace !== '' && (node.namespace ?? '') !== filters.namespace) return false;
    if (filters.problemsOnly && node.health === 'healthy') return false;
    return matchesSearch(node, filters.search);
  });
}

type ScopeKind = 'primary' | 'dependency' | 'runtime';

const KNATIVE_PRIMARY: ReadonlySet<NodeCategory> = new Set<NodeCategory>([
  'k_service',
  'route',
  'configuration',
  'revision',
  'broker',
  'trigger',
  'source',
  'sink_binding',
  'subscription',
  'channel',
  'sink',
]);

const KNATIVE_DEPENDENCY: ReadonlySet<NodeCategory> = new Set<NodeCategory>([
  'event_type',
  'backing_service',
  'topic',
  'storage_class',
  'persistent_volume',
  'persistent_volume_claim',
  'ceph_pool',
  'policy',
  'canary',
  'virtual_service',
  'destination_rule',
  'peer_authentication',
  'authorization_policy',
  'config_map',
  'secret',
]);

const RUNTIME_DETAIL: ReadonlySet<NodeCategory> = new Set<NodeCategory>([
  'deployment',
  'replica_set',
  'pod',
  'pod_autoscaler',
  'service',
  'endpoints',
  'namespace',
  'unknown',
]);

function scopeKind(category: NodeCategory): ScopeKind | null {
  if (KNATIVE_PRIMARY.has(category)) return 'primary';
  if (KNATIVE_DEPENDENCY.has(category)) return 'dependency';
  if (RUNTIME_DETAIL.has(category)) return 'runtime';
  return null;
}

/**
 * Knative-first projection.
 *
 * Every filtered Knative Serving/Eventing node is a root. Outgoing edges pull
 * in their runtime chain and dependencies; incoming dependency edges pull in
 * policies/Istio/backing resources that point at a Knative resource. Runtime
 * objects are never pulled in from a shared dependency, so one ConfigMap
 * mounted by hundreds of unrelated pods does not drag the cluster back in.
 *
 * Search deliberately bypasses the scope so operators can find any resource by
 * name; the scope resumes when the search field is cleared.
 */
export function visibleTopologyNodes(
  nodes: readonly ResourceNode[],
  edges: readonly Edge[],
  filters: FilterState,
): ResourceNode[] {
  const allowed = new Set(filterNodes(nodes, filters).map((node) => resourceKey(node.id)));
  if (filters.scope === 'all' || filters.search.trim() !== '') {
    return nodes.filter((node) => allowed.has(resourceKey(node.id)));
  }

  const byKey = new Map(nodes.map((node) => [resourceKey(node.id), node]));
  const outgoing = new Map<string, string[]>();
  const incoming = new Map<string, string[]>();
  for (const edge of edges) {
    const from = resourceKey(edge.from);
    const to = resourceKey(edge.to);
    if (from === to || !byKey.has(from) || !byKey.has(to)) continue;
    const targets = outgoing.get(from);
    if (targets) targets.push(to);
    else outgoing.set(from, [to]);
    const sources = incoming.get(to);
    if (sources) sources.push(from);
    else incoming.set(to, [from]);
  }

  const included = new Map<string, ScopeKind>();
  const queue: string[] = [];
  for (const node of nodes) {
    const key = resourceKey(node.id);
    if (allowed.has(key) && KNATIVE_PRIMARY.has(node.category)) {
      included.set(key, 'primary');
      queue.push(key);
    }
  }

  for (let index = 0; index < queue.length; index += 1) {
    const key = queue[index]!;
    const kind = included.get(key);
    if (!kind) continue;

    for (const next of outgoing.get(key) ?? []) {
      if (!allowed.has(next) || included.has(next)) continue;
      const node = byKey.get(next);
      const nextKind = node ? scopeKind(node.category) : null;
      if (nextKind === null || !canFollow(kind, nextKind)) continue;
      included.set(next, nextKind);
      queue.push(next);
    }

    // Policies and mesh resources usually point *at* the workload. Event
    // producers can also point *at* a SinkBinding (the bound Deployment is the
    // subject), so include one runtime hop without expanding its own pod fan-out.
    for (const previous of incoming.get(key) ?? []) {
      if (!allowed.has(previous) || included.has(previous)) continue;
      const node = byKey.get(previous);
      if (!node) continue;
      const previousKind = scopeKind(node.category);
      if (previousKind === 'dependency') {
        included.set(previous, 'dependency');
        queue.push(previous);
      } else if (previousKind === 'runtime' && kind === 'primary') {
        included.set(previous, 'runtime');
      }
    }
  }

  return nodes.filter((node) => included.has(resourceKey(node.id)));
}

function canFollow(from: ScopeKind, to: ScopeKind): boolean {
  if (from === 'primary') return true;
  if (from === 'dependency') return to === 'primary' || to === 'dependency';
  return to === 'dependency' || to === 'runtime';
}

export function filterEdges(edges: readonly Edge[], visible: readonly ResourceNode[]): Edge[] {
  const live = new Set(visible.map((node) => resourceKey(node.id)));
  return edges.filter(
    (edge) => live.has(resourceKey(edge.from)) && live.has(resourceKey(edge.to)),
  );
}

export interface NamespaceOption {
  value: string;
  label: string;
  count: number;
}

export function namespaceOptions(nodes: readonly ResourceNode[]): NamespaceOption[] {
  const counts = new Map<string, number>();
  for (const node of nodes) {
    if (node.namespace === null) continue;
    counts.set(node.namespace, (counts.get(node.namespace) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([value, count]) => ({ value, label: value, count }))
    .sort((a, b) => a.value.localeCompare(b.value));
}

export function selectNode(
  current: string | null,
  id: ResourceNode['id'],
): string | null {
  const key = resourceKey(id);
  return current === key ? null : key;
}
