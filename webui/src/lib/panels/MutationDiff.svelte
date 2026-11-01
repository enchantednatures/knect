<script lang="ts">
  import { ownerTone } from '../theme';
  import type { DiffLine, MutationDiff, MutationSource } from '../types';

  let {
    diff,
    loading = false,
    error = null,
  }: { diff: MutationDiff | null; loading?: boolean; error?: string | null } = $props();

  interface Pane {
    source: MutationSource;
    title: string;
    doc: string | null;
    hint: string;
  }

  const panes = $derived<Pane[]>(
    diff === null
      ? []
      : [
          {
            source: 'user_intent',
            title: 'User intent',
            doc: diff.user_intent,
            hint: 'What the operator applied',
          },
          {
            source: 'knative_defaults',
            title: 'Knative defaults',
            doc: diff.knative_defaults,
            hint: 'Controller-injected defaults',
          },
          {
            source: 'policy_mutators',
            title: 'Policy mutators',
            doc: diff.policy_mutators,
            hint: 'Policy and mesh mutations',
          },
        ],
  );

  function linesFor(doc: string | null): string[] {
    return doc === null ? [] : doc.replace(/\n$/, '').split('\n');
  }

  function changedLines(lines: DiffLine[], source: MutationSource): Map<number, DiffLine> {
    const map = new Map<number, DiffLine>();
    for (const line of lines) {
      if (line.side === source && line.changed) map.set(line.line_number, line);
    }
    return map;
  }

  function ownersFor(lines: DiffLine[], source: MutationSource): string[] {
    const owners = new Set<string>();
    for (const line of lines) {
      if (line.side === source && line.owner) owners.add(line.owner);
    }
    return [...owners];
  }
</script>

<section class="diff" aria-labelledby="mutation-diff-title">
  <div class="diff-head">
    <h3 id="mutation-diff-title">Three-way mutation diff</h3>
    <span class="chip chip--neutral">{diff?.lines.length ?? 0} attributed lines</span>
  </div>

  {#if loading}
    <p class="state" role="status">Loading mutation ownership…</p>
  {:else if error}
    <p class="state state--error" role="alert">{error}</p>
  {:else if diff === null}
    <p class="state">Select a resource to attribute its managed fields.</p>
  {:else}
    <div class="panes">
      {#each panes as pane (pane.source)}
        {@const sourceLines = linesFor(pane.doc)}
        {@const changed = changedLines(diff.lines, pane.source)}
        {@const owners = ownersFor(diff.lines, pane.source)}
        <article class="pane" aria-label={pane.title}>
          <header class="pane-head">
            <div>
              <h4>{pane.title}</h4>
              <p class="faint">{pane.hint}</p>
            </div>
            <span class="chip chip--neutral mono">{sourceLines.length} lines</span>
          </header>
          {#if owners.length > 0}
            <div class="owners">
              {#each owners as owner (owner)}
                <span class="owner" style:--owner={ownerTone(owner)}>{owner}</span>
              {/each}
            </div>
          {/if}
          {#if sourceLines.length === 0}
            <p class="state state--small">No document available for this pane.</p>
          {:else}
            <ol class="code" aria-label={`${pane.title} manifest`}>
              {#each sourceLines as text, index (index)}
                {@const lineNumber = index + 1}
                {@const attribution = changed.get(lineNumber)}
                <li class:changed={Boolean(attribution)}>
                  <span class="ln" aria-hidden="true">{lineNumber}</span>
                  <code>{text || ' '}</code>
                  {#if attribution?.owner}
                    <span class="line-owner" style:--owner={ownerTone(attribution.owner)}>
                      {attribution.owner}
                    </span>
                  {/if}
                </li>
              {/each}
            </ol>
          {/if}
        </article>
      {/each}
    </div>
  {/if}
</section>

<style>
  .diff {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .diff-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .diff-head h3 {
    margin: 0;
    font-size: 12.5px;
  }

  .panes {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }

  .pane {
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .pane-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 6px;
    padding: 8px 9px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .pane-head h4 {
    margin: 0;
    font-size: 11.5px;
  }

  .pane-head p {
    margin: 1px 0 0;
    font-size: 9.5px;
  }

  .owners {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }

  .owner {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-family: var(--font-data);
    font-size: 9px;
    color: var(--text-muted);
  }

  .owner::before {
    content: '';
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--owner);
  }

  .code {
    margin: 0;
    padding: 6px 0;
    list-style: none;
    overflow-x: auto;
    font-family: var(--font-data);
    font-size: 10.5px;
    line-height: 1.55;
    max-height: 46vh;
    overflow-y: auto;
  }

  .code li {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    padding: 0 8px;
    white-space: pre;
  }

  .code li.changed {
    background: rgb(251 191 36 / 0.1);
    box-shadow: inset 2px 0 0 var(--warn);
  }

  .ln {
    width: 2.4ch;
    flex: none;
    text-align: right;
    color: var(--text-faint);
    user-select: none;
  }

  code {
    font-family: inherit;
    color: var(--text-muted);
  }

  .line-owner {
    margin-left: auto;
    font-size: 8.5px;
    color: var(--owner);
    opacity: 0.9;
    white-space: nowrap;
  }

  .state {
    margin: 0;
    padding: 12px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-faint);
    font-size: 11.5px;
  }

  .state--small {
    padding: 10px;
    font-size: 10.5px;
  }

  .state--error {
    color: var(--danger);
    border-color: rgb(248 113 113 / 0.4);
  }

  @media (max-width: 980px) {
    .panes {
      grid-template-columns: minmax(0, 1fr);
    }

    .code {
      max-height: 30vh;
    }
  }
</style>
