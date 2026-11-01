import { describe, expect, it } from 'vitest';
import {
  defaultFilters,
  filterEdges,
  filterNodes,
  isFiltered,
  matchesSearch,
  namespaceOptions,
  selectNode,
} from '../lib/filters';
import { resourceKey } from '../lib/types';
import { broker, kservice, policy, revision, topology } from './fixtures';

describe('graph filters', () => {
  it('returns every node with default filters', () => {
    expect(filterNodes(topology.nodes, defaultFilters())).toHaveLength(topology.nodes.length);
    expect(isFiltered(defaultFilters())).toBe(false);
  });

  it('filters by search across names, kinds, labels, and summaries', () => {
    const filters = { ...defaultFilters(), search: 'hello-00001' };
    const result = filterNodes(topology.nodes, filters);
    expect(result.map((node) => node.name)).toEqual(['hello-00001']);
    expect(matchesSearch(revision, 'serving.knative.dev/v1')).toBe(true);
    expect(matchesSearch(kservice, 'scale-to-zero')).toBe(true);
    expect(matchesSearch(revision, 'not-present')).toBe(false);
  });

  it('filters by health and hides healthy nodes in problems-only mode', () => {
    const degraded = filterNodes(topology.nodes, { ...defaultFilters(), health: 'degraded' });
    expect(degraded.every((node) => node.health === 'degraded')).toBe(true);
    expect(degraded.some((node) => node.name === 'orders-to-hello')).toBe(true);

    const problems = filterNodes(topology.nodes, { ...defaultFilters(), problemsOnly: true });
    expect(problems.every((node) => node.health !== 'healthy')).toBe(true);
  });

  it('filters by namespace and hidden category', () => {
    const filters = {
      ...defaultFilters(),
      namespace: 'default',
      hiddenCategories: ['broker' as const],
    };
    const result = filterNodes(topology.nodes, filters);
    expect(result.some((node) => node.name === 'orders')).toBe(false);
    expect(result.every((node) => node.namespace === 'default')).toBe(true);
  });

  it('honours the group toggles', () => {
    const withoutPolicy = filterNodes(topology.nodes, {
      ...defaultFilters(),
      showPolicies: false,
    });
    expect(withoutPolicy.some((node) => node.name === policy.name)).toBe(false);

    const withoutBacking = filterNodes(topology.nodes, {
      ...defaultFilters(),
      showBacking: false,
      showStorage: false,
    });
    expect(withoutBacking.some((node) => node.name === 'redis-master')).toBe(false);
    expect(withoutBacking.some((node) => node.name === 'redis-data')).toBe(false);
  });

  it('reports active filters and namespace option counts', () => {
    expect(isFiltered({ ...defaultFilters(), search: 'x' })).toBe(true);
    expect(namespaceOptions(topology.nodes)).toEqual([{ value: 'default', label: 'default', count: 6 }]);
  });

  it('drops edges whose endpoints are filtered out', () => {
    const visible = filterNodes(topology.nodes, {
      ...defaultFilters(),
      hiddenCategories: ['broker' as const],
    });
    const edges = filterEdges(topology.edges, visible);
    expect(edges.some((edge) => edge.id === 'edge-trigger-broker')).toBe(false);
    expect(edges.some((edge) => edge.id === 'edge-kservice-revision')).toBe(true);
  });

  it('toggles node selection', () => {
    const key = resourceKey(broker.id);
    expect(selectNode(null, broker.id)).toBe(key);
    expect(selectNode(key, broker.id)).toBeNull();
  });
});
