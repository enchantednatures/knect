<script lang="ts">
  import { useSvelteFlow } from '@xyflow/svelte';
  import { store } from './store.svelte';

  const { fitView } = useSvelteFlow();

  $effect(() => {
    const drawer = store.drawer;
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
        minZoom: compact ? 0.62 : 0.2,
        maxZoom: 1.1,
      });
    }, 60);
    return () => clearTimeout(timer);
  });
</script>
