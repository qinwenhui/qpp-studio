<script lang="ts">
  /** 识别结果面板:阅读序文本行 + 置信度 + 双向联动 + 搜索/复制/导出。 */
  import Icon from '$lib/components/Icon.svelte';
  import ModeSwitch from '$lib/components/ModeSwitch.svelte';
  import { api } from '$lib/api';
  import { app, toast } from '$lib/state/app.svelte';
  import { getActiveItem, imagesStore, reRecognize } from '$lib/state/images.svelte';
  import type { TextLine } from '$lib/types';

  let search = $state('');

  const active = $derived(getActiveItem());
  const lines = $derived(active?.outcome?.result?.lines ?? []);
  const filtered = $derived(
    search.trim()
      ? lines
          .map((l, i) => ({ l, i }))
          .filter(({ l }) => l.text.toLowerCase().includes(search.trim().toLowerCase()))
      : lines.map((l, i) => ({ l, i })),
  );

  const doneCount = $derived(imagesStore.items.filter((i) => i.phase === 'done').length);

  function hover(i: number | null) {
    app.focus = i !== null && active ? { id: active.item.id, line: i } : null;
  }

  function select(i: number) {
    app.selected = { id: active!.item.id, line: i };
  }

  async function copyText(text: string, tip = '已复制') {
    try {
      await api.copyText(text);
      toast('success', tip);
    } catch (e) {
      toast('error', String(e));
    }
  }

  function allText(): string {
    return lines
      .map((l) => l.text)
      .filter(Boolean)
      .join('\n');
  }

  /** 行的「字面尺寸」:优先逐字盒短边中位(真实字面,与纸张视图同口径);
   *  无 chars(直提)时用行框左右边长均值 ×0.78(抗旋转,行框含行距余量)。 */
  function charSize(l: TextLine): number {
    const shorts = (l.chars ?? [])
      .filter((c) => c.text !== ' ')
      .map((c) => {
        const xs = c.pts.map((p) => p[0]);
        const ys = c.pts.map((p) => p[1]);
        const w = Math.max(...xs) - Math.min(...xs);
        const h = Math.max(...ys) - Math.min(...ys);
        return Math.min(w, h);
      })
      .sort((a, b) => a - b);
    if (shorts.length) return shorts[shorts.length >> 1];
    const eL = Math.hypot(l.pts[3][0] - l.pts[0][0], l.pts[3][1] - l.pts[0][1]);
    const eR = Math.hypot(l.pts[2][0] - l.pts[1][0], l.pts[2][1] - l.pts[1][1]);
    return Math.max(1, ((eL + eR) / 2) * 0.78);
  }

  /** 版式感知 Markdown:字面尺寸相对正文基准判标题层级(1.9×/1.45×/1.18×
   *  ≈ 文档常规 H1/H2/H3 之比),垂直净间隙突变分段落(与后端同规则)。 */
  function toMarkdown(): string {
    const metrics = lines.map((l) => {
      const ys = l.pts.map((p) => p[1]);
      return { size: charSize(l), cy: (Math.max(...ys) + Math.min(...ys)) / 2 };
    });
    const sorted = metrics.map((m) => m.size).sort((a, b) => a - b);
    // 下中位:标题行总是更大,上中位会被标题污染
    const body = Math.max(1, sorted[(sorted.length - 1) >> 1] ?? 1);
    let out = '';
    let prev: { cy: number; size: number } | null = null;
    lines.forEach((l, i) => {
      const text = l.text.trim();
      if (!text) return;
      const { size, cy } = metrics[i];
      if (prev) {
        const gap = cy - prev.cy - (prev.size + size) / 2;
        if (gap > body * 0.75) out += '\n';
      }
      const level =
        size >= body * 1.9 ? 1 : size >= body * 1.45 ? 2 : size >= body * 1.18 ? 3 : 0;
      out += (level ? '#'.repeat(level) + ' ' : '') + text + '\n';
      prev = { cy, size };
    });
    return out;
  }

  async function exportAs(fmt: 'txt' | 'json' | 'md') {
    if (!active?.outcome?.ok) return;
    const base = active.item.name.replace(/\.[^.]+$/, '');
    const content =
      fmt === 'txt'
        ? allText()
        : fmt === 'md'
          ? toMarkdown()
          : JSON.stringify(
              {
                image: { name: active.item.name, width: active.item.w, height: active.item.h },
                timings: active.outcome.result?.timings,
                lines: lines.map((l) => ({ text: l.text, confidence: l.confidence, box: l.pts })),
              },
              null,
              2,
            );
    try {
      const path = await api.exportContent(content, fmt, `${base}.${fmt}`);
      toast('success', `已导出: ${path}`);
    } catch (e) {
      if (String(e) !== '已取消') toast('error', String(e));
    }
  }

  async function exportPdf(fmt: 'txt' | 'json' | 'md') {
    if (!active || active.item.origin !== 'pdf') return;
    try {
      const content = await api.pdfExportMerged(active.item.id, fmt);
      const base = active.item.name.replace(/\.[^.]+$/, '');
      const path = await api.exportContent(content, fmt, `${base}-全文.${fmt}`);
      toast('success', `已导出: ${path}`);
    } catch (e) {
      if (String(e) !== '已取消') toast('error', String(e));
    }
  }

  /** 本页强制 OCR(直提页上):走后端按需执行线程,结果写回该页,
   *  翻回本页不再回退成直提结果 */
  async function reOcrPage() {
    if (!active || !active.pdfPages) return;
    const page = active.pdfPages.current;
    active.phase = 'running';
    active.outcome = undefined;
    try {
      await api.pdfOcrPage(active.item.id, page);
    } catch (e) {
      toast('error', String(e));
    }
  }

  async function cancelBatch() {
    await api.batchCancel();
    toast('info', '已请求取消,进行中的图片会完成当前张');
  }
</script>

<div class="results">
  <div class="head">
    <div class="title-row">
      <ModeSwitch />
      <span class="title" title={active?.item.name}>{active?.item.name ?? '识别结果'}</span>
      {#if active?.outcome?.result?.extracted}
        <span class="src-badge extract" title="结果来自 PDF 文本层直提(毫秒级,无框线)">直提</span>
        {#if active.item.origin === 'pdf'}
          <button
            class="badge-btn"
            onclick={() => reOcrPage()}
            title="这一页改用 OCR 识别(可得到精确框线,约零点几秒)">本页改用OCR</button>
        {/if}
      {:else if lines.length}
        <span class="src-badge" title="结果来自 OCR 识别">OCR</span>
      {/if}
      {#if active && (active.phase === 'done' || active.phase === 'error')}
        <button
          class="icon-btn rerun-btn"
          title="重新识别这张(结果与耗时原地更新)"
          onclick={() => reRecognize(active.item.id)}
        >
          <Icon name="refresh" size={14} />
          重新识别
        </button>
      {/if}
      {#if imagesStore.items.length > 1}
        <span class="count">{doneCount}/{imagesStore.items.length}</span>
      {/if}
      {#if active?.outcome?.result?.numDetRetried}
        <span
          class="retry-badge"
          title="识别置信度低于阈值的行,引擎自动对其区域做了二次检测(非效率下降)"
        >
          <Icon name="refresh" size={10} />
          区域重试 {active.outcome.result.numDetRetried} 次
        </span>
      {/if}
    </div>
    {#if lines.length}
      <div class="tools">
        <div class="search">
          <Icon name="search" size={13} />
          <input class="input" placeholder="搜索文本…" bind:value={search} />
        </div>
      </div>
    {/if}
  </div>

  {#if app.batchRunning}
    {@const batchPct = imagesStore.items.length > 0 ? Math.round((doneCount / imagesStore.items.length) * 100) : 0}
    <div class="batch-bar">
      <span class="spin"><Icon name="spinner" size={13} spinning /></span>
      批量识别中 {doneCount}/{imagesStore.items.length}
      <div class="mini-bar"><div class="mini-fill" style="width: {batchPct}%"></div></div>
      <button class="link" onclick={cancelBatch}>取消</button>
    </div>
  {/if}

  <div class="list" role="list">
    {#if !active}
      <div class="placeholder">暂无图片</div>
    {:else if active.phase === 'queued' || active.phase === 'running'}
      <div class="placeholder">
        <span class="spin"><Icon name="spinner" size={15} spinning /></span>
        识别中…
      </div>
    {:else if active.phase === 'error'}
      <div class="placeholder err selectable">{active.outcome?.error ?? '识别失败'}</div>
    {:else if !lines.length}
      <div class="placeholder">未检测到文字</div>
    {:else}
      {#each filtered as { l, i } (i)}
        <div
          class="row"
          class:hover={app.focus?.line === i && app.focus?.id === active.item.id}
          class:selected={app.selected?.line === i && app.selected?.id === active.item.id}
          role="listitem"
          onmouseenter={() => hover(i)}
          onmouseleave={() => hover(null)}
          onclick={() => select(i)}
        >
          <span class="idx">{i + 1}</span>
          <span class="text selectable">{l.text || '⌁'}</span>
          <button
            class="copy"
            title="复制此行"
            onclick={(e) => {
              e.stopPropagation();
              void copyText(l.text, '已复制该行');
            }}
          >
            <Icon name="copy" size={13} />
          </button>
          <span class="conf" style="color: hsl({8 + Math.min(1, Math.max(0, (l.confidence - 0.5) / 0.45)) * 132} 64% 52%)">
            {#if l.retried}
              <span class="retried-mark" title="本行经过区域重试(第二遍读取);若置信度仍低请人工复核">↻</span>
            {/if}
            {(l.confidence * 100).toFixed(0)}
          </span>
        </div>
      {/each}
    {/if}
  </div>

  {#if lines.length}
    <div class="foot">
      <button class="btn" onclick={() => copyText(allText(), '已复制全部文本')}>
        <Icon name="copy" size={14} />
        复制全部
      </button>
      <button class="btn" onclick={() => exportAs('txt')}>
        <Icon name="download" size={14} />
        TXT
      </button>
      <button class="btn" onclick={() => exportAs('md')} title="版式感知:行高判标题层级、行距分段落">
        <Icon name="download" size={14} />
        MD
      </button>
      <button class="btn" onclick={() => exportAs('json')}>
        <Icon name="download" size={14} />
        JSON
      </button>
      {#if active?.item.origin === 'pdf'}
        <button class="btn" onclick={() => exportPdf('txt')} title="按页序合并全部已识别页">
          <Icon name="layers" size={14} />
          合并 TXT
        </button>
        <button class="btn" onclick={() => exportPdf('md')} title="整册版式感知 Markdown(带页分隔)">
          <Icon name="layers" size={14} />
          合并 MD
        </button>
      {/if}
      <span class="ms" title="引擎端到端耗时">
        {active?.outcome?.result?.timings?.totalMs?.toFixed(1) ?? '—'} ms
      </span>
    </div>
  {/if}
</div>

<style>
  .results {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }
  .head {
    padding: 12px 12px 8px;
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    flex-direction: column;
    gap: 9px;
    flex: none;
  }
  .title-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .title {
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .src-badge {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 8px;
    border-radius: 999px;
    flex: none;
    font-size: 10.5px;
    font-weight: 600;
    color: var(--text-secondary);
    background: color-mix(in srgb, var(--text-faint) 14%, transparent);
  }
  .src-badge.extract {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .badge-btn {
    display: inline-flex;
    align-items: center;
    height: 20px;
    padding: 0 8px;
    border-radius: 999px;
    flex: none;
    font-size: 10.5px;
    font-weight: 600;
    color: var(--accent);
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    transition: all var(--speed-fast) var(--ease-out);
    white-space: nowrap;
  }
  .badge-btn:hover {
    background: var(--accent-soft);
  }
  .count {
    margin-left: auto;
    font-size: 11px;
    font-family: var(--font-mono);
    color: var(--text-faint);
    flex: none;
  }
  .retried-mark {
    font-size: 9px;
    margin-right: 1px;
    opacity: 0.85;
  }
  .retry-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 8px;
    border-radius: 999px;
    font-size: 10.5px;
    color: var(--info);
    background: color-mix(in srgb, var(--info) 12%, transparent);
    flex: none;
  }
  .rerun-btn {
    width: auto;
    gap: 4px;
    padding: 0 8px;
    height: 24px;
    font-size: 11.5px;
    flex: none;
  }
  .rerun-btn:hover {
    color: var(--accent);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--text-faint);
  }
  .search .input {
    flex: 1;
    height: 28px;
  }

  .batch-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    font-size: 12px;
    color: var(--accent);
    background: var(--accent-soft);
    flex: none;
  }
  .batch-bar .link {
    margin-left: auto;
    color: var(--text-secondary);
    text-decoration: underline;
    font-size: 11.5px;
  }
  .mini-bar {
    flex: 1;
    height: 4px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    overflow: hidden;
  }
  .mini-fill {
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    transition: width var(--speed-slow) var(--ease-out);
  }

  .list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 8px;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px 7px;
    border-radius: var(--radius-sm);
    cursor: default;
    transition: background var(--speed-fast);
    animation: fade-in var(--speed) var(--ease-out);
  }
  .row:hover,
  .row.hover {
    background: var(--bg-hover);
  }
  .row.selected {
    background: var(--accent-soft);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .idx {
    min-width: 18px;
    text-align: right;
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--text-faint);
    padding-top: 3px;
    flex: none;
  }
  .text {
    flex: 1;
    font-size: 12.5px;
    color: var(--text-primary);
    word-break: break-all;
    line-height: 1.5;
  }
  .copy {
    opacity: 0;
    color: var(--text-faint);
    width: 22px;
    height: 22px;
    border-radius: var(--radius-xs);
    display: grid;
    place-items: center;
    flex: none;
    transition: all var(--speed-fast);
  }
  .row:hover .copy {
    opacity: 1;
  }
  .copy:hover {
    color: var(--accent);
    background: var(--bg-active);
  }
  .conf {
    font-size: 10.5px;
    font-family: var(--font-mono);
    padding-top: 3px;
    flex: none;
    opacity: 0.9;
  }

  .placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 120px;
    color: var(--text-faint);
    font-size: 12.5px;
  }
  .placeholder.err {
    color: var(--danger);
    padding: 0 14px;
    text-align: center;
    word-break: break-all;
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-subtle);
    flex: none;
  }
  .foot .ms {
    margin-left: auto;
    font-size: 11.5px;
    font-family: var(--font-mono);
    font-weight: 700;
    color: var(--success);
  }
</style>
