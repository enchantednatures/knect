<script lang="ts">
  import { useSvelteFlow } from '@xyflow/svelte';
  import { store } from './store.svelte';

  const { fitView } = useSvelteFlow();

  $effect(() => {
    const drawer = store.drawer;
    const layoutEpoch = store.layoutEpoch;
    const padding =
      drawer === 'inspector' || drawer === 'tap'
        ? { top: 0.08, right: 0.08, bottom: 0.08, left: 0.08 }
        : { top: 0.12, right: 0.12, bottom: 0.12, left: 0.12 };
    const compact =
      typeof window !== 'undefined' && window.matchMedia('(max-width: 767px)').matches;
    const timer = setTimeout(() => {
      void fitView({
        padding,
        duration: 220,
        minZoom: compact ? 0.3 : 0.05,
        maxZoom: 1.1,
      });
    }, layoutEpoch === 0 ? 60 : 140);
    return () => clearTimeout(timer);
  });
</script>
