<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import Graph from './lib/Graph.svelte';
  import Icon from './lib/Icon.svelte';
  import Filters from './lib/panels/Filters.svelte';
  import Inspector from './lib/panels/Inspector.svelte';
  import StatusBar from './lib/panels/StatusBar.svelte';
  import Tap from './lib/panels/Tap.svelte';
  import { store } from './lib/store.svelte';
  import type { ConnectionState } from './lib/events';

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

  const inspectorOpen = $derived(store.drawer === 'inspector' && store.selectedNode !== null);
  const tapOpen = $derived(store.drawer === 'tap');
  const filtersOpen = $derived(store.drawer === 'filters');
  const context = $derived(store.status?.context ?? store.graph.context);

  onMount(() => {
    void store.bootstrap();
    return () => store.close();
  });

  onDestroy(() => store.close());

  function retry(): void {
    void store.bootstrap();
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    if (store.drawer === 'inspector') {
      store.clearSelection();
    } else if (store.drawer !== null) {
      store.setDrawer(null);
    }
  }

  function toggleTap(): void {
    store.setDrawer(tapOpen ? null : 'tap');
  }

  function toggleFilters(): void {
    store.setDrawer(filtersOpen ? null : 'filters');
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="shell">
  <header class="topbar">
    <div class="brand">
      <span class="brand-mark" aria-hidden="true">K</span>
      <div class="brand-text">
        <h1>knect</h1>
        <p>topology console</p>
      </div>
    </div>

    <div class="meta">
      <span class="chip chip--neutral mono" title="Kubernetes context">{context}</span>
      <span class="meta-item mono">gen {store.graph.generation}</span>
      <span class="meta-item mono">{store.graph.stats.node_count} nodes</span>
      <span class="meta-item mono">{store.graph.stats.edge_count} edges</span>
      <span class="chip chip--{connectionTone[store.connection]}" data-testid="topbar-connection">
        <span class="dot" aria-hidden="true"></span>{connectionLabel[store.connection]}
      </span>
    </div>

    <div class="actions">
      <button class="icon-btn mobile-only" type="button" aria-label="Toggle filters" aria-pressed={filtersOpen} onclick={toggleFilters}>
        <Icon path="M4 6h16 M7 12h10 M10 18h4" size={15} />
      </button>
      <button class="icon-btn" type="button" aria-label="Refresh topology" onclick={() => void store.refresh()}>
        <Icon path="M20 11a8 8 0 1 0-2 5.3 M20 5v6h-6" size={15} />
      </button>
      <button class="tap-btn" type="button" aria-pressed={tapOpen} onclick={toggleTap}>
        <Icon path="M13 2 4 14h7l-1 8 9-12h-7z" size={14} />
        <span>Tap</span>
        {#if store.tapHandles.length > 0}
          <span class="tap-count mono">{store.tapHandles.length}</span>
        {/if}
      </button>
    </div>
  </header>

  <div class="workspace">
    <div class="rail" class:rail--open={filtersOpen}>
      <Filters />
    </div>
    {#if filtersOpen}
      <button class="backdrop" type="button" aria-label="Close filters" onclick={() => store.setDrawer(null)}></button>
    {/if}

    <main class="canvas-shell">
      <Graph />

      {#if store.transportError && !store.fatalError}
        <div class="banner banner--error" role="alert">
          <span>{store.transportError}</span>
          <button type="button" class="banner-dismiss" aria-label="Dismiss error" onclick={() => store.dismissTransportError()}>
            <Icon path="M6 6l12 12 M18 6 6 18" size={13} />
          </button>
        </div>
      {/if}

      {#if store.loading}
        <div class="overlay" role="status" aria-live="polite">
          <div class="loading-card">
            <span class="spinner" aria-hidden="true"></span>
            <h2>Connecting to knectd</h2>
            <p class="muted">Reading daemon status, topology generation, and the SSE stream.</p>
          </div>
        </div>
      {:else if store.fatalError}
        <div class="overlay" role="alert">
          <div class="loading-card loading-card--error">
            <span class="overlay-icon" aria-hidden="true">
              <Icon path="M12 3 2 21h20z M12 9v5 M12 17h.01" size={22} />
            </span>
            <h2>Cannot reach the daemon</h2>
            <p class="muted">{store.fatalError}</p>
            <p class="hint mono">
              knectd serves this page at http://127.0.0.1:7420. If the assets are missing, build them with
              <code>npm --prefix webui ci &amp;&amp; npm --prefix webui run build</code>.
            </p>
            <button type="button" class="primary" onclick={retry}>Retry connection</button>
          </div>
        </div>
      {:else if !store.hasGraph}
        <div class="overlay" role="status">
          <div class="loading-card">
            <span class="overlay-icon" aria-hidden="true">
              <Icon path="M4 7h16 M4 12h16 M4 17h10" size={22} />
            </span>
            <h2>No resources yet</h2>
            <p class="muted">
              The daemon is live and streaming. Nothing matches the current Kubernetes context, or discovery is
              still warming up.
            </p>
            <button type="button" class="primary" onclick={() => void store.refresh()}>Refresh graph</button>
          </div>
        </div>
      {:else if store.visibleNodes.length === 0}
        <div class="overlay overlay--soft" role="status">
          <div class="loading-card loading-card--compact">
            <h2>No nodes match the filters</h2>
            <p class="muted">{store.nodes.length} resources are hidden by search, health, namespace, or category filters.</p>
            <button type="button" class="primary" onclick={() => store.resetFilters()}>Reset filters</button>
          </div>
        </div>
      {/if}
    </main>

    {#if inspectorOpen}
      <div class="drawer" data-testid="inspector-drawer">
        <Inspector />
      </div>
      <button class="drawer-backdrop" type="button" aria-label="Close inspector" onclick={() => store.clearSelection()}></button>
    {/if}

    {#if tapOpen}
      <div class="drawer" data-testid="tap-drawer">
        <Tap />
      </div>
      <button class="drawer-backdrop" type="button" aria-label="Close tap panel" onclick={() => store.setDrawer(null)}></button>
    {/if}
  </div>

  <StatusBar />

  {#if store.actionMessage}
    <div class="toast" role="status" aria-live="polite">
      <Icon path="M5 12l5 5L20 7" size={14} />
      <span>{store.actionMessage}</span>
      <button type="button" aria-label="Dismiss action result" onclick={() => store.dismissAction()}>
        <Icon path="M6 6l12 12 M18 6 6 18" size={12} />
      </button>
    </div>
  {/if}

  <p class="sr-only" aria-live="polite">Connection {connectionLabel[store.connection]}</p>
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: 16px;
    min-height: 52px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex: none;
    z-index: 20;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    flex: none;
  }

  .brand-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 1px solid rgb(56 189 248 / 0.5);
    border-radius: 8px;
    background: linear-gradient(145deg, rgb(56 189 248 / 0.22), rgb(56 189 248 / 0.04));
    color: var(--accent);
    font-family: var(--font-data);
    font-size: 14px;
    font-weight: 700;
  }

  .brand-text h1 {
    margin: 0;
    font-size: 15px;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }

  .brand-text p {
    margin: 1px 0 0;
    font-family: var(--font-data);
    font-size: 8.5px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .meta::-webkit-scrollbar {
    display: none;
  }

  .meta-item {
    font-size: 10px;
    color: var(--text-faint);
    white-space: nowrap;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
    flex: none;
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    color: var(--text-muted);
  }

  .icon-btn:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }

  .tap-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 11px;
    border: 1px solid rgb(56 189 248 / 0.4);
    border-radius: var(--radius-sm);
    background: rgb(56 189 248 / 0.08);
    color: var(--accent);
    font-size: 11.5px;
  }

  .tap-btn:hover,
  .tap-btn[aria-pressed='true'] {
    background: rgb(56 189 248 / 0.15);
  }

  .tap-count {
    min-width: 16px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-ink);
    font-size: 9px;
    text-align: center;
  }

  .workspace {
    position: relative;
    display: flex;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .rail {
    width: 248px;
    flex: none;
    height: 100%;
    overflow: hidden;
    z-index: 8;
  }

  .canvas-shell {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  .drawer {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: min(440px, 92vw);
    z-index: 12;
    animation: slide-in var(--motion-base) var(--ease);
  }

  .drawer-backdrop {
    display: none;
  }

  @media (min-width: 1024px) {
    .drawer {
      position: relative;
      top: auto;
      right: auto;
      bottom: auto;
      width: min(440px, 38vw);
      flex: none;
      z-index: 3;
    }
  }

  .banner {
    position: absolute;
    top: 10px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: min(560px, 92%);
    padding: 7px 10px;
    border-radius: var(--radius-sm);
    font-size: 11px;
    z-index: 14;
  }

  .banner--error {
    border: 1px solid rgb(248 113 113 / 0.45);
    background: rgb(40 18 20 / 0.96);
    color: var(--danger);
  }

  .banner-dismiss {
    display: inline-flex;
    border: 0;
    background: transparent;
    color: inherit;
    padding: 2px;
  }

  .overlay {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 20px;
    background: rgb(9 12 17 / 0.55);
    backdrop-filter: blur(2px);
    z-index: 10;
  }

  .overlay--soft {
    background: rgb(9 12 17 / 0.4);
  }

  .loading-card {
    width: min(430px, 100%);
    padding: 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: var(--shadow-panel);
    text-align: left;
  }

  .loading-card--error {
    border-color: rgb(248 113 113 / 0.45);
  }

  .loading-card--compact {
    width: min(380px, 100%);
    padding: 18px;
  }

  .loading-card h2 {
    margin: 8px 0 6px;
    font-size: 17px;
    letter-spacing: -0.01em;
  }

  .loading-card p {
    margin: 0 0 10px;
    font-size: 12px;
  }

  .hint {
    padding: 8px;
    border-radius: var(--radius-xs);
    background: var(--surface-sunken);
    color: var(--text-faint);
    font-size: 9.5px;
    overflow-wrap: anywhere;
  }

  .hint code {
    color: var(--text-muted);
  }

  .overlay-icon {
    color: var(--text-faint);
  }

  .primary {
    min-height: 32px;
    padding: 4px 12px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--accent-ink);
    font-size: 12px;
    font-weight: 600;
  }

  .primary:hover {
    filter: brightness(1.08);
  }

  .spinner {
    display: inline-block;
    width: 18px;
    height: 18px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 700ms linear infinite;
  }

  .toast {
    position: absolute;
    right: 14px;
    bottom: 44px;
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: min(420px, 92vw);
    padding: 8px 11px;
    border: 1px solid rgb(52 211 153 / 0.4);
    border-radius: var(--radius-sm);
    background: rgb(12 30 24 / 0.97);
    color: var(--ok);
    font-size: 11px;
    box-shadow: var(--shadow-panel);
    z-index: 30;
  }

  .toast button {
    display: inline-flex;
    border: 0;
    background: transparent;
    color: inherit;
    padding: 2px;
  }

  .mobile-only {
    display: none;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @keyframes slide-in {
    from {
      transform: translateX(18px);
      opacity: 0;
    }
    to {
      transform: translateX(0);
      opacity: 1;
    }
  }

  @media (max-width: 1023px) {
    .rail {
      position: absolute;
      top: 0;
      left: 0;
      bottom: 0;
      width: min(320px, 86vw);
      transform: translateX(-102%);
      transition: transform var(--motion-base) var(--ease);
    }

    .rail--open {
      transform: translateX(0);
    }

    .mobile-only {
      display: inline-flex;
    }

    .backdrop {
      position: absolute;
      inset: 0;
      z-index: 7;
      border: 0;
      background: rgb(9 12 17 / 0.5);
    }
  }

  @media (max-width: 767px) {
    .topbar {
      flex-wrap: wrap;
      gap: 8px;
      min-height: 0;
      padding: 8px 10px;
    }

    .brand-text p,
    .meta-item {
      display: none;
    }

    .meta {
      order: 3;
      width: 100%;
      gap: 8px;
    }

    .actions {
      margin-left: auto;
    }

    .drawer {
      width: 100%;
      top: auto;
      height: min(76vh, 640px);
      border-top: 1px solid var(--border);
      animation: sheet-in var(--motion-base) var(--ease);
    }

    .drawer-backdrop {
      display: block;
      position: absolute;
      inset: 0;
      z-index: 11;
      border: 0;
      background: rgb(9 12 17 / 0.55);
    }

    .loading-card {
      padding: 18px;
    }
  }

  @keyframes sheet-in {
    from {
      transform: translateY(24px);
      opacity: 0;
    }
    to {
      transform: translateY(0);
      opacity: 1;
    }
  }
</style>
