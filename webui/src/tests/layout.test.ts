import { describe, expect, it } from 'vitest';
import {
  bounds,
  computeLayout,
  layoutSignature,
  mergePositions,
  nodeSize,
} from '../lib/layout';
import type { Edge } from '../lib/types';
import { resourceKey } from '../lib/types';
import { kserviceId, node, revisionId, topology } from './fixtures';

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

  it('packs disconnected nodes into a shelf instead of one tall column', () => {
    const nodes = Array.from({ length: 40 }, (_, index) =>
      node({
        id: { ...revisionId, name: `revision-${index.toString().padStart(2, '0')}` },
        category: 'revision',
      }),
    );
    const positions = computeLayout(nodes, []);
    const box = bounds(nodes, positions);

    expect(box.width).toBeGreaterThan(box.height);
    expect(box.height).toBeLessThan(40 * nodeSize('revision').height);
    expect(new Set(Object.values(positions).map(({ x, y }) => `${x},${y}`)).size).toBe(40);
  });

  it('gives sibling nodes of the same category distinct positions', () => {
    const deployment = node({
      id: { ...revisionId, kind: 'Deployment', name: 'web' },
      category: 'deployment',
    });
    const firstPod = node({
      id: { ...revisionId, kind: 'Pod', name: 'web-1' },
      category: 'pod',
    });
    const secondPod = node({
      id: { ...revisionId, kind: 'Pod', name: 'web-2' },
      category: 'pod',
    });
    const edges: Edge[] = [
      {
        id: 'edge-deployment-pod-1',
        from: deployment.id,
        to: firstPod.id,
        kind: 'runs',
        label: null,
        health: 'healthy',
        warnings: [],
        metadata: {},
      },
      {
        id: 'edge-deployment-pod-2',
        from: deployment.id,
        to: secondPod.id,
        kind: 'runs',
        label: null,
        health: 'healthy',
        warnings: [],
        metadata: {},
      },
    ];
    const positions = computeLayout([deployment, firstPod, secondPod], edges);
    const distinct = new Set(
      Object.values(positions).map(({ x, y }) => `${x.toString()},${y.toString()}`),
    );

    expect(distinct.size).toBe(3);
  });

  it('grids a large connected component instead of stacking one rank vertically', () => {
    const nodes = Array.from({ length: 60 }, (_, index) =>
      node({
        id: { ...revisionId, name: `revision-${index.toString().padStart(2, '0')}` },
        category: 'revision',
      }),
    );
    const edges: Edge[] = nodes.slice(1).map((node, index) => ({
      id: `edge-${index}`,
      from: nodes[index]!.id,
      to: node.id,
      kind: 'owns',
      label: null,
      health: 'healthy',
      warnings: [],
      metadata: {},
    }));
    const positions = computeLayout(nodes, edges);
    const box = bounds(nodes, positions);

    expect(box.width).toBeGreaterThan(box.height);
    expect(box.height).toBeLessThan(20 * nodeSize('revision').height);
    expect(new Set(Object.values(positions).map(({ x, y }) => `${x},${y}`)).size).toBe(60);
  });
});
