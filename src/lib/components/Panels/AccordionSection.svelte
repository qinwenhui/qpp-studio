<script lang="ts">
  /** 手风琴小节(参考毛玻璃设置面板的分组样式)。 */
  import Icon from '$lib/components/Icon.svelte';
  let {
    title,
    subtitle = '',
    open = true,
    children,
  }: {
    title: string;
    subtitle?: string;
    open?: boolean;
    children: import('svelte').Snippet;
  } = $props();

  let expanded = $state(open);
</script>

<section class="acc">
  <button class="head" onclick={() => (expanded = !expanded)} aria-expanded={expanded}>
    <Icon name="chevronRight" size={14} class="chev" />
    <span class="title">{title}</span>
    {#if subtitle}
      <span class="subtitle">{subtitle}</span>
    {/if}
  </button>
  {#if expanded}
    <div class="body">
      {@render children()}
    </div>
  {/if}
</section>

<style>
  .acc {
    border-bottom: 1px solid var(--border-subtle);
  }
  .acc:first-child {
    border-top-left-radius: var(--radius-md);
    border-top-right-radius: var(--radius-md);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 11px 12px;
    color: var(--text-primary);
    font-size: 12.5px;
    font-weight: 600;
    text-align: left;
    transition: background var(--speed-fast);
  }
  .head:hover {
    background: var(--bg-hover);
  }
  .head :global(.chev) {
    color: var(--text-faint);
    transition: transform var(--speed) var(--ease-out);
    flex: none;
  }
  .head[aria-expanded='true'] :global(.chev) {
    transform: rotate(90deg);
  }
  .subtitle {
    margin-left: auto;
    font-size: 11px;
    font-weight: 400;
    color: var(--text-faint);
  }
  .body {
    padding: 4px 12px 14px 30px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    animation: rise-in var(--speed-fast) var(--ease-out);
  }
</style>
