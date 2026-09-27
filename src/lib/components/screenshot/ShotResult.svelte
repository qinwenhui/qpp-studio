<script lang="ts">
  /** 截图结果迷你弹窗:挂在 shot-result.html。 */
  import { onMount } from 'svelte';
  import Icon from '$lib/components/Icon.svelte';
  import LogoMark from '$lib/components/LogoMark.svelte';
  import { api } from '$lib/api';
  import { toast } from '$lib/state/app.svelte';
  import type { ItemOutcome } from '$lib/types';

  let data = $state<ItemOutcome | null>(null);

  onMount(async () => {
    data = await api.shotResultData();
  });

  const lines = $derived(
    (data?.outcome.result?.lines ?? []).filter((l) => l.text).slice(0, 6),
  );
  const more = $derived(
    (data?.outcome.result?.lines ?? []).filter((l) => l.text).length - lines.length,
  );

  async function copyAll() {
    if (!data?.outcome.result) return;
    const text = data.outcome.result.lines
      .map((l) => l.text)
      .filter(Boolean)
      .join('\n');
    try {
      await api.copyText(text);
      toast('success', '已复制');
    } catch (e) {
      toast('error', String(e));
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      void api.closeShotResult();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="popup">
  <div class="head">
    <LogoMark size={18} />
    <span class="t">识别完成</span>
    {#if data?.outcome.result}
      <span class="ms">{data.outcome.result.timings.totalMs.toFixed(0)}ms · {data.outcome.result.lines.length} 行</span>
    {/if}
    <button class="icon-btn close" onclick={() => api.closeShotResult()} title="关闭 (Esc)">
      <Icon name="x" size={13} />
    </button>
  </div>

  {#if !data}
    <div class="loading"><Icon name="spinner" size={15} spinning /></div>
  {:else if !data.outcome.ok}
    <div class="err">{data.outcome.error ?? '识别失败'}</div>
  {:else if !lines.length}
    <div class="empty">未检测到文字</div>
  {:else}
    <div class="lines selectable">
      {#each lines as l, i (i)}
        <p>{l.text}</p>
      {/each}
      {#if more > 0}
        <p class="more">…还有 {more} 行,查看详情</p>
      {/if}
    </div>
    <div class="actions">
      <button class="btn primary" onclick={copyAll}>
        <Icon name="copy" size={13} />
        复制文本
      </button>
      <button class="btn" onclick={() => api.focusMain()}>
        查看详情
        <Icon name="arrowRight" size={13} />
      </button>
    </div>
  {/if}
</div>

<style>
  :global(html),
  :global(body) {
    background: transparent;
    overflow: hidden;
    user-select: none;
    font-family: system-ui, 'Segoe UI', 'Microsoft YaHei UI', sans-serif;
  }
  .popup {
    display: flex;
    flex-direction: column;
    height: 100vh;
    border-radius: 14px;
    overflow: hidden;
    background: rgb(22 26 33 / 0.94);
    color: #e8edf4;
    border: 1px solid rgb(255 255 255 / 0.12);
    box-shadow: 0 18px 50px rgb(0 0 0 / 0.5);
    animation: rise-in 180ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 10px 10px 14px;
    border-bottom: 1px solid rgb(255 255 255 / 0.08);
    flex: none;
  }
  .head .t {
    font-size: 12.5px;
    font-weight: 650;
  }
  .head .ms {
    font-size: 11px;
    color: #2dd4a7;
    font-family: 'Cascadia Code', Consolas, monospace;
    margin-left: auto;
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    color: #98a5b8;
  }
  .icon-btn:hover {
    background: rgb(255 255 255 / 0.08);
    color: #fff;
  }
  .loading,
  .empty,
  .err {
    flex: 1;
    display: grid;
    place-items: center;
    color: #98a5b8;
    font-size: 12.5px;
    padding: 0 16px;
    text-align: center;
  }
  .err {
    color: #f87171;
    word-break: break-all;
  }
  .lines {
    flex: 1;
    overflow-y: auto;
    padding: 10px 14px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12.5px;
    line-height: 1.5;
  }
  .lines p {
    white-space: pre-wrap;
    word-break: break-all;
  }
  .lines .more {
    color: #5d6b80;
    font-size: 11.5px;
  }
  .actions {
    display: flex;
    gap: 8px;
    padding: 10px 14px 12px;
    border-top: 1px solid rgb(255 255 255 / 0.08);
    flex: none;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 14px;
    border-radius: 8px;
    font-size: 12.5px;
    color: #98a5b8;
  }
  .btn:hover {
    background: rgb(255 255 255 / 0.07);
    color: #fff;
  }
  .btn.primary {
    background: #2dd4a7;
    color: #062018;
    font-weight: 600;
  }
  .btn.primary:hover {
    background: #24b98f;
  }
  @keyframes rise-in {
    from {
      opacity: 0;
      transform: translateY(10px) scale(0.98);
    }
  }
</style>
