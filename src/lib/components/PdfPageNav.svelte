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
  const ocrState = $derived(pdf?.ocrState ?? 'idle');
  const onDemand = $derived(!!pdf?.onDemand);
  /** 当前页单页识别中(按需模式;整册跑的时候用进度条) */
  const pageBusy = $derived(active?.phase === 'running' || active?.phase === 'queued');

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
      if (!result.recognized) {
        // 未识别页:立即清掉上一页的陈旧结果(否则画布是新页、框线/右栏
        // 还是旧页的,视觉上"结果对不上"),右栏进入识别中
        active.outcome = undefined;
        active.phase = 'queued';
        // 整册没在跑(按需/已暂停) → 单页识别;舰队跑着则等它覆盖
        if (ocrState !== 'running') {
          active.phase = 'running';
          void api
            .pdfRecognizePage(active.item.id, clamped)
            .catch((e) => toast('error', String(e)));
        }
      }
    } catch (e) {
      toast('error', String(e));
    }
  }

  /** 暂停:静默中断,返回当前完成数,缺失页留给「继续」 */
  async function pauseOcr() {
    if (!active || !pdf) return;
    try {
      const completed = await api.pdfPause(active.item.id);
      pdf.ocrState = 'paused';
      if (completed != null) pdf.ocrDone = completed;
    } catch (e) {
      toast('error', String(e));
    }
  }

  /** 继续 / 按需模式的「识别全部」:只跑缺失页。
   *  无缺失页时后端返回 false——直接落定「已完成」,别乐观等事件。 */
  async function resumeOcr() {
    if (!active || !pdf) return;
    try {
      const hadWork = await api.pdfResume(active.item.id);
      pdf.ocrState = hadWork ? 'running' : 'done';
      // 注意不碰 onDemand:暂停后翻到未识别页仍要单页识别,
      // onDemand 只控制 UI 展示(徽标/按钮),不控制识别触发
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

    <!-- 识别控制 + 进度(状态机:按需 idle / 运行 running / 已暂停 paused / 完成 done) -->
    {#if onDemand && (ocrState === 'idle' || ocrState === 'done')}
      {#if ocrDone >= count && count > 0}
        <!-- 按需翻完全本:与整册完成同款标记,不再显示「识别全部」 -->
        <span class="ocr-done">
          <Icon name="check" size={11} />
          已完成
        </span>
      {:else}
        {#if pageBusy}
          <span class="ocr-progress" title="按需模式:翻到哪页识别哪页">
            <span class="spin"><Icon name="spinner" size={11} spinning /></span>
            <span>本页识别中</span>
          </span>
        {:else}
          <span class="ocr-badge" title="大文档默认按需:翻到哪页识别哪页">按需</span>
        {/if}
        <button
          class="ctrl-btn"
          onclick={() => resumeOcr()}
          title="识别全部 {count} 页(可随时暂停)">
          <Icon name="layers" size={13} />
          识别全部
        </button>
      {/if}
    {:else if ocrState === 'running' && count > 0}
      <span class="ocr-progress">
        <span class="spin"><Icon name="spinner" size={11} spinning /></span>
        <span class="done">{ocrDone}/{count}</span>
        <span class="mini-bar">
          <span class="mini-fill" style="width: {count ? (ocrDone / count) * 100 : 0}%"></span>
        </span>
      </span>
      <button
        class="ctrl-btn"
        onclick={() => pauseOcr()}
        title="暂停识别(已完成的保留,随时可继续)">
        <Icon name="pause" size={12} />
        暂停
      </button>
    {:else if ocrState === 'paused'}
      <span class="ocr-paused">
        已暂停 {ocrDone}/{count}
      </span>
      <button
        class="ctrl-btn accent"
        onclick={() => resumeOcr()}
        title="继续识别剩余 {count - ocrDone} 页">
        <Icon name="play" size={12} />
        继续
      </button>
    {:else if ocrState === 'done' && count > 0}
      <span class="ocr-done">
        <Icon name="check" size={11} />
        已完成
      </span>
    {/if}

    {#if active?.canExtract}
      <span class="ocr-badge" title="数字原生 PDF:文本层直提,无需 OCR">直提</span>
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

  .ocr-badge {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 8px;
    border-radius: 999px;
    font-size: 11px;
    color: var(--text-secondary);
    background: color-mix(in srgb, var(--text-faint) 14%, transparent);
    white-space: nowrap;
  }
  .ocr-paused {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    font-family: var(--font-mono);
    color: var(--warning, #d29a4a);
    white-space: nowrap;
  }
  .ctrl-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 10px;
    border-radius: 999px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--accent);
    background: var(--accent-soft);
    transition: all var(--speed-fast) var(--ease-out);
    white-space: nowrap;
  }
  .ctrl-btn:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .ctrl-btn.accent {
    color: var(--accent-contrast);
    background: var(--accent);
  }
  .ctrl-btn.accent:hover {
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
