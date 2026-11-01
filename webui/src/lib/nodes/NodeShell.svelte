<script lang="ts">
  import { Handle, Position } from '@xyflow/svelte';
  import Icon from '../Icon.svelte';
  import { categoryMeta, formatAge, healthMeta, severityTone } from '../theme';
  import type { ResourceNode } from '../types';

  let {
    node,
    selected = false,
    policyCount = 0,
    failingPolicies = 0,
    label,
    compact = false,
    showHandles = true,
  }: {
    node: ResourceNode;
    selected?: boolean;
    policyCount?: number;
    failingPolicies?: number;
    label?: string;
    compact?: boolean;
    /** Handles require a SvelteFlow node context; component tests disable them. */
    showHandles?: boolean;
  } = $props();

  const meta = $derived(categoryMeta(node.category));
  const health = $derived(healthMeta(node.health));
  const title = $derived(label ?? meta.label);
  const shownBadges = $derived(node.badges.slice(0, compact ? 1 : 3));
  const extraBadges = $derived(Math.max(0, node.badges.length - shownBadges.length));
</script>

<div
  class="node-shell cat-{meta.group}"
  class:node-shell--selected={selected}
  class:node-shell--compact={compact}
  data-health={node.health}
  data-testid="node-shell"
  data-node-name={node.name}
>
  {#if showHandles}
    <Handle type="target" position={Position.Left} isConnectable={false} />
  {/if}
  <div class="node-rail" aria-hidden="true"></div>
  <div class="node-body">
    <div class="node-head">
      <span class="node-icon" aria-hidden="true"><Icon path={meta.icon} size={15} /></span>
      <span class="node-kind">{title}</span>
      <span class="chip chip--{health.tone} node-health">
        <span class="dot" aria-hidden="true"></span>{health.label}
      </span>
    </div>
    <div class="node-name" title={node.name}>{node.name}</div>
    {#if !compact}
      <div class="node-scope mono">
        {node.namespace ?? 'cluster'}
        <span class="faint">·</span>
        {formatAge(node.last_seen)}
      </div>
      <p class="node-summary">{node.summary || 'No summary reported'}</p>
    {/if}
    <div class="node-foot">
      {#each shownBadges as badge (badge.label)}
        <span class="chip chip--{severityTone(badge.severity)}" title={badge.tooltip ?? ''}>
          {badge.label}
        </span>
      {/each}
      {#if extraBadges > 0}
        <span class="chip chip--neutral">+{extraBadges}</span>
      {/if}
      {#if policyCount > 0}
        <span class="chip chip--{failingPolicies > 0 ? 'danger' : 'neutral'}" data-testid="policy-badge">
          {failingPolicies > 0 ? `${failingPolicies} failing` : `${policyCount} policy`}
        </span>
      {/if}
    </div>
  </div>
  {#if showHandles}
    <Handle type="source" position={Position.Right} isConnectable={false} />
  {/if}
</div>

<style>
  .node-shell {
    --cat: var(--cat-generic);
    position: relative;
    display: flex;
    width: 100%;
    min-height: 84px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-panel);
    overflow: hidden;
    transition:
      border-color var(--motion-fast) var(--ease),
      box-shadow var(--motion-fast) var(--ease);
  }

  .node-shell:hover {
    border-color: var(--border-strong);
  }

  .node-shell.cat-kservice {
    --cat: var(--cat-kservice);
  }
  .node-shell.cat-revision {
    --cat: var(--cat-revision);
  }
  .node-shell.cat-broker {
    --cat: var(--cat-broker);
  }
  .node-shell.cat-trigger {
    --cat: var(--cat-trigger);
  }
  .node-shell.cat-backing {
    --cat: var(--cat-backing);
  }
  .node-shell.cat-storage {
    --cat: var(--cat-storage);
  }
  .node-shell.cat-policy {
    --cat: var(--cat-policy);
  }

  .node-rail {
    width: 3px;
    flex: none;
    background: var(--cat);
  }

  .node-body {
    min-width: 0;
    flex: 1;
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .node-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  .node-icon {
    color: var(--cat);
    display: inline-flex;
    flex: none;
  }

  .node-kind {
    color: var(--cat);
    font-family: var(--font-data);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .node-health {
    margin-left: auto;
    padding: 0 6px;
    min-height: 19px;
    font-size: 10px;
  }

  .node-name {
    font-size: 14px;
    font-weight: 600;
    line-height: 1.25;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }

  .node-scope {
    font-size: 10px;
    color: var(--text-faint);
    display: flex;
    gap: 4px;
  }

  .node-summary {
    margin: 0;
    color: var(--text-muted);
    font-size: 11.5px;
    line-height: 1.35;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .node-foot {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: auto;
    padding-top: 4px;
  }

  .node-foot :global(.chip) {
    min-height: 18px;
    font-size: 9.5px;
    padding: 0 5px;
  }

  .node-shell--compact .node-name {
    font-size: 12.5px;
  }

  .node-shell--compact .node-body {
    padding: 8px 10px;
  }
</style>
