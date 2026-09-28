<script lang="ts">
  /** PDF 页码导航 + 后台识别进度指示。
   *  PDF 拖入时后台自动识别全部页,进度由 App.svelte 统一写进
   *  pdfPages.ocrDone(单一事件源),这里只做展示。
   *  翻页 = 渲染新页 + 同步该页结果(已识别的页即时显示)。 */
  import Icon from '$lib/components/Icon.svelte';
  import { api } from '$lib/api';
  import { toast } from '$lib/state/app.svelte';
  import { getActiveItem } from '$lib/state/images.svelte';

  const active = $derived(getActiveItem());
  const pdf = $derived(active?.pdfPages);
  const isPdf = $derived(!!pdf);
  const current = $derived(pdf?.current ?? 0);
  const count = $derived(pdf?.count ?? 0);
  const ocrDone = $derived(pdf?.ocrDone ?? 0);
  const ocrRunning = $derived(ocrDone < count && count > 0);

  let jumpTo = $state('');

  async function go(page: number) {
    if (!active || !pdf) return;
    const clamped = Math.max(0, Math.min(count - 1, page));
    if (clamped === current) return;
    pdf.current = clamped;
    try {
      const result = await api.pdfRenderPage(active.item.id, clamped);
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

  async function extractAll() {
    if (!active) return;
    try {
      toast('info', '正在从文本层提取(毫秒级)…');
      await api.pdfExtractAll(active.item.id);
      toast('success', '提取完成');
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

    <!-- 识别进度:后台自动识别,这里只显示进度 -->
    {#if ocrRunning}
      <span class="ocr-progress">
        <span class="spin"><Icon name="spinner" size={11} spinning /></span>
        <span class="done">{ocrDone}/{count}</span>
        <span class="mini-bar">
          <span class="mini-fill" style="width: {count ? (ocrDone / count) * 100 : 0}%"></span>
        </span>
      </span>
    {:else if ocrDone >= count && count > 0}
      <span class="ocr-done">
        <Icon name="check" size={11} />
        已完成
      </span>
    {/if}

    {#if active?.canExtract}
      <button
        class="extract-btn"
        onclick={() => extractAll()}
        title="此 PDF 有文本层,直接提取(毫秒级,跳过 OCR)"
      >
        <Icon name="zap" size={13} />
        一键提取
      </button>
    {/if}

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

  .ocr-progress {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    font-family: var(--font-mono);
    color: var(--accent);
  }
  .ocr-progress .done {
    white-space: nowrap;
  }
  .ocr-progress .spin {
    display: inline-flex;
  }
  .mini-bar {
    width: 60px;
    height: 4px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    overflow: hidden;
  }
  .mini-fill {
    display: block;
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    transition: width 0.3s var(--ease-out);
  }

  .ocr-done {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    color: var(--success);
  }

  .extract-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 10px;
    border-radius: 999px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--accent-contrast);
    background: var(--accent);
    transition: all var(--speed-fast) var(--ease-out);
    white-space: nowrap;
  }
  .extract-btn:hover {
    filter: brightness(1.1);
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
</style>
