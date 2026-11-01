<script lang="ts">
  let {
    path,
    size = 16,
    strokeWidth = 1.6,
    label,
  }: { path: string; size?: number; strokeWidth?: number; label?: string } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden={label ? undefined : 'true'}
  role={label ? 'img' : undefined}
>
  {#if label}<title>{label}</title>{/if}
  <path d={path} />
</svg>
