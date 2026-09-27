<script lang="ts">
  import { ICON_PATHS, type IconName } from '$lib/icons/paths';

  let {
    name,
    size = 18,
    strokeWidth = 1.5,
    spinning = false,
    filled = false,
    class: cls = '',
  }: {
    name: IconName;
    size?: number;
    strokeWidth?: number;
    spinning?: boolean;
    filled?: boolean;
    class?: string;
  } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={filled ? 'currentColor' : 'none'}
  stroke={filled ? 'none' : 'currentColor'}
  stroke-width={filled ? 0 : strokeWidth}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  class={spinning ? `spin ${cls}` : cls}
>
  {#each ICON_PATHS[name] as d (d)}
    <path {d} />
  {/each}
</svg>

<style>
  .spin {
    animation: spin 0.9s linear infinite;
  }
</style>
