import { describe, expect, it } from 'vitest';
import { emptyRealtime, graphFromTopology } from '../lib/reducer';
import { AppStore } from '../lib/store.svelte';
import { resourceKey } from '../lib/types';
import { topology } from './fixtures';

describe('AppStore layout', () => {
  it('recomputes positions for the visible graph when filters change', () => {
    const store = new AppStore();
    store.realtime = {
      ...emptyRealtime('fixture'),
      graph: graphFromTopology(topology),
    };

    store.syncLayout();
    expect(Object.keys(store.positions).sort()).toEqual(
      store.visibleNodes.map((node) => resourceKey(node.id)).sort(),
    );

    store.setFilter('namespace', 'default');
    expect(Object.keys(store.positions).sort()).toEqual(
      store.visibleNodes.map((node) => resourceKey(node.id)).sort(),
    );
    expect(store.visibleNodes.length).toBeGreaterThan(0);
    expect(store.visibleNodes.length).toBeLessThan(topology.nodes.length);

    store.setFilter('namespace', 'namespace-with-no-nodes');
    expect(store.positions).toEqual({});
  });
});
