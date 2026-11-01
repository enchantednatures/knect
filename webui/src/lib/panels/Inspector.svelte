<script lang="ts">
  import Icon from '../Icon.svelte';
  import { store } from '../store.svelte';
  import {
    categoryColor,
    categoryMeta,
    formatAge,
    formatBytes,
    formatNumber,
    healthMeta,
    ownerTone,
    severityTone,
  } from '../theme';
  import { resourceKey, resourceLabel, type PolicyRecord } from '../types';
  import MutationDiff from './MutationDiff.svelte';

  type Tab = 'overview' | 'yaml' | 'mutations' | 'policies' | 'links' | 'storage';

  const tabs: { id: Tab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'yaml', label: 'YAML' },
    { id: 'mutations', label: 'Mutations' },
    { id: 'policies', label: 'Policies' },
    { id: 'links', label: 'Links' },
    { id: 'storage', label: 'Storage' },
  ];

  let tab = $state<Tab>('overview');

  const node = $derived(store.selectedNode);
  const details = $derived(store.details);
  const meta = $derived(node ? categoryMeta(node.category) : null);
  const health = $derived(node ? healthMeta(node.health) : null);
  const policies = $derived.by(() => {
    const merged = new Map<string, PolicyRecord>();
    for (const policy of details?.policies ?? []) merged.set(policy.id, policy);
    for (const policy of store.policiesForSelection) merged.set(policy.id, policy);
    return [...merged.values()].sort((a, b) => a.name.localeCompare(b.name));
  });

  $effect(() => {
    if (store.selectedKey !== null) tab = 'overview';
  });

  $effect(() => {
    if (
      tab === 'mutations' &&
      store.selectedKey !== null &&
      store.diff === null &&
      !store.diffLoading &&
      store.diffError === null
    ) {
      void store.loadDiff();
    }
  });

  function isTappable(): boolean {
    return node?.category === 'broker' || node?.category === 'trigger';
  }
</script>

{#if node && meta && health}
  <aside class="inspector" aria-label="Resource inspector">
    <header class="inspector-head">
      <span class="head-icon" style:color={categoryColor(node.category)} aria-hidden="true">
        <Icon path={meta.icon} size={17} />
      </span>
      <div class="head-text">
        <span class="head-kind">{meta.label}</span>
        <h2 title={node.name}>{node.name}</h2>
        <p class="faint mono">
          {node.id.api_version}/{node.id.kind}
          {node.namespace ? ` · ${node.namespace}` : ' · cluster-scoped'}
        </p>
      </div>
      <span class="chip chip--{health.tone}">
        <span class="dot" aria-hidden="true"></span>{health.label}
      </span>
      <button class="close" type="button" aria-label="Close inspector" onclick={() => store.clearSelection()}>
        <Icon path="M6 6l12 12 M18 6 6 18" size={15} />
      </button>
    </header>

    {#if isTappable()}
      <div class="tap-callout">
        <span>CloudEvents can be captured on this {meta.label.toLowerCase()}.</span>
        <button type="button" class="action" onclick={() => store.setDrawer('tap')}>Open tap</button>
      </div>
    {/if}

    <div class="tabs" role="tablist" aria-label="Inspector sections">
      {#each tabs as entry (entry.id)}
        <button
          type="button"
          role="tab"
          id={`inspector-tab-${entry.id}`}
          aria-selected={tab === entry.id}
          aria-controls={`inspector-panel-${entry.id}`}
          class:active={tab === entry.id}
          onclick={() => (tab = entry.id)}
        >
          {entry.label}
          {#if entry.id === 'policies' && policies.length > 0}
            <span class="tab-count mono">{policies.length}</span>
          {/if}
        </button>
      {/each}
    </div>

    <div class="inspector-body">
      {#if store.detailsLoading}
        <p class="state" role="status">Loading resource details…</p>
      {:else if store.detailsError}
        <div class="state state--error" role="alert">
          <p>{store.detailsError}</p>
          <button type="button" class="action" onclick={() => node && store.loadDetails(node)}>Retry</button>
        </div>
      {:else}
        <div
          id={`inspector-panel-${tab}`}
          role="tabpanel"
          aria-labelledby={`inspector-tab-${tab}`}
          class="panel"
        >
          {#if tab === 'overview'}
            <p class="summary">{node.summary || 'No summary reported.'}</p>

            {#if details?.metrics}
              <div class="metrics">
                {#if details.metrics.replicas !== null}
                  <div class="metric">
                    <span class="metric-label">Replicas</span>
                    <span class="metric-value mono">{formatNumber(details.metrics.replicas)}</span>
                  </div>
                {/if}
                {#if details.metrics.ready_replicas !== null}
                  <div class="metric">
                    <span class="metric-label">Ready</span>
                    <span class="metric-value mono">{formatNumber(details.metrics.ready_replicas)}</span>
                  </div>
                {/if}
                {#if details.metrics.cpu_millicores !== null}
                  <div class="metric">
                    <span class="metric-label">CPU</span>
                    <span class="metric-value mono">{formatNumber(details.metrics.cpu_millicores)}m</span>
                  </div>
                {/if}
                {#if details.metrics.memory_bytes !== null}
                  <div class="metric">
                    <span class="metric-label">Memory</span>
                    <span class="metric-value mono">{formatBytes(details.metrics.memory_bytes)}</span>
                  </div>
                {/if}
                {#if details.metrics.requests_per_second !== null}
                  <div class="metric">
                    <span class="metric-label">Traffic</span>
                    <span class="metric-value mono">{formatNumber(details.metrics.requests_per_second)}/s</span>
                  </div>
                {/if}
                {#if details.metrics.error_rate !== null}
                  <div class="metric">
                    <span class="metric-label">Errors</span>
                    <span class="metric-value mono">{(details.metrics.error_rate * 100).toFixed(2)}%</span>
                  </div>
                {/if}
              </div>
            {/if}

            <dl class="kv">
              <div><dt>Generation</dt><dd class="mono">{store.graph.generation}</dd></div>
              <div><dt>First seen</dt><dd class="mono">{formatAge(node.first_seen)} ago</dd></div>
              <div><dt>Last seen</dt><dd class="mono">{formatAge(node.last_seen)} ago</dd></div>
              <div><dt>UID</dt><dd class="mono ellipsis">{node.uid ?? '—'}</dd></div>
            </dl>

            {#if node.badges.length > 0}
              <div class="chips">
                {#each node.badges as badge (badge.label)}
                  <span class="chip chip--{severityTone(badge.severity)}" title={badge.tooltip ?? ''}>
                    {badge.label}
                  </span>
                {/each}
              </div>
            {/if}

            {#if details && details.warnings.length > 0}
              <h3 class="subhead">Warnings</h3>
              <ul class="plain-list">
                {#each details.warnings as warning (warning.code + warning.message)}
                  <li>
                    <span class="chip chip--{severityTone(warning.severity)}">{warning.severity}</span>
                    <span>{warning.message}</span>
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if tab === 'yaml'}
            {#if details?.yaml}
              <div class="yaml-head">
                <span class="chip chip--ok" title="Daemon redaction passed">
                  {details.yaml_redacted ? 'redacted' : 'unredacted'}
                </span>
                <span class="faint mono">{details.yaml.split('\n').length} lines</span>
              </div>
              <pre class="yaml" aria-label="Redacted manifest">{details.yaml}</pre>
            {:else}
              <p class="state">No YAML is available for this resource.</p>
            {/if}
          {:else if tab === 'mutations'}
            <MutationDiff diff={store.diff} loading={store.diffLoading} error={store.diffError} />
            {#if details && details.mutations.length > 0}
              <h3 class="subhead">Managed fields</h3>
              <ul class="mutation-list">
                {#each details.mutations as record (record.manager + (record.time ?? ''))}
                  <li>
                    <div class="mutation-head">
                      <span class="owner" style:--owner={ownerTone(record.manager)}>{record.manager}</span>
                      <span class="chip chip--neutral">{record.manager_kind}</span>
                      <span class="chip chip--neutral">{record.operation}</span>
                      {#if record.subresource}
                        <span class="chip chip--neutral">{record.subresource}</span>
                      {/if}
                    </div>
                    <ul class="paths">
                      {#each record.paths as path (path.path)}
                        <li class="mono">
                          <span class="path">{path.path}</span>
                          {#if path.value_preview}
                            <span class="faint">{path.value_preview}</span>
                          {/if}
                        </li>
                      {/each}
                    </ul>
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if tab === 'policies'}
            {#if policies.length === 0}
              <p class="state">No policy findings for this resource.</p>
            {:else}
              <ul class="policy-list">
                {#each policies as policy (policy.id)}
                  <li>
                    <div class="policy-head">
                      <span class="chip chip--{severityTone(policy.severity)}">{policy.severity}</span>
                      <span class="chip chip--{policy.result === 'pass' ? 'ok' : policy.result === 'fail' ? 'danger' : 'warn'}">
                        {policy.result}
                      </span>
                      <span class="chip chip--neutral">{policy.kind}</span>
                      <span class="policy-name">{policy.name}</span>
                    </div>
                    <p>{policy.message}</p>
                    {#if Object.keys(policy.details).length > 0}
                      <dl class="kv kv--tight">
                        {#each Object.entries(policy.details) as [key, value] (key)}
                          <div><dt>{key}</dt><dd class="mono">{value}</dd></div>
                        {/each}
                      </dl>
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if tab === 'links'}
            {#if !details || details.links.length === 0}
              <p class="state">No correlated links for this resource.</p>
            {:else}
              <ul class="link-list">
                {#each details.links as link (resourceKey(link.source) + '>' + resourceKey(link.target))}
                  <li>
                    <div class="link-head">
                      <span class="chip chip--neutral">{link.via}</span>
                      <span class="chip chip--{healthMeta(link.health).tone}">{healthMeta(link.health).label}</span>
                      <span class="faint mono">{Math.round(link.confidence * 100)}% confidence</span>
                    </div>
                    <p class="mono link-endpoints">
                      {resourceLabel(link.source)} → {resourceLabel(link.target)}
                    </p>
                    {#if link.evidence.length > 0}
                      <ul class="evidence">
                        {#each link.evidence as evidence (evidence.kind + evidence.description)}
                          <li>
                            <span class="chip chip--neutral">{evidence.kind}</span>
                            <span>{evidence.description}</span>
                            {#if evidence.value_redacted}
                              <span class="faint mono">{evidence.value_redacted}</span>
                            {/if}
                          </li>
                        {/each}
                      </ul>
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if tab === 'storage'}
            {#if !details || details.storage.length === 0}
              <p class="state">No storage claims mounted by this resource.</p>
            {:else}
              <ul class="storage-list">
                {#each details.storage as record (resourceKey(record.pvc))}
                  <li>
                    <div class="storage-head">
                      <span class="mono">{record.pvc.name}</span>
                      {#if record.phase}<span class="chip chip--neutral">{record.phase}</span>{/if}
                      {#if record.capacity}<span class="chip chip--neutral">{record.capacity}</span>{/if}
                    </div>
                    <dl class="kv kv--tight">
                      <div><dt>Class</dt><dd class="mono">{record.storage_class ?? '—'}</dd></div>
                      <div><dt>Modes</dt><dd class="mono">{record.access_modes.join(', ') || '—'}</dd></div>
                      <div><dt>Volume</dt><dd class="mono">{record.volume ?? '—'}</dd></div>
                      <div><dt>Backend</dt><dd class="mono">{record.backend ? resourceLabel(record.backend) : '—'}</dd></div>
                    </dl>
                    {#each record.warnings as warning (warning.code)}
                      <p class="storage-warning">
                        <span class="chip chip--{severityTone(warning.severity)}">{warning.severity}</span>
                        {warning.message}
                      </p>
                    {/each}
                  </li>
                {/each}
              </ul>
            {/if}
          {/if}
        </div>
      {/if}
    </div>
  </aside>
{/if}

<style>
  .inspector {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background: var(--surface);
    border-left: 1px solid var(--border);
    box-shadow: var(--shadow-panel);
    min-height: 0;
  }

  .inspector-head {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 13px 14px 11px;
    border-bottom: 1px solid var(--border);
  }

  .head-icon {
    margin-top: 2px;
    flex: none;
  }

  .head-text {
    min-width: 0;
    flex: 1;
  }

  .head-kind {
    font-family: var(--font-data);
    font-size: 9px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .head-text h2 {
    margin: 1px 0 2px;
    font-size: 15px;
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .head-text p {
    margin: 0;
    font-size: 9.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    flex: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    color: var(--text-muted);
  }

  .close:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }

  .tap-callout {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 14px;
    border-bottom: 1px solid var(--border);
    background: rgb(56 189 248 / 0.06);
    color: var(--text-muted);
    font-size: 10.5px;
  }

  .tabs {
    display: flex;
    gap: 2px;
    padding: 6px 8px 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
    flex: none;
  }

  .tabs::-webkit-scrollbar {
    display: none;
  }

  .tabs button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 6px 8px 7px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-faint);
    font-size: 11px;
    white-space: nowrap;
  }

  .tabs button:hover {
    color: var(--text-muted);
  }

  .tabs button.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }

  .tab-count {
    font-size: 8.5px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--surface-raised);
    color: var(--text-muted);
  }

  .inspector-body {
    flex: 1;
    overflow-y: auto;
    padding: 12px 14px 24px;
    min-height: 0;
  }

  .panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .summary {
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .metrics {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 6px;
  }

  .metric {
    padding: 7px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    background: var(--surface-sunken);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .metric-label {
    font-family: var(--font-data);
    font-size: 8.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .metric-value {
    font-size: 13px;
    color: var(--text);
  }

  .kv {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .kv div {
    display: flex;
    gap: 8px;
    justify-content: space-between;
    font-size: 10.5px;
  }

  .kv dt {
    color: var(--text-faint);
    flex: none;
  }

  .kv dd {
    margin: 0;
    color: var(--text-muted);
    min-width: 0;
    text-align: right;
  }

  .kv--tight div {
    font-size: 10px;
  }

  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .subhead {
    margin: 4px 0 0;
    font-family: var(--font-data);
    font-size: 9.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .plain-list,
  .policy-list,
  .mutation-list,
  .link-list,
  .storage-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 9px;
  }

  .plain-list li {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    color: var(--text-muted);
    font-size: 11px;
  }

  .yaml-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .yaml {
    margin: 0;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    color: var(--text-muted);
    font-family: var(--font-data);
    font-size: 10.5px;
    line-height: 1.55;
    overflow: auto;
    max-height: 52vh;
    white-space: pre;
  }

  .mutation-list > li,
  .policy-list > li,
  .link-list > li,
  .storage-list > li {
    padding: 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .mutation-head,
  .policy-head,
  .link-head,
  .storage-head {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    align-items: center;
  }

  .owner {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-family: var(--font-data);
    font-size: 10px;
    color: var(--text);
  }

  .owner::before {
    content: '';
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--owner);
  }

  .paths {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 9.5px;
  }

  .paths li {
    display: flex;
    gap: 8px;
    justify-content: space-between;
  }

  .path {
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .policy-name {
    font-weight: 600;
    font-size: 11px;
  }

  .policy-list p,
  .link-list p,
  .storage-list p {
    margin: 0;
    color: var(--text-muted);
    font-size: 10.5px;
  }

  .link-endpoints {
    overflow-wrap: anywhere;
    font-size: 9.5px !important;
  }

  .evidence {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 10px;
    color: var(--text-muted);
  }

  .evidence li {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    align-items: center;
  }

  .storage-warning {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    color: var(--warn) !important;
  }

  .state {
    margin: 0;
    padding: 12px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-faint);
    font-size: 11.5px;
  }

  .state--error {
    display: flex;
    flex-direction: column;
    gap: 8px;
    color: var(--danger);
    border-color: rgb(248 113 113 / 0.4);
  }

  .action {
    align-self: flex-start;
    min-height: 28px;
    padding: 3px 10px;
    border: 1px solid rgb(56 189 248 / 0.4);
    border-radius: var(--radius-sm);
    background: rgb(56 189 248 / 0.1);
    color: var(--accent);
    font-size: 11px;
  }

  .action:hover {
    background: rgb(56 189 248 / 0.16);
  }

  @media (max-width: 480px) {
    .metrics {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
