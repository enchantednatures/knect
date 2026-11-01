// Pure filter/selection helpers shared by the filter rail and the canvas.

import { categoryMeta } from './theme';
import type { Edge, Health, NodeCategory, ResourceNode } from './types';
import { resourceKey } from './types';

export interface FilterState {
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
