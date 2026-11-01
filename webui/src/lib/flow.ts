// Translates protocol graph nodes/edges into SvelteFlow objects. Kept pure so
// the mapping is unit-testable without mounting the canvas.

import { MarkerType, type Edge as FlowEdge, type Node as FlowNode } from '@xyflow/svelte';
import { nodeSize, type NodePosition } from './layout';
import { categoryMeta, edgeHasTraffic, edgeLabel, healthMeta } from './theme';
import type { Edge, NodeCategory, ResourceNode } from './types';
import { resourceKey } from './types';

export interface ResourceNodeData extends Record<string, unknown> {
  node: ResourceNode;
  policyCount: number;
  failingPolicies: number;
}

export type ResourceFlowNode = FlowNode<ResourceNodeData>;

export function nodeTypeFor(category: NodeCategory): string {
  switch (category) {
    case 'k_service':
      return 'kservice';
    case 'revision':
      return 'revision';
    case 'broker':
      return 'broker';
    case 'trigger':
      return 'trigger';
    case 'backing_service':
    case 'topic':
      return 'backing_service';
    case 'storage_class':
    case 'persistent_volume':
    case 'persistent_volume_claim':
    case 'ceph_pool':
      return 'storage';
    case 'policy':
    case 'peer_authentication':
    case 'authorization_policy':
      return 'policy';
    default:
      return 'resource';
  }
}

export function buildFlowNodes(
  nodes: readonly ResourceNode[],
  positions: Record<string, NodePosition>,
  selectedKey: string | null,
  policyCounts: ReadonlyMap<string, { total: number; failing: number }>,
): ResourceFlowNode[] {
  return nodes.map((node) => {
    const key = resourceKey(node.id);
    const counts = policyCounts.get(key) ?? { total: 0, failing: 0 };
    const size = nodeSize(node.category);
    return {
      id: key,
      type: nodeTypeFor(node.category),
      position: positions[key] ?? { x: 0, y: 0 },
      style: `width: ${size.width}px;`,
      selected: selectedKey === key,
      draggable: true,
      focusable: true,
      ariaRole: 'button',
      ariaLabel: `${categoryMeta(node.category).label} ${node.name}${
        node.namespace ? ` in ${node.namespace}` : ''
      }, ${healthMeta(node.health).label}`,
      domAttributes: {
        'data-node-key': key,
        'data-health': node.health,
      },
      data: { node, policyCount: counts.total, failingPolicies: counts.failing },
    } satisfies ResourceFlowNode;
  });
}

export interface EdgeFlowData extends Record<string, unknown> {
  kind: Edge['kind'];
  health: Edge['health'];
  live: boolean;
}

export function buildFlowEdges(
  edges: readonly Edge[],
  liveEdgeIds: ReadonlySet<string> = new Set(),
): FlowEdge<EdgeFlowData>[] {
  return edges.map((edge) => {
    const traffic = edgeHasTraffic(edge.kind, edge.metadata);
    const percent = edge.metadata['traffic_percent'];
    const trafficText = typeof percent === 'number' ? `${percent}%` : null;
    const label = trafficText ?? edge.label ?? (traffic ? edgeLabel(edge.kind) : null);
    const tone = healthMeta(edge.health).tone;
    const flowEdge: FlowEdge<EdgeFlowData> = {
      id: edge.id,
      source: resourceKey(edge.from),
      target: resourceKey(edge.to),
      type: 'default',
      animated: traffic || liveEdgeIds.has(edge.id),
      selectable: false,
      class: `knect-edge knect-edge--${tone}${liveEdgeIds.has(edge.id) ? ' knect-edge--tapped' : ''}`,
      markerEnd: {
        type: MarkerType.ArrowClosed,
        width: 14,
        height: 14,
        color: 'var(--edge-color, #4a5a70)',
      },
      data: {
        kind: edge.kind,
        health: edge.health,
        live: traffic || liveEdgeIds.has(edge.id),
      },
    };
    if (label !== null) flowEdge.label = label;
    return flowEdge;
  });
}

/** Maps a failing policy to the resource identity it should badge. */
export function policyCounts(
  policies: readonly { resource: { api_version: string; kind: string; namespace: string | null; name: string } | null; result: string }[],
): Map<string, { total: number; failing: number }> {
  const counts = new Map<string, { total: number; failing: number }>();
  for (const policy of policies) {
    if (!policy.resource) continue;
    const key = resourceKey(policy.resource);
    const current = counts.get(key) ?? { total: 0, failing: 0 };
    current.total += 1;
    if (policy.result === 'fail' || policy.result === 'error') current.failing += 1;
    counts.set(key, current);
  }
  return counts;
}
