<script lang="ts">
  import Icon from '../Icon.svelte';
  import { namespaceOptions } from '../filters';
  import { store } from '../store.svelte';
  import { categoryLabel, categoryMeta } from '../theme';
  import type { Health, NodeCategory } from '../types';

  const categories = $derived.by(() => {
    const counts = new Map<NodeCategory, number>();
    for (const node of store.nodes) {
      counts.set(node.category, (counts.get(node.category) ?? 0) + 1);
    }
    return [...counts.entries()]
      .map(([category, count]) => ({ category, count }))
      .sort((a, b) => categoryLabel(a.category).localeCompare(categoryLabel(b.category)));
  });

  const namespaces = $derived(namespaceOptions(store.nodes));

  const healthOptions: { value: Health | 'all'; label: string }[] = [
    { value: 'all', label: 'Any health' },
    { value: 'healthy', label: 'Healthy' },
    { value: 'degraded', label: 'Degraded' },
    { value: 'progressing', label: 'Progressing' },
    { value: 'blocked', label: 'Blocked' },
    { value: 'unknown', label: 'Unknown' },
  ];

  function toggle(category: NodeCategory): void {
    store.toggleCategory(category);
  }
</script>

<aside class="filters" aria-label="Graph filters">
  <div class="filters-head">
    <div>
      <h2>Filters</h2>
      <p class="faint">{store.visibleNodes.length} of {store.nodes.length} visible</p>
    </div>
    <button class="icon-btn" type="button" aria-label="Reset filters" onclick={() => store.resetFilters()}>
      <Icon path="M4 4v6h6 M20 20v-6h-6 M5 15a7 7 0 0 0 11 3 M19 9A7 7 0 0 0 8 6" size={14} />
    </button>
  </div>

  <label class="field">
    <span class="field-label">Search</span>
    <span class="search-wrap">
      <Icon path="M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14 M16 16l4 4" size={13} />
      <input
        type="search"
        placeholder="name, kind, label…"
        value={store.filters.search}
        oninput={(event) => store.setFilter('search', event.currentTarget.value)}
      />
    </span>
  </label>

  <div class="field-row">
    <label class="field">
      <span class="field-label">Health</span>
      <select
        value={store.filters.health}
        onchange={(event) => store.setFilter('health', event.currentTarget.value as Health | 'all')}
      >
        {#each healthOptions as option (option.value)}
          <option value={option.value}>{option.label}</option>
        {/each}
      </select>
    </label>
    <label class="field">
      <span class="field-label">Namespace</span>
      <select
        value={store.filters.namespace}
        onchange={(event) => store.setFilter('namespace', event.currentTarget.value)}
      >
        <option value="">All namespaces</option>
        {#each namespaces as option (option.value)}
          <option value={option.value}>{option.label} ({option.count})</option>
        {/each}
      </select>
    </label>
  </div>

  <div class="toggle-row">
    <button
      class="toggle"
      class:on={store.filters.problemsOnly}
      type="button"
      aria-pressed={store.filters.problemsOnly}
      onclick={() => store.setFilter('problemsOnly', !store.filters.problemsOnly)}
    >
      Problems only
    </button>
  </div>

  <section class="group" aria-labelledby="filters-groups">
    <h3 id="filters-groups">Resource groups</h3>
    <div class="toggle-row">
      <button
        class="toggle"
        class:on={store.filters.showPolicies}
        type="button"
        aria-pressed={store.filters.showPolicies}
        onclick={() => store.setFilter('showPolicies', !store.filters.showPolicies)}
      >
        Policy
      </button>
      <button
        class="toggle"
        class:on={store.filters.showBacking}
        type="button"
        aria-pressed={store.filters.showBacking}
        onclick={() => store.setFilter('showBacking', !store.filters.showBacking)}
      >
        Backing
      </button>
      <button
        class="toggle"
        class:on={store.filters.showStorage}
        type="button"
        aria-pressed={store.filters.showStorage}
        onclick={() => store.setFilter('showStorage', !store.filters.showStorage)}
      >
        Storage
      </button>
    </div>
  </section>

  <section class="group" aria-labelledby="filters-categories">
    <h3 id="filters-categories">Categories</h3>
    <ul class="category-list">
      {#each categories as entry (entry.category)}
        {@const meta = categoryMeta(entry.category)}
        {@const hidden = store.filters.hiddenCategories.includes(entry.category)}
        <li>
          <button
            class="category"
            class:off={hidden}
            type="button"
            aria-pressed={!hidden}
            onclick={() => toggle(entry.category)}
          >
            <span class="cat-swatch" style:background="var(--cat-{meta.group})" aria-hidden="true"></span>
            <span class="cat-name">{meta.label}</span>
            <span class="cat-count mono">{entry.count}</span>
          </button>
        </li>
      {/each}
    </ul>
    {#if categories.length === 0}
      <p class="faint empty-note">No resource categories observed yet.</p>
    {/if}
  </section>
</aside>

<style>
  .filters {
    display: flex;
    flex-direction: column;
    gap: 14px;
    width: 100%;
    padding: 16px 14px 24px;
    overflow-y: auto;
    border-right: 1px solid var(--border);
    background: var(--surface);
    height: 100%;
  }

  .filters-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 8px;
  }

  .filters-head h2 {
    margin: 0;
    font-size: 14px;
    letter-spacing: -0.01em;
  }

  .filters-head p {
    margin: 2px 0 0;
    font-size: 10.5px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
    flex: 1;
  }

  .field-label {
    font-family: var(--font-data);
    font-size: 9.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .search-wrap {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 9px;
    height: 32px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    color: var(--text-faint);
  }

  .search-wrap:focus-within {
    border-color: var(--accent);
  }

  .search-wrap input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    outline: none;
    font-size: 12px;
    color: var(--text);
  }

  .field-row {
    display: flex;
    gap: 10px;
  }

  select {
    height: 32px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    color: var(--text);
    font-size: 11.5px;
    min-width: 0;
  }

  .toggle-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .toggle {
    min-height: 28px;
    padding: 3px 9px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-sunken);
    color: var(--text-faint);
    font-size: 11px;
    transition:
      color var(--motion-fast) var(--ease),
      border-color var(--motion-fast) var(--ease),
      background var(--motion-fast) var(--ease);
  }

  .toggle:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }

  .toggle.on {
    color: var(--accent);
    border-color: rgb(56 189 248 / 0.45);
    background: rgb(56 189 248 / 0.08);
  }

  .group h3 {
    margin: 0 0 7px;
    font-family: var(--font-data);
    font-size: 9.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
    font-weight: 600;
  }

  .category-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .category {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 30px;
    padding: 3px 7px;
    border: 1px solid transparent;
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--text-muted);
    font-size: 11.5px;
    text-align: left;
  }

  .category:hover {
    background: var(--surface-raised);
    color: var(--text);
  }

  .category.off {
    opacity: 0.42;
  }

  .category.off .cat-name {
    text-decoration: line-through;
  }

  .cat-swatch {
    width: 7px;
    height: 7px;
    border-radius: 2px;
    flex: none;
  }

  .cat-name {
    flex: 1;
  }

  .cat-count {
    font-size: 10px;
    color: var(--text-faint);
  }

  .empty-note {
    font-size: 11px;
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    color: var(--text-muted);
  }

  .icon-btn:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }

  @media (max-width: 1023px) {
    .filters {
      width: min(320px, 86vw);
      box-shadow: var(--shadow-panel);
    }
  }
</style>
