<script lang="ts">
  /** 结果面板模式切换:纸张 | 列表(内嵌在各视图工具条里,不做悬浮层)。 */
  import Icon from '$lib/components/Icon.svelte';
  import { app } from '$lib/state/app.svelte';
</script>

<div class="mode-switch" role="group" aria-label="结果视图">
  <button
    class="mode-btn"
    class:active={app.paneMode === 'paper'}
    onclick={() => (app.paneMode = 'paper')}
    title="纸张:按位置重排的可复制文本"
  >
    <Icon name="fileText" size={12} />
    纸张
  </button>
  <button
    class="mode-btn"
    class:active={app.paneMode === 'list'}
    onclick={() => (app.paneMode = 'list')}
    title="列表:阅读顺序文本行"
  >
    <Icon name="type" size={12} />
    列表
  </button>
</div>

<style>
  .mode-switch {
    display: inline-flex;
    gap: 2px;
    padding: 3px;
    border-radius: 999px;
    background: var(--bg-hover);
    border: 1px solid var(--border-subtle);
    flex: none;
  }
  .mode-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 10px;
    border-radius: 999px;
    font-size: 11px;
    color: var(--text-secondary);
    transition: all var(--speed-fast) var(--ease-out);
  }
  .mode-btn:hover {
    color: var(--text-primary);
  }
  .mode-btn.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
    font-weight: 600;
  }
</style>
