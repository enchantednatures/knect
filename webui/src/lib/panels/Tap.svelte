<script lang="ts">
  import Icon from '../Icon.svelte';
  import { store } from '../store.svelte';
  import { formatAge } from '../theme';
  import type { TapHandle } from '../types';

  let kind = $state<'broker' | 'trigger'>('broker');
  let namespace = $state('');
  let name = $state('');
  let typeFilter = $state('');

  const events = $derived([...store.realtime.tapEvents].reverse());

  $effect(() => {
    const node = store.selectedNode;
    if (node && (node.category === 'broker' || node.category === 'trigger')) {
      kind = node.category === 'broker' ? 'broker' : 'trigger';
      namespace = node.namespace ?? '';
      name = node.name;
    }
  });

  const stateTone: Record<TapHandle['state'], string> = {
    starting: 'progress',
    active: 'ok',
    stopping: 'warn',
    stopped: 'neutral',
    failed: 'danger',
    expired: 'neutral',
    unknown: 'neutral',
  };

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (namespace.trim() === '' || name.trim() === '') {
      store.tapError = 'Namespace and name are required to start a tap.';
      return;
    }
    await store.startTap(kind, namespace.trim(), name.trim(), typeFilter);
  }

  function payloadPreview(payload: unknown): string {
    try {
      const text = JSON.stringify(payload);
      return text.length > 220 ? `${text.slice(0, 220)}…` : text;
    } catch {
      return '[unserializable payload]';
    }
  }
</script>

<aside class="tap-panel" aria-label="CloudEvent tap">
  <header class="tap-head">
    <div>
      <h2>Live tap</h2>
      <p class="faint">CloudEvents captured through an ephemeral Trigger.</p>
    </div>
    <button class="close" type="button" aria-label="Close tap panel" onclick={() => store.setDrawer(null)}>
      <Icon path="M6 6l12 12 M18 6 6 18" size={15} />
    </button>
  </header>

  <form class="tap-form" onsubmit={submit}>
    <div class="field-row">
      <label class="field">
        <span class="field-label">Target kind</span>
        <select bind:value={kind}>
          <option value="broker">Broker</option>
          <option value="trigger">Trigger</option>
        </select>
      </label>
      <label class="field">
        <span class="field-label">Namespace</span>
        <input bind:value={namespace} placeholder="default" autocomplete="off" />
      </label>
    </div>
    <label class="field">
      <span class="field-label">Target name</span>
      <input bind:value={name} placeholder="orders" autocomplete="off" />
    </label>
    <label class="field">
      <span class="field-label">CloudEvent type filter (optional)</span>
      <input bind:value={typeFilter} placeholder="order.created" autocomplete="off" />
    </label>
    <div class="form-actions">
      <button class="primary" type="submit" disabled={store.tapBusy}>
        {store.tapBusy ? 'Starting…' : 'Start tap'}
      </button>
      <button type="button" class="ghost" onclick={() => void store.refreshTaps()} disabled={store.tapBusy}>
        Refresh taps
      </button>
    </div>
    {#if store.tapError}
      <p class="error" role="alert">{store.tapError}</p>
    {/if}
  </form>

  <section class="section" aria-labelledby="tap-handles">
    <h3 id="tap-handles">Active taps <span class="faint mono">{store.tapHandles.length}</span></h3>
    {#if store.tapHandles.length === 0}
      <p class="state">No taps running. Start one above or from a Broker/Trigger inspector.</p>
    {:else}
      <ul class="handle-list">
        {#each store.tapHandles as tap (tap.id)}
          <li>
            <div class="handle-head">
              <span class="chip chip--{stateTone[tap.state]}">{tap.state}</span>
              <span class="mono handle-target">
                {tap.target.kind} {tap.target.namespace}/{tap.target.name}
              </span>
              <button
                class="ghost small"
                type="button"
                disabled={store.tapBusy || tap.state === 'stopped'}
                onclick={() => void store.stopTap(tap.id)}
              >
                Stop
              </button>
            </div>
            <p class="faint mono">
              {tap.id} · expires {formatAge(tap.expires) === '—' ? tap.expires : `in ${formatAge(tap.expires)}`}
            </p>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="section events" aria-labelledby="tap-events">
    <h3 id="tap-events">Captured events <span class="faint mono">{events.length}</span></h3>
    {#if events.length === 0}
      <p class="state">Events will stream here as CloudEvents are delivered.</p>
    {:else}
      <ol class="event-list">
        {#each events as event (event.id)}
          <li>
            <div class="event-head">
              <span class="chip chip--neutral">{event.ce_type || 'event'}</span>
              <span class="faint mono">{event.received_at}</span>
            </div>
            <p class="mono event-source">{event.ce_source}{event.ce_subject ? ` · ${event.ce_subject}` : ''}</p>
            <pre class="payload">{payloadPreview(event.payload)}</pre>
          </li>
        {/each}
      </ol>
    {/if}
  </section>
</aside>

<style>
  .tap-panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: 100%;
    height: 100%;
    padding: 13px 14px 18px;
    background: var(--surface);
    border-left: 1px solid var(--border);
    box-shadow: var(--shadow-panel);
    overflow-y: auto;
  }

  .tap-head {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    align-items: flex-start;
  }

  .tap-head h2 {
    margin: 0;
    font-size: 15px;
  }

  .tap-head p {
    margin: 2px 0 0;
    font-size: 10.5px;
  }

  .close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    color: var(--text-muted);
  }

  .close:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }

  .tap-form {
    display: flex;
    flex-direction: column;
    gap: 9px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
  }

  .field-row {
    display: flex;
    gap: 8px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
    min-width: 0;
  }

  .field-label {
    font-family: var(--font-data);
    font-size: 9px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  input,
  select {
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    background: var(--surface);
    color: var(--text);
    font-size: 11.5px;
    min-width: 0;
  }

  input:focus,
  select:focus {
    border-color: var(--accent);
    outline: none;
  }

  .form-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .primary,
  .ghost {
    min-height: 30px;
    padding: 3px 11px;
    border-radius: var(--radius-sm);
    font-size: 11.5px;
  }

  .primary {
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 600;
  }

  .primary:hover {
    filter: brightness(1.08);
  }

  .primary:disabled,
  .ghost:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .ghost {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-muted);
  }

  .ghost:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--border-strong);
  }

  .small {
    min-height: 24px;
    padding: 1px 8px;
    font-size: 10px;
  }

  .error {
    margin: 0;
    color: var(--danger);
    font-size: 10.5px;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .section h3 {
    margin: 0;
    font-size: 11.5px;
    font-family: var(--font-data);
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .state {
    margin: 0;
    padding: 10px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-faint);
    font-size: 10.5px;
  }

  .handle-list,
  .event-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .handle-list li,
  .event-list li {
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .handle-head,
  .event-head {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .handle-target {
    flex: 1;
    font-size: 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .handle-list p {
    margin: 0;
    font-size: 9px;
  }

  .events {
    min-height: 0;
  }

  .event-list {
    max-height: 42vh;
    overflow-y: auto;
  }

  .event-head .faint {
    font-size: 9px;
    margin-left: auto;
  }

  .event-source {
    margin: 0;
    font-size: 9.5px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .payload {
    margin: 0;
    padding: 6px;
    border-radius: var(--radius-xs);
    background: var(--bg);
    color: var(--text-faint);
    font-family: var(--font-data);
    font-size: 9px;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 120px;
    overflow: auto;
  }

  @media (max-width: 480px) {
    .field-row {
      flex-direction: column;
    }
  }
</style>
