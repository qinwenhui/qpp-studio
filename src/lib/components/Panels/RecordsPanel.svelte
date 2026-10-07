<script lang="ts">
  /** 识别记录:本次会话的任务队列(实时状态)+ 历史记录(持久)合一。 */
  import Icon from '$lib/components/Icon.svelte';
  import { api, mediaUrl } from '$lib/api';
  import { app, toast } from '$lib/state/app.svelte';
  import { imagesStore, removeItem, setActive, clearAll, reRecognize } from '$lib/state/images.svelte';
  import {
    historyStore,
    refreshHistory,
    deleteHistory,
    clearHistory,
    timeAgo,
  } from '$lib/state/history.svelte';
  import { onMount } from 'svelte';

  // 识别完成后的历史刷新统一在 App.svelte 的 ocr://item-done 里做(全局一份,
  // 面板收起时也生效)。这里只需在每次打开/切到本面板时拉一次最新快照。
  onMount(() => {
    void refreshHistory();
  });

  const total = $derived(imagesStore.items.length);
  const done = $derived(imagesStore.items.filter((i) => i.phase === 'done').length);
  const failed = $derived(imagesStore.items.filter((i) => i.phase === 'error').length);
  const finished = $derived(done + failed);
  const pct = $derived(total > 0 ? Math.round((finished / total) * 100) : 0);

  let now = $state(Date.now());
  $effect(() => {
    if (!app.batchRunning) return;
    const t = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(t);
  });
  /** 批次墙钟:从批量开始到结束的真实时间(含解码/调度,非单张引擎耗时之和) */
  const wallS = $derived(
    app.batchStartedAt > 0
      ? Math.max(0, ((app.batchRunning ? now : app.batchEndedAt) - app.batchStartedAt) / 1000)
      : 0,
  );
  /** 本批引擎端耗时合计(ms) — 与墙钟对比即得并行收益 */
  const engineSumS = $derived(
    imagesStore.items.reduce(
      (a, i) => a + (i.outcome?.result?.timings.totalMs ?? 0),
      0,
    ) / 1000,
  );
  const throughput = $derived(wallS > 0.05 ? done / wallS : 0);
  const speedup = $derived(wallS > 0.05 && engineSumS > 0 ? engineSumS / wallS : 0);
  const batchDone = $derived(!app.batchRunning && app.batchStartedAt > 0 && total > 1);

  function fmtSecs(s: number): string {
    return s >= 60 ? `${Math.floor(s / 60)}m${(s % 60).toFixed(0)}s` : `${s.toFixed(s < 10 ? 1 : 0)}s`;
  }

  const ORIGIN_ICON: Record<string, 'image' | 'clipboard' | 'crop'> = {
    file: 'image',
    clipboard: 'clipboard',
    screenshot: 'crop',
  };
  const PHASE: Record<string, { label: string; cls: string }> = {
    new: { label: '待识别', cls: 'new' },
    queued: { label: '排队中', cls: 'queued' },
    running: { label: '识别中', cls: 'running' },
    done: { label: '完成', cls: 'done' },
    error: { label: '失败', cls: 'error' },
  };

  function openItem(id: string) {
    setActive(id);
  }

  async function reopen(id: string) {
    try {
      const { item, outcome } = await api.historyReopen(id);
      const { addItemWithOutcome } = await import('$lib/state/images.svelte');
      addItemWithOutcome(item, outcome);
    } catch (e) {
      toast('error', String(e));
    }
  }

  async function cancelBatch() {
    await api.batchCancel();
    toast('info', '已请求取消,进行中的图片会完成当前张');
  }
</script>

<div class="records">
  <div class="head">
    <div class="title-row">
      {#if total > 0}
        <span class="count">{finished}/{total}{failed > 0 ? ` · ${failed} 失败` : ''}</span>
      {/if}
      {#if total > 0}
        <button class="btn danger" onclick={() => clearAll()} title="清空本次会话">
          <Icon name="trash" size={13} />
          清空
        </button>
      {/if}
    </div>
    {#if total > 0}
      <div class="progress">
        <div class="bar">
          <div class="fill done-fill" style="width: {pct}%"></div>
        </div>
        <span class="pct">{pct}%</span>
        <span class="elapsed" class:running={app.batchRunning}>
          {#if app.batchRunning}
            <Icon name="spinner" size={11} spinning />
          {:else if total === 1 && imagesStore.items.length === 1 && imagesStore.items[0].outcome?.result?.timings}
            {fmtSecs(imagesStore.items[0].outcome.result.timings.totalMs / 1000)}
          {:else}
            {fmtSecs(wallS)}
          {/if}
        </span>
        {#if app.batchRunning}
          <button class="link" onclick={cancelBatch}>取消</button>
        {/if}
      </div>
      {#if batchDone}
        <div class="wallstats" title="墙钟 = 批次真实总耗时(含解码与调度);加速比 = 引擎耗时合计 ÷ 墙钟">
          墙钟 {fmtSecs(wallS)} · {throughput.toFixed(1)} 张/s · 引擎合计 {engineSumS.toFixed(1)}s{#if speedup > 1.05} · 并行加速 {speedup.toFixed(1)}×{/if}
        </div>
      {/if}
    {/if}
  </div>

  <div class="list" role="list">
    {#if total === 0 && historyStore.entries.length === 0}
      <div class="placeholder">拖入、粘贴或打开图片后,这里就是你的识别记录</div>
    {/if}

    {#if total > 0}
      <div class="section-head">
        <span class="section-title">当前识别</span>
        <span class="section-count">{finished}/{total}</span>
      </div>
    {/if}

    {#each imagesStore.items as st, i (st.item.id)}
      <div
        class="row"
        class:active={st.item.id === imagesStore.activeId}
        role="listitem"
        onclick={() => openItem(st.item.id)}
        onkeydown={(e) => e.key === 'Enter' && openItem(st.item.id)}
      >
        <span class="idx">{i + 1}</span>
        <div class="thumb">
          {#if st.item.thumbToken}
            <img src={mediaUrl(st.item.thumbToken)} alt="" draggable="false" />
          {:else}
            <Icon name="image" size={16} />
          {/if}
          {#if st.phase === 'running' || st.phase === 'queued'}
            <span class="spin"><Icon name="spinner" size={12} spinning /></span>
          {/if}
        </div>
        <div class="info">
          <div class="name" title={st.item.path}>{st.item.name}</div>
          <div class="meta">
            <Icon name={ORIGIN_ICON[st.item.origin] ?? 'image'} size={10.5} />
            {st.item.w}×{st.item.h}
            {#if st.phase === 'done' && st.outcome?.result}
              · {st.outcome.result.lines.length} 行 · <b class="ms">{st.outcome.result.timings.totalMs.toFixed(0)}ms</b>
            {/if}
          </div>
        </div>
        <span class={`status ${PHASE[st.phase]?.cls ?? ''}`}>
          {#if st.phase === 'error'}
            <span class="err-text" title={st.outcome?.error ?? ''}>
              {st.outcome?.error?.slice(0, 30) ?? '失败'}
            </span>
          {:else}
            {PHASE[st.phase]?.label ?? st.phase}
          {/if}
        </span>
        <button
          class="rm rerun"
          title="重新识别(队列保持不动)"
          onclick={(e) => {
            e.stopPropagation();
            void reRecognize(st.item.id);
          }}
        >
          <Icon name="refresh" size={12} />
        </button>
        <button
          class="rm"
          title="移除"
          onclick={(e) => {
            e.stopPropagation();
            void removeItem(st.item.id);
          }}
        >
          <Icon name="x" size={11} />
        </button>
      </div>
    {/each}

    {#if historyStore.entries.length > 0}
      <div class="section-head">
        <span class="section-title">历史</span>
        <button class="link" onclick={() => clearHistory()}>清空历史</button>
      </div>
      {#each historyStore.entries as e (e.id)}
        {@const already = imagesStore.items.some((i) => i.item.id === e.id)}
        <div
          class="row history"
          class:active={already && imagesStore.activeId === e.id}
          role="listitem"
          title={e.textPreview}
          onclick={() => reopen(e.id)}
          onkeydown={(e2) => e2.key === 'Enter' && reopen(e.id)}
        >
          <span class="idx"><Icon name="clock" size={11} /></span>
          <div class="thumb">
            {#if e.thumbToken}
              <img
                src={mediaUrl(e.thumbToken)}
                alt=""
                draggable="false"
              />
            {:else}
              <Icon name="image" size={16} />
            {/if}
          </div>
          <div class="info">
            <div class="name">{e.name}</div>
            <div class="meta">
              <Icon name={ORIGIN_ICON[e.origin] ?? 'image'} size={10.5} />
              {e.lineCount} 行 · <b class="ms">{e.totalMs?.toFixed(0) ?? '—'}ms</b> · {timeAgo(e.at)}
            </div>
          </div>
          <button
            class="rm"
            title="删除"
            onclick={(e2) => {
              e2.stopPropagation();
              void deleteHistory(e.id);
            }}
          >
            <Icon name="x" size={11} />
          </button>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .records {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }
  .head {
    padding: 12px 12px 10px;
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    flex-direction: column;
    gap: 9px;
    flex: none;
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .title {
    font-size: 13px;
    font-weight: 650;
    flex: 1;
  }
  .count {
    font-size: 11px;
    font-family: var(--font-mono);
    color: var(--text-faint);
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .bar {
    flex: 1;
    height: 6px;
    border-radius: 3px;
    background: var(--bg-hover);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: 3px;
    transition: width var(--speed-slow) var(--ease-out);
  }
  .done-fill {
    background: linear-gradient(90deg, var(--accent), color-mix(in srgb, var(--accent) 55%, var(--info)));
  }
  .pct {
    font-size: 11px;
    font-family: var(--font-mono);
    color: var(--text-secondary);
    min-width: 32px;
    text-align: right;
  }
  .elapsed {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    font-family: var(--font-mono);
    color: var(--text-faint);
  }
  .elapsed.running {
    color: var(--accent);
  }
  .link {
    font-size: 11.5px;
    color: var(--text-secondary);
    text-decoration: underline;
  }
  .link:hover {
    color: var(--danger);
  }

  .list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 10px 4px 4px;
  }
  .section-title {
    font-size: 10.5px;
    font-weight: 700;
    color: var(--text-faint);
    letter-spacing: 0.08em;
  }
  .section-count {
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--text-faint);
  }
  .wallstats {
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 11px;
    font-family: var(--font-mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 6px 7px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background var(--speed-fast);
    animation: fade-in var(--speed) var(--ease-out);
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.active {
    background: var(--accent-soft);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .row.history .name {
    color: var(--text-secondary);
  }
  .idx {
    min-width: 16px;
    display: inline-flex;
    justify-content: flex-end;
    font-size: 10px;
    font-family: var(--font-mono);
    color: var(--text-faint);
    flex: none;
  }
  .thumb {
    position: relative;
    width: 52px;
    height: 40px;
    border-radius: var(--radius-xs);
    overflow: hidden;
    background: var(--bg-hover);
    display: grid;
    place-items: center;
    color: var(--text-faint);
    flex: none;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .thumb .spin {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.4);
    color: var(--accent);
  }
  .info {
    flex: 1;
    min-width: 0;
  }
  .name {
    font-size: 12px;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    color: var(--text-faint);
    margin-top: 2px;
    white-space: nowrap;
    overflow: hidden;
  }
  .meta .ms {
    color: var(--success);
    font-family: var(--font-mono);
    font-weight: 600;
    margin-left: 6px;
  }
  .status {
    flex: none;
    font-size: 10.5px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--bg-hover);
    color: var(--text-faint);
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status.running {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .status.done {
    color: var(--success);
    background: color-mix(in srgb, var(--success) 12%, transparent);
  }
  .status.error {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .err-text {
    max-width: 90px;
    overflow: hidden;
    text-overflow: ellipsis;
    display: inline-block;
    vertical-align: bottom;
  }
  .rm {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-xs);
    color: var(--text-faint);
    opacity: 0;
    transition: all var(--speed-fast);
    flex: none;
  }
  .row:hover .rm {
    opacity: 1;
  }
  .rm:hover {
    color: var(--danger);
    background: var(--bg-active);
  }
  .rm.rerun:hover {
    color: var(--accent);
  }
  .placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 120px;
    color: var(--text-faint);
    font-size: 12.5px;
    padding: 0 14px;
    text-align: center;
  }
</style>
