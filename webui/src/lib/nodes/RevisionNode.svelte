<script lang="ts">
  import type { NodeProps } from '@xyflow/svelte';
  import type { ResourceNodeData } from '../flow';
  import NodeShell from './NodeShell.svelte';

  let { data, selected }: NodeProps = $props();
  const typed = $derived(data as ResourceNodeData);
</script>

<NodeShell
  node={typed.node}
  {selected}
  policyCount={typed.policyCount}
  failingPolicies={typed.failingPolicies}
  label="Revision"
/>
