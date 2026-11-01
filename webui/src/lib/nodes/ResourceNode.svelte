<script lang="ts">
  import type { NodeProps } from '@xyflow/svelte';
  import type { ResourceNodeData } from '../flow';
  import { categoryMeta } from '../theme';
  import NodeShell from './NodeShell.svelte';

  let { data, selected }: NodeProps = $props();
  const typed = $derived(data as ResourceNodeData);
  const label = $derived(categoryMeta(typed.node.category).label);
</script>

<NodeShell
  node={typed.node}
  {selected}
  policyCount={typed.policyCount}
  failingPolicies={typed.failingPolicies}
  {label}
/>
