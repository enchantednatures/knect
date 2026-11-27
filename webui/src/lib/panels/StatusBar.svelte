<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../Icon.svelte';
  import { store } from '../store.svelte';
  import { formatAge } from '../theme';
  import type { ConnectionState } from '../events';

  let now = $state(Date.now());

  onMount(() => {
    const timer = setInterval(() => {
      now = Date.now();
    }, 5_000);
    return () => clearInterval(timer);
  });

  const connectionLabel: Record<ConnectionState, string> = {
    idle: 'Idle',
    connecting: 'Connecting',
    live: 'Live',
    reconnecting: 'Reconnecting',
    closed: 'Disconnected',
    error: 'Error',
  };

  const connectionTone: Record<ConnectionState, string> = {
    idle: 'neutral',
    connecting: 'progress',
    live: 'ok',
    reconnecting: 'warn',
    closed: 'danger',
    error: 'danger',
  };

  const connectionText = $derived(connectionLabel[store.connection]);
  const connectionToneClass = $derived(connectionTone[store.connection]);
  const eventAge = $derived(
    store.lastEventAt === null
      ? '—'
      : formatAge(new Date(store.lastEventAt).toISOString(), now),
  );
  const heartbeatAge = $derived(
    store.realtime.heartbeat === null
      ? '—'
      : formatAge(store.realtime.heartbeat.ts, now),
  );
</script>

<footer class="status-bar" data-testid="status-bar">
  <span class="chip chip--{connectionToneClass}" data-testid="connection-state">
    <span class="dot" aria-hidden="true"></span>{connectionText}
  </span>
  <span class="status-item mono" title="Kubernetes context">
    <Icon path="M4 7h16 M4 12h16 M4 17h16" size={12} />
    {store.status?.context ?? store.graph.context}
  </span>
  <span class="status-item mono" title="Topology scope">
    {store.filters.scope === 'knative' ? 'knative scope' : 'all resources'}
  </span>
  <span class="status-item mono">gen {store.graph.generation}</span>
  <span class="status-item mono" title="Visible nodes">{store.visibleNodes.length} nodes</span>
  <span class="status-item mono" title="Visible edges">{store.visibleEdges.length} edges</span>
  <span class="status-item mono">{store.realtime.policies.length} policies</span>
  <span class="status-spacer"></span>
  <span class="status-item mono" title="Last SSE event">sse {eventAge}</span>
  <span class="status-item mono" title="Last daemon heartbeat">hb {heartbeatAge}</span>
  {#if store.status?.actions_enabled}
    <span class="chip chip--warn">actions on</span>
  {:else}
    <span class="chip chip--neutral">read-only</span>
  {/if}
</footer>

<style>
  .status-bar {
    display: flex;
    align-items: center;
    gap: 14px;
    min-height: 34px;
    padding: 0 14px;
    border-top: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-faint);
    font-size: 10.5px;
    overflow-x: auto;
    scrollbar-width: none;
    flex: none;
  }

  .status-bar::-webkit-scrollbar {
    display: none;
  }

  .status-item {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
  }

  .status-spacer {
    flex: 1;
    min-width: 8px;
  }

  @media (max-width: 767px) {
    .status-bar {
      gap: 10px;
      padding: 0 10px;
      min-height: 38px;
    }

    .status-item:nth-of-type(n + 4):not(.status-spacer) {
      display: none;
    }

    .status-spacer {
      display: none;
    }
  }
</style>
