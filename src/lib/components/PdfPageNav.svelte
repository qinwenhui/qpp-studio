<script lang="ts">
  /** PDF 页码导航:← 上一页 | 12/120 | 下一页 → + 输入跳转。
   *  翻页时自动请求该页渲染,浏览到哪页识别到哪页。 */
  import Icon from '$lib/components/Icon.svelte';
  import { api } from '$lib/api';
  import { toast } from '$lib/state/app.svelte';
  import { getActiveItem, imagesStore } from '$lib/state/images.svelte';

  const active = $derived(getActiveItem());
  const pdf = $derived(active?.pdfPages);
  const isPdf = $derived(!!pdf);
  const current = $derived(pdf?.current ?? 0);
  const count = $derived(pdf?.count ?? 0);

  let jumpTo = $state('');

  async function go(page: number) {
    if (!active || !pdf) return;
    const clamped = Math.max(0, Math.min(count - 1, page));
    if (clamped === current) return;
    pdf.current = clamped;
    // 请求渲染该页(返回新的 media 令牌,画布自动刷新)
    try {
      const result = await api.pdfRenderPage(active.item.id, clamped);
      // 更新条目的媒体令牌(画布 <img> src 变化即刷新)
      active.item.mediaToken = result.mediaToken;
      active.item.w = result.w;
      active.item.h = result.h;
    } catch (e) {
      toast('error', String(e));
    }
  }

  function onJumpKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      const n = parseInt(jumpTo, 10);
      if (!isNaN(n) && n >= 1 && n <= count) {
        void go(n - 1);
        jumpTo = '';
      }
    }
  }

  async function ocrAll() {
    if (!active) return;
    try {
      toast('info', `开始识别全部 ${count} 页…`);
      await api.pdfOcrRange(active.item.id, 0, count - 1);
    } catch (e) {
      toast('error', String(e));
    }
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (!isPdf) return;
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA') return;
    if (e.key === 'ArrowLeft' || e.key === 'PageUp') {
      e.preventDefault();
      void go(current - 1);
    } else if (e.key === 'ArrowRight' || e.key === 'PageDown') {
      e.preventDefault();
      void go(current + 1);
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if isPdf}
  <div class="pdf-nav">
    <button class="nav-btn" disabled={current === 0} onclick={() => go(current - 1)} title="上一页 (←)">
      <Icon name="chevronLeft" size={14} />
    </button>

    <span class="page-indicator">
      <input
        class="page-input"
        type="text"
        bind:value={jumpTo}
        onkeydown={onJumpKeydown}
        placeholder={String(current + 1)}
        size="3"
        title="输入页码,回车跳转"
      />
      <span class="separator">/</span>
      <span class="total">{count}</span>
    </span>

    <button
      class="nav-btn"
      disabled={current >= count - 1}
      onclick={() => go(current + 1)}
      title="下一页 (→)"
    >
      <Icon name="chevronRight" size={14} />
    </button>

    <button
      class="ocr-all-btn"
      onclick={() => ocrAll()}
      title="识别全部 {count} 页"
    >
      <Icon name="layers" size={13} />
      识别全部
    </button>

    <span class="page-hint">{active?.item.name}</span>
  </div>
{/if}

<style>
  .pdf-nav {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 14px;
    border-top: 1px solid var(--border-subtle);
    background: color-mix(in srgb, var(--bg-panel) 85%, transparent);
    backdrop-filter: blur(var(--backdrop-blur)) saturate(var(--backdrop-saturate));
    flex: none;
  }

  .nav-btn {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    transition: all var(--speed-fast) var(--ease-out);
  }
  .nav-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text-primary);
  }
  .nav-btn:disabled {
    opacity: 0.3;
  }

  .page-indicator {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--text-secondary);
  }
  .page-input {
    width: 36px;
    height: 24px;
    text-align: center;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-xs);
    background: var(--bg-input);
    color: var(--text-primary);
    font: inherit;
    outline: none;
  }
  .page-input:focus {
    border-color: var(--accent);
  }
  .separator {
    color: var(--text-faint);
  }
  .total {
    color: var(--text-faint);
    min-width: 28px;
  }

  .page-hint {
    margin-left: auto;
    font-size: 11px;
    color: var(--text-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 260px;
  }

  .ocr-all-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 10px;
    border-radius: 999px;
    font-size: 11.5px;
    color: var(--accent);
    background: var(--accent-soft);
    transition: all var(--speed-fast) var(--ease-out);
    white-space: nowrap;
  }
  .ocr-all-btn:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
</style>
