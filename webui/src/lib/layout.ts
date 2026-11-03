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
  const size = SIZES[category] ?? DEFAULT_NODE_SIZE;
  // Dagre mutates node labels with x/y. Returning the shared descriptor would
  // make every node of a category alias the same position.
  return { width: size.width, height: size.height };
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
  /**
   * Maximum shelf width used to pack disconnected components. Defaults to a
   * square root of the total component area so wide and tall graphs both stay
   * compact.
   */
  rowWidth?: number;
}

/** Gap between packed connected components. */
const COMPONENT_GAP = 96;

/** Components larger than this use a compact grid instead of dagre. */
const GRID_COMPONENT_THRESHOLD = 24;

/** Stable category order for the large-component grid. */
const CATEGORY_ORDER: readonly NodeCategory[] = [
  'k_service',
  'route',
  'configuration',
  'revision',
  'deployment',
  'replica_set',
  'pod',
  'pod_autoscaler',
  'service',
  'endpoints',
  'config_map',
  'secret',
  'broker',
  'trigger',
  'source',
  'sink_binding',
  'subscription',
  'channel',
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
  'namespace',
  'unknown',
];

const CATEGORY_RANK = new Map(CATEGORY_ORDER.map((category, index) => [category, index]));

interface LayoutComponent {
  key: string;
  nodes: ResourceNode[];
  edges: Edge[];
  positions: Record<string, NodePosition>;
  width: number;
  height: number;
}

/**
 * Computes top-left SvelteFlow positions for every node. Input order does not
 * matter: nodes and edges are sorted before they reach dagre.
 *
 * Dagre places every disconnected component in its own vertical band. On a
 * cluster with many isolated resources (pods, policies, backing services) that
 * produces a graph thousands of pixels tall and extremely long edges. Layout
 * each connected component independently, then pack those blocks into a
 * deterministic shelf.
 */
export function computeLayout(
  nodes: readonly ResourceNode[],
  edges: readonly Edge[],
  options: LayoutOptions = {},
): Record<string, NodePosition> {
  if (nodes.length === 0) return {};

  const orderedNodes = [...nodes].sort(compareNodes);
  const components = connectedComponents(orderedNodes, edges).map((component) =>
    layoutComponent(component, options),
  );
  const margin = options.margin ?? 32;
  const rowWidth = options.rowWidth ?? shelfWidth(components);
  const positions: Record<string, NodePosition> = {};

  const orderedComponents = [...components].sort((left, right) => {
    const height = right.height - left.height;
    if (height !== 0) return height;
    const width = right.width - left.width;
    if (width !== 0) return width;
    return left.key < right.key ? -1 : left.key > right.key ? 1 : 0;
  });

  let cursorX = margin;
  let cursorY = margin;
  let rowHeight = 0;
  for (const component of orderedComponents) {
    if (cursorX > margin && cursorX + component.width > margin + rowWidth) {
      cursorX = margin;
      cursorY += rowHeight + COMPONENT_GAP;
      rowHeight = 0;
    }
    for (const [key, position] of Object.entries(component.positions)) {
      positions[key] = {
        x: round(position.x + cursorX),
        y: round(position.y + cursorY),
      };
    }
    cursorX += component.width + COMPONENT_GAP;
    rowHeight = Math.max(rowHeight, component.height);
  }
  return positions;
}

function compareNodes(left: ResourceNode, right: ResourceNode): number {
  const leftKey = resourceKey(left.id);
  const rightKey = resourceKey(right.id);
  return leftKey < rightKey ? -1 : leftKey > rightKey ? 1 : 0;
}

function connectedComponents(nodes: readonly ResourceNode[], edges: readonly Edge[]): LayoutComponent[] {
  const byKey = new Map(nodes.map((node) => [resourceKey(node.id), node]));
  const adjacency = new Map<string, Set<string>>();
  for (const node of nodes) {
    adjacency.set(resourceKey(node.id), new Set());
  }
  for (const edge of edges) {
    const from = resourceKey(edge.from);
    const to = resourceKey(edge.to);
    if (from === to || !byKey.has(from) || !byKey.has(to)) continue;
    adjacency.get(from)!.add(to);
    adjacency.get(to)!.add(from);
  }

  const visited = new Set<string>();
  const components: LayoutComponent[] = [];
  for (const node of nodes) {
    const start = resourceKey(node.id);
    if (visited.has(start)) continue;

    const memberKeys: string[] = [];
    const stack = [start];
    visited.add(start);
    while (stack.length > 0) {
      const key = stack.pop()!;
      memberKeys.push(key);
      for (const neighbour of adjacency.get(key) ?? []) {
        if (!visited.has(neighbour)) {
          visited.add(neighbour);
          stack.push(neighbour);
        }
      }
    }

    memberKeys.sort();
    const members = new Set(memberKeys);
    const componentNodes = memberKeys.map((key) => byKey.get(key)!);
    const componentEdges = edges.filter(
      (edge) => members.has(resourceKey(edge.from)) && members.has(resourceKey(edge.to)),
    );
    components.push({
      key: memberKeys[0] ?? start,
      nodes: componentNodes,
      edges: componentEdges,
      positions: {},
      width: 0,
      height: 0,
    });
  }
  return components;
}

function layoutComponent(
  component: LayoutComponent,
  options: LayoutOptions,
): LayoutComponent {
  if (component.nodes.length > GRID_COMPONENT_THRESHOLD) {
    return layoutGrid(component, options);
  }

  const graph = new dagre.graphlib.Graph({ multigraph: true, directed: true });
  graph.setGraph({
    rankdir: 'LR',
    ranker: 'network-simplex',
    nodesep: options.nodeSep ?? 40,
    ranksep: options.rankSep ?? 64,
    marginx: 0,
    marginy: 0,
    acyclicer: 'greedy',
    align: 'UL',
  });
  graph.setDefaultEdgeLabel(() => ({}));

  for (const node of component.nodes) {
    graph.setNode(resourceKey(node.id), nodeSize(node.category));
  }
  const orderedEdges = [...component.edges].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  for (const edge of orderedEdges) {
    graph.setEdge(
      resourceKey(edge.from),
      resourceKey(edge.to),
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

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  const raw: Record<string, NodePosition> = {};
  for (const node of component.nodes) {
    const key = resourceKey(node.id);
    const laidOut = graph.node(key) as { x?: number; y?: number } | undefined;
    const size = nodeSize(node.category);
    if (!laidOut || typeof laidOut.x !== 'number' || typeof laidOut.y !== 'number') {
      raw[key] = { x: 0, y: 0 };
      continue;
    }
    raw[key] = {
      x: laidOut.x - size.width / 2,
      y: laidOut.y - size.height / 2,
    };
    minX = Math.min(minX, raw[key].x);
    minY = Math.min(minY, raw[key].y);
    maxX = Math.max(maxX, raw[key].x + size.width);
    maxY = Math.max(maxY, raw[key].y + size.height);
  }

  if (!Number.isFinite(minX)) {
    return { ...component, positions: raw, width: 0, height: 0 };
  }

  const positions: Record<string, NodePosition> = {};
  for (const [key, position] of Object.entries(raw)) {
    positions[key] = { x: position.x - minX, y: position.y - minY };
  }
  const width = maxX - minX;
  const height = maxY - minY;
  if (component.nodes.length >= 8 && height > width * 2.5) {
    // Dagre stacks many siblings (for example pods owned by one deployment)
    // into a single rank. Grid those components instead of producing a tall
    // ribbon of nodes.
    return layoutGrid(component, options);
  }
  return {
    ...component,
    positions,
    width,
    height,
  };
}

/**
 * Compact category-ordered grid for components that are too large for a
 * readable layered layout. It keeps the canvas bounded and cheap to compute;
 * users can narrow the namespace/category filters to get the flow layout back.
 */
function layoutGrid(component: LayoutComponent, options: LayoutOptions): LayoutComponent {
  const ordered = [...component.nodes].sort((left, right) => {
    const leftRank = CATEGORY_RANK.get(left.category) ?? CATEGORY_ORDER.length;
    const rightRank = CATEGORY_RANK.get(right.category) ?? CATEGORY_ORDER.length;
    if (leftRank !== rightRank) return leftRank - rightRank;
    return compareNodes(left, right);
  });

  const gap = options.nodeSep ?? 40;
  let widest = 0;
  let tallest = 0;
  for (const node of ordered) {
    const size = nodeSize(node.category);
    widest = Math.max(widest, size.width);
    tallest = Math.max(tallest, size.height);
  }
  const cellWidth = widest + gap;
  const cellHeight = tallest + gap;
  const columns = Math.max(1, Math.ceil(Math.sqrt(ordered.length * 1.6)));
  const rows = Math.ceil(ordered.length / columns);
  const positions: Record<string, NodePosition> = {};

  ordered.forEach((node, index) => {
    const column = index % columns;
    const row = Math.floor(index / columns);
    const size = nodeSize(node.category);
    positions[resourceKey(node.id)] = {
      x: round(column * cellWidth + (widest - size.width) / 2),
      y: round(row * cellHeight + (tallest - size.height) / 2),
    };
  });

  return {
    ...component,
    positions,
    width: columns === 0 ? 0 : (columns - 1) * cellWidth + widest,
    height: rows === 0 ? 0 : (rows - 1) * cellHeight + tallest,
  };
}

function shelfWidth(components: readonly LayoutComponent[]): number {
  const area = components.reduce(
    (total, component) => total + (component.width + COMPONENT_GAP) * (component.height + COMPONENT_GAP),
    0,
  );
  const widest = components.reduce((max, component) => Math.max(max, component.width), 0);
  return Math.max(widest, Math.ceil(Math.sqrt(Math.max(area, 1)) * 1.15));
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
