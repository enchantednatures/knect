import { describe, expect, it } from 'vitest';
import {
  bounds,
  computeLayout,
  layoutSignature,
  mergePositions,
  nodeSize,
} from '../lib/layout';
import { resourceKey } from '../lib/types';
import { kserviceId, revisionId, topology } from './fixtures';

describe('graph layout', () => {
  it('is deterministic for the same topology', () => {
    const first = computeLayout(topology.nodes, topology.edges);
    const second = computeLayout(topology.nodes, topology.edges);
    expect(second).toEqual(first);
  });

  it('places sources left of targets in a left-to-right graph', () => {
    const positions = computeLayout(topology.nodes, topology.edges);
    const source = positions[resourceKey(kserviceId)];
    const target = positions[resourceKey(revisionId)];
    expect(source).toBeDefined();
    expect(target).toBeDefined();
    expect(source!.x).toBeLessThan(target!.x);
  });

  it('changes the structural signature when identity or edges change', () => {
    const base = layoutSignature(topology.nodes, topology.edges);
    const renamed = layoutSignature(
      topology.nodes.map((node) =>
        node.id.name === 'hello' ? { ...node, summary: 'different', labels: { x: 'y' } } : node,
      ),
      topology.edges,
    );
    const removed = layoutSignature(topology.nodes.slice(1), topology.edges);
    expect(renamed).toBe(base);
    expect(removed).not.toBe(base);
  });

  it('handles an empty graph and reports sane bounds', () => {
    expect(computeLayout([], [])).toEqual({});
    expect(bounds([], {})).toMatchObject({ width: 0, height: 0 });
  });

  it('reports the full canvas bounds using node sizes', () => {
    const positions = computeLayout(topology.nodes, topology.edges);
    const box = bounds(topology.nodes, positions);
    expect(box.width).toBeGreaterThan(nodeSize('k_service').width);
    expect(box.height).toBeGreaterThan(0);
  });

  it('preserves prior positions for surviving nodes', () => {
    const previous = { [resourceKey(revisionId)]: { x: 999, y: 111 } };
    const next = computeLayout(topology.nodes, topology.edges);
    const merged = mergePositions(previous, topology.nodes, next);
    expect(merged[resourceKey(revisionId)]).toEqual({ x: 999, y: 111 });
    expect(merged[resourceKey(kserviceId)]).toEqual(next[resourceKey(kserviceId)]);
  });
});
