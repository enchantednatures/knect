// Deterministic left-to-right layered layout (dagre).
//
// `layoutSignature` changes only when the node/edge identity set changes. The
// store recomputes on that signature, so `node.updated` and heartbeat ticks
// never move a node the operator is looking at.

import dagre from 'dagre';
import { edgeHasTraffic } from './theme';
import type { Edge, NodeCategory, ResourceNode } from './types';
import { resourceKey } from './types';

export interface NodePosition {
  x: number;
  y: number;
}

export interface NodeSize {
  width: number;
  height: number;
}

const SIZES: Partial<Record<NodeCategory, NodeSize>> = {
  policy: { width: 168, height: 72 },
  persistent_volume_claim: { width: 190, height: 108 },
  persistent_volume: { width: 190, height: 108 },
  storage_class: { width: 190, height: 108 },
  ceph_pool: { width: 190, height: 108 },
  backing_service: { width: 204, height: 116 },
  topic: { width: 188, height: 100 },
  broker: { width: 204, height: 120 },
  trigger: { width: 204, height: 120 },
  revision: { width: 216, height: 124 },
  k_service: { width: 224, height: 128 },
};

export const DEFAULT_NODE_SIZE: NodeSize = { width: 216, height: 124 };

export function nodeSize(category: NodeCategory): NodeSize {
  return SIZES[category] ?? DEFAULT_NODE_SIZE;
}

export function layoutSignature(
  nodes: readonly Pick<ResourceNode, 'id' | 'category'>[],
  edges: readonly Pick<Edge, 'id' | 'from' | 'to'>[],
): string {
  const left = nodes
    .map((node) => `${resourceKey(node.id)}:${node.category}`)
    .sort()
    .join('|');
  const right = edges
    .map((edge) => `${edge.id}:${resourceKey(edge.from)}>${resourceKey(edge.to)}`)
    .sort()
    .join('|');
  return `${left}::${right}`;
}

export interface LayoutOptions {
  rankSep?: number;
  nodeSep?: number;
  margin?: number;
}

/**
 * Computes top-left SvelteFlow positions for every node. Input order does not
 * matter: nodes and edges are sorted before they reach dagre.
 */
export function computeLayout(
  nodes: readonly ResourceNode[],
  edges: readonly Edge[],
  options: LayoutOptions = {},
): Record<string, NodePosition> {
  if (nodes.length === 0) return {};

  const graph = new dagre.graphlib.Graph({ multigraph: true, directed: true });
  graph.setGraph({
    rankdir: 'LR',
    ranker: 'network-simplex',
    nodesep: options.nodeSep ?? 40,
    ranksep: options.rankSep ?? 64,
    marginx: options.margin ?? 32,
    marginy: options.margin ?? 32,
    acyclicer: 'greedy',
    align: 'UL',
  });
  graph.setDefaultEdgeLabel(() => ({}));

  const orderedNodes = [...nodes].sort((a, b) => {
    const left = resourceKey(a.id);
    const right = resourceKey(b.id);
    return left < right ? -1 : left > right ? 1 : 0;
  });
  const present = new Set<string>();
  for (const node of orderedNodes) {
    const key = resourceKey(node.id);
    present.add(key);
    graph.setNode(key, nodeSize(node.category));
  }

  const orderedEdges = [...edges].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  for (const edge of orderedEdges) {
    const from = resourceKey(edge.from);
    const to = resourceKey(edge.to);
    if (!present.has(from) || !present.has(to) || from === to) continue;
    graph.setEdge(
      from,
      to,
      {
        minlen: 1,
        weight: edgeHasTraffic(edge.kind, edge.metadata) ? 3 : 1,
        width: 0,
        height: 0,
      },
      edge.id,
    );
  }

  dagre.layout(graph);

  const positions: Record<string, NodePosition> = {};
  for (const node of orderedNodes) {
    const key = resourceKey(node.id);
    const laidOut = graph.node(key) as { x?: number; y?: number } | undefined;
    const size = nodeSize(node.category);
    if (!laidOut || typeof laidOut.x !== 'number' || typeof laidOut.y !== 'number') {
      positions[key] = { x: 0, y: 0 };
      continue;
    }
    positions[key] = {
      x: round(laidOut.x - size.width / 2),
      y: round(laidOut.y - size.height / 2),
    };
  }
  return positions;
}

function round(value: number): number {
  return Math.round(value * 2) / 2;
}

export interface LayoutBounds {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
  width: number;
  height: number;
}

export function bounds(
  nodes: readonly Pick<ResourceNode, 'id' | 'category'>[],
  positions: Record<string, NodePosition>,
): LayoutBounds {
  if (nodes.length === 0) {
    return { minX: 0, minY: 0, maxX: 0, maxY: 0, width: 0, height: 0 };
  }
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const node of nodes) {
    const position = positions[resourceKey(node.id)];
    if (!position) continue;
    const size = nodeSize(node.category);
    minX = Math.min(minX, position.x);
    minY = Math.min(minY, position.y);
    maxX = Math.max(maxX, position.x + size.width);
    maxY = Math.max(maxY, position.y + size.height);
  }
  if (!Number.isFinite(minX)) {
    return { minX: 0, minY: 0, maxX: 0, maxY: 0, width: 0, height: 0 };
  }
  return { minX, minY, maxX, maxY, width: maxX - minX, height: maxY - minY };
}

/** Keeps the previous position for nodes whose identity survived, then fills new nodes. */
export function mergePositions(
  previous: Record<string, NodePosition>,
  nodes: readonly Pick<ResourceNode, 'id'>[],
  next: Record<string, NodePosition>,
): Record<string, NodePosition> {
  const merged: Record<string, NodePosition> = {};
  for (const node of nodes) {
    const key = resourceKey(node.id);
    const prior = previous[key];
    merged[key] = prior ?? next[key] ?? { x: 0, y: 0 };
  }
  return merged;
}
