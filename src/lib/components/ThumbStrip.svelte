<script lang="ts">
  /** 底部缩略图带(多图时出现):点击切换 / 悬停删除角标 / 滚轮横滚。 */
  import { mediaUrl } from '$lib/api';
  import { imagesStore, removeItem, setActive } from '$lib/state/images.svelte';
  import Icon from './Icon.svelte';

  // $state:列表容器随首张图片才出现(模板 {#if}),bind:this 绑定时触发 effect 重挂监听
  let strip = $state<HTMLDivElement>();

  $effect(() => {
    const el = strip;
    if (!el) return;
    // 竖直滚轮 → 横向滚动缩略图带(passive:false 才能 preventDefault)
    const onWheel = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        e.preventDefault();
        el.scrollLeft += e.deltaY;
      }
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  });
</script>

{#if imagesStore.items.length > 1}
  <div class="strip" bind:this={strip}>
    {#each imagesStore.items as st (st.item.id)}
      <button
        class="thumb"
        class:active={st.item.id === imagesStore.activeId}
        onclick={() => setActive(st.item.id)}
        title={st.item.name}
      >
        {#if st.item.thumbToken}
          <img src={mediaUrl(st.item.thumbToken)} alt="" draggable="false" />
        {:else}
          <span class="thumb-ph"><Icon name="image" size={16} /></span>
        {/if}
        {#if st.phase === 'done'}
          <span class="badge ok"><Icon name="check" size={11} strokeWidth={2.4} /></span>
        {:else if st.phase === 'error'}
          <span class="badge err"><Icon name="x" size={11} strokeWidth={2.4} /></span>
        {:else if st.phase === 'running' || st.phase === 'queued'}
          <span class="badge run"><Icon name="spinner" size={11} spinning /></span>
        {/if}
        <span
          class="rm"
          role="button"
          tabindex="-1"
          title="移除"
          onclick={(e) => {
            e.stopPropagation();
            void removeItem(st.item.id);
          }}
          onkeydown={() => {}}
        >
          <Icon name="x" size={10} strokeWidth={2.2} />
        </span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .strip {
    display: flex;
    gap: 8px;
    align-items: center;
    height: var(--thumbstrip-h);
    padding: 0 12px;
    overflow-x: auto;
    border-top: 1px solid var(--border-subtle);
    background: color-mix(in srgb, var(--bg-panel) 82%, transparent);
    backdrop-filter: blur(var(--backdrop-blur)) saturate(var(--backdrop-saturate));
    flex: none;
  }
  .thumb {
    position: relative;
    flex: none;
    height: 62px;
    aspect-ratio: 4 / 3;
    border-radius: var(--radius-sm);
    overflow: hidden;
    border: 2px solid transparent;
    background: var(--bg-hover);
    transition: all var(--speed-fast) var(--ease-out);
    animation: rise-in var(--speed) var(--ease-out);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .thumb-ph {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-faint);
    background: var(--bg-hover);
  }
  .thumb:hover {
    border-color: var(--border-strong);
  }
  .thumb.active {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .badge {
    position: absolute;
    right: 3px;
    bottom: 3px;
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    color: white;
  }
  .badge.ok {
    background: var(--success);
  }
  .badge.err {
    background: var(--danger);
  }
  .badge.run {
    background: color-mix(in srgb, var(--bg-elevated) 85%, transparent);
    color: var(--accent);
  }
  .rm {
    position: absolute;
    top: 3px;
    right: 3px;
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: rgb(0 0 0 / 0.6);
    color: white;
    opacity: 0;
    transition: opacity var(--speed-fast);
  }
  .thumb:hover .rm {
    opacity: 1;
  }
  .rm:hover {
    background: var(--danger);
  }
</style>
