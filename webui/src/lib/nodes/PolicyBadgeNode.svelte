<script lang="ts">
  import { Handle, Position } from '@xyflow/svelte';
  import type { NodeProps } from '@xyflow/svelte';
  import type { ResourceNodeData } from '../flow';
  import Icon from '../Icon.svelte';
  import { categoryMeta, healthMeta, severityTone } from '../theme';

  let { data, selected }: NodeProps = $props();
  const typed = $derived(data as ResourceNodeData);
  const meta = $derived(categoryMeta(typed.node.category));
  const health = $derived(healthMeta(typed.node.health));
</script>

<div
  class="policy-node"
  class:policy-node--selected={selected}
  data-health={typed.node.health}
  data-testid="node-shell"
  data-node-name={typed.node.name}
>
  <Handle type="target" position={Position.Left} isConnectable={false} />
  <span class="policy-icon" aria-hidden="true"><Icon path={meta.icon} size={14} /></span>
  <span class="policy-name" title={typed.node.name}>{typed.node.name}</span>
  <span class="chip chip--{health.tone} policy-health">{health.label}</span>
  {#each typed.node.badges.slice(0, 1) as badge (badge.label)}
    <span class="chip chip--{severityTone(badge.severity)}">{badge.label}</span>
  {/each}
  <Handle type="source" position={Position.Right} isConnectable={false} />
</div>

<style>
  .policy-node {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    background: var(--surface);
    border: 1px solid rgb(251 113 133 / 0.4);
    border-radius: var(--radius-sm);
    box-shadow: var(--shadow-panel);
  }

  .policy-node--selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .policy-icon {
    color: var(--cat-policy);
    display: inline-flex;
  }

  .policy-name {
    font-size: 12px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .policy-health {
    margin-left: auto;
    font-size: 9px;
    min-height: 17px;
    padding: 0 5px;
  }
</style>
