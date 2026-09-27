<script lang="ts">
  /** 拖拽导入遮罩(拖入时全窗高亮)。 */
  import Icon from './Icon.svelte';
  import { app } from '$lib/state/app.svelte';
</script>

{#if app.dropActive}
  <div class="drop">
    <div class="frame">
      <Icon name="download" size={34} />
      <p>松开,开始识别</p>
      <span>支持 PNG / JPG / BMP,可多选</span>
    </div>
  </div>
{/if}

<style>
  .drop {
    position: absolute;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--bg-app) 72%, transparent);
    backdrop-filter: blur(6px);
    animation: fade-in var(--speed-fast) var(--ease-out);
    /* 纯信息展示:绝不拦截点击,即使事件丢失(leave/drop 未触发)也不会封死 UI */
    pointer-events: none;
  }
  .frame {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 38px 64px;
    border-radius: var(--radius-xl);
    border: 2px dashed var(--accent);
    background: var(--accent-soft);
    color: var(--accent);
    box-shadow: var(--shadow-lg);
    pointer-events: none;
  }
  .frame p {
    font-size: 16px;
    font-weight: 650;
    color: var(--text-primary);
  }
  .frame span {
    font-size: 12px;
    color: var(--text-secondary);
  }
</style>
