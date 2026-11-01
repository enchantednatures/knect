<script lang="ts">
  import {
    Background,
    BackgroundVariant,
    Controls,
    MiniMap,
    SvelteFlow,
    type Node,
    type NodeTypes,
  } from '@xyflow/svelte';
  import { store } from './store.svelte';
  import ViewportFitter from './ViewportFitter.svelte';
  import { buildFlowEdges, buildFlowNodes, policyCounts, type ResourceNodeData } from './flow';
  import { categoryColor } from './theme';
  import BackingServiceNode from './nodes/BackingServiceNode.svelte';
  import BrokerNode from './nodes/BrokerNode.svelte';
  import KServiceNode from './nodes/KServiceNode.svelte';
  import PolicyBadgeNode from './nodes/PolicyBadgeNode.svelte';
  import ResourceNode from './nodes/ResourceNode.svelte';
  import RevisionNode from './nodes/RevisionNode.svelte';
  import StorageNode from './nodes/StorageNode.svelte';
  import TriggerNode from './nodes/TriggerNode.svelte';
  import type { Edge } from './types';
  import { resourceKey } from './types';

  const nodeTypes = {
    kservice: KServiceNode,
    revision: RevisionNode,
    broker: BrokerNode,
    trigger: TriggerNode,
    backing_service: BackingServiceNode,
    storage: StorageNode,
    policy: PolicyBadgeNode,
    resource: ResourceNode,
  } as unknown as NodeTypes;

  const counts = $derived(policyCounts(store.realtime.policies));

  const flowNodes = $derived(
    buildFlowNodes(store.visibleNodes, store.positions, store.selectedKey, counts),
  );

  const liveEdgeIds = $derived.by(() => {
    const live = new Set<string>();
    if (store.tapHandles.length === 0) return live;
    for (const edge of store.visibleEdges) {
      if (!edge.metadata) continue;
      for (const tap of store.tapHandles) {
        if (tap.state !== 'active' && tap.state !== 'starting') continue;
        const target = tap.target;
        if (
          (edge.from.name === target.name && edge.from.namespace === target.namespace) ||
          (edge.to.name === target.name && edge.to.namespace === target.namespace)
        ) {
          live.add(edge.id);
        }
      }
    }
    return live;
  });

  const flowEdges = $derived(buildFlowEdges(store.visibleEdges, liveEdgeIds));

  function onNodeClick({ node }: { node: Node }): void {
    const data = node.data as ResourceNodeData;
    if (data?.node) store.selectNode(data.node);
  }

  function minimapColor(node: Node): string {
    const data = node.data as ResourceNodeData;
    return data?.node ? categoryColor(data.node.category) : 'var(--cat-generic)';
  }

  function edgeExists(edge: Edge): boolean {
    return Boolean(store.positions[resourceKey(edge.from)] && store.positions[resourceKey(edge.to)]);
  }
</script>

<div class="canvas" data-testid="graph-canvas">
  <SvelteFlow
    nodes={flowNodes}
    edges={flowEdges}
    {nodeTypes}
    fitView
    fitViewOptions={{ padding: 0.18, minZoom: 0.2, maxZoom: 1.1 }}
    minZoom={0.1}
    maxZoom={1.8}
    colorMode="dark"
    nodesDraggable
    nodesConnectable={false}
    elementsSelectable
    onnodeclick={onNodeClick}
    proOptions={{ hideAttribution: false }}
  >
    <ViewportFitter />
    <Background variant={BackgroundVariant.Dots} gap={22} size={1} />
    <Controls />
    <MiniMap
      pannable
      zoomable
      ariaLabel="Topology minimap"
      nodeColor={minimapColor}
      maskColor="rgb(9 12 17 / 0.72)"
      class="knect-minimap"
    />
  </SvelteFlow>
  {#if store.visibleEdges.length > 0 && !store.visibleEdges.some(edgeExists)}
    <p class="sr-only">Some graph edges reference nodes hidden by the current filter.</p>
  {/if}
</div>

{#if store.visibleNodes.length > 0}
  <div class="legend" aria-hidden="true">
    <span><span class="legend-swatch" style:background="var(--cat-kservice)"></span>service</span>
    <span><span class="legend-swatch" style:background="var(--cat-revision)"></span>revision</span>
    <span><span class="legend-swatch" style:background="var(--cat-broker)"></span>broker</span>
    <span><span class="legend-swatch" style:background="var(--cat-trigger)"></span>trigger</span>
    <span><span class="legend-swatch" style:background="var(--cat-backing)"></span>backing</span>
    <span><span class="legend-swatch" style:background="var(--cat-storage)"></span>storage</span>
  </div>
{/if}

<style>
  .canvas {
    position: absolute;
    inset: 0;
  }

  .canvas :global(.svelte-flow__minimap) {
    display: block;
  }

  .legend {
    position: absolute;
    left: 16px;
    bottom: 16px;
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: rgb(17 22 32 / 0.9);
    backdrop-filter: blur(6px);
    font-family: var(--font-data);
    font-size: 9.5px;
    color: var(--text-faint);
    pointer-events: none;
    z-index: 5;
  }

  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }

  .legend-swatch {
    width: 7px;
    height: 7px;
    border-radius: 2px;
  }

  @media (max-width: 767px) {
    .canvas :global(.svelte-flow__minimap) {
      display: none;
    }

    .legend {
      display: none;
    }
  }
</style>
