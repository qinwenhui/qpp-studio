<script lang="ts">
  /** 性能统计:分段耗时堆叠条 + 计数 + 引擎信息(引擎 Timings 恒有值)。 */
  import { app } from '$lib/state/app.svelte';
  import { getActiveItem } from '$lib/state/images.svelte';
  import { settings } from '$lib/state/settings.svelte';

  const STAGES = [
    { key: 'detPreMs', label: '检测预处理', color: '#38bdf8' },
    { key: 'detInferMs', label: '检测推理', color: '#2dd4a7' },
    { key: 'detPostMs', label: '检测后处理', color: '#a3e635' },
    { key: 'cropMs', label: '区域裁剪', color: '#fbbf24' },
    { key: 'clsMs', label: '方向分类', color: '#f97316' },
    { key: 'recPreMs', label: '识别预处理', color: '#f472b6' },
    { key: 'recInferMs', label: '识别推理', color: '#e879f9' },
    { key: 'recPostMs', label: '识别后处理', color: '#c084fc' },
  ] as const;

  const timings = $derived(getActiveItem()?.outcome?.result?.timings);
  const total = $derived(timings?.totalMs ?? 0);
  const counts = $derived(getActiveItem()?.outcome?.result);

  // 账本口径:九段阶段耗时只覆盖检测/识别两大块,与 total 的差额 =
  // 工作副本准备 / 框合并 / 结果组装 —— 单列为「其他」,让百分比加总=100%
  const otherMs = $derived(
    timings
      ? Math.max(
          0,
          timings.totalMs -
            (timings.detPreMs +
              timings.detInferMs +
              timings.detPostMs +
              timings.cropMs +
              timings.clsMs +
              timings.recPreMs +
              timings.recInferMs +
              timings.recPostMs),
        )
      : 0,
  );

  const TIER_LABEL: Record<string, string> = { tiny: '极速 Tiny', small: '均衡 Small', medium: '精准 Medium' };
  const PRESET_LABEL: Record<string, string> = { speed: '速度', balanced: '均衡', accuracy: '精度' };
</script>

<div class="stats">
  <div class="head">
    {#if getActiveItem()?.item}
      <span class="sub">{getActiveItem()?.item.w}×{getActiveItem()?.item.h}</span>
    {/if}
  </div>

  <div class="body">
    {#if timings}
      <div class="total">
        <span class="num">{total.toFixed(1)}</span>
        <span class="unit">ms 端到端</span>
      </div>

      <div class="stack" role="img" aria-label="耗时分布">
        {#each STAGES as s (s.key)}
          {#if (timings[s.key] ?? 0) > 0}
            <span
              class="seg"
              style="width: {Math.max(0.8, ((timings[s.key] / Math.max(total, 0.001)) * 100)).toFixed(2)}%; background: {s.color}"
              title={`${s.label} ${timings[s.key].toFixed(1)}ms`}
            ></span>
          {/if}
        {/each}
        {#if otherMs > 0}
          <span
            class="seg other"
            style="width: {Math.max(0.8, ((otherMs / Math.max(total, 0.001)) * 100)).toFixed(2)}%"
            title={`其他(副本准备/框合并/结果组装) ${otherMs.toFixed(1)}ms`}
          ></span>
        {/if}
      </div>

      <ul class="legend">
        {#each STAGES as s (s.key)}
          <li class:zero={(timings[s.key] ?? 0) === 0}>
            <i style="background:{s.color}"></i>
            <span class="label">{s.label}</span>
            <span class="ms">{(timings[s.key] ?? 0).toFixed(1)}</span>
            <span class="pct">{total > 0 ? ((timings[s.key] / total) * 100).toFixed(0) : 0}%</span>
          </li>
        {/each}
        <li class:zero={otherMs === 0}>
          <i class="other-dot" class:warn={otherMs > 30}></i>
          <span class="label" title="工作副本准备 / 框合并 / 结果组装(九段之外的差额)">其他(组装)</span>
          <span class="ms">{otherMs.toFixed(1)}</span>
          <span class="pct">{total > 0 ? ((otherMs / total) * 100).toFixed(0) : 0}%</span>
        </li>
      </ul>
      {#if otherMs > 30}
        <p class="note warn-note">其他耗时 {otherMs.toFixed(1)}ms 偏高——常为超大图的工作副本准备,可尝试在设置里调小「检测输入长边」以外的原图尺寸因素,或直接换更小档位</p>
      {/if}

      {#if counts}
        <div class="counts">
          <div class="cell"><b>{counts.numBoxes}</b><span>检出框</span></div>
          <div class="cell"><b>{counts.numMerged}</b><span>同行合并</span></div>
          <div class="cell"><b>{counts.numFlipped}</b><span>翻正</span></div>
          <div class="cell"><b>{counts.numUnread}</b><span>未读出</span></div>
        </div>
      {/if}
    {:else}
      <div class="placeholder">识别一张图片后展示耗时分解</div>
    {/if}

    <div class="engine">
      <div class="row"><span class="k">模型档位</span><span class="v">{TIER_LABEL[app.engineTier] ?? app.engineTier}</span></div>
      <div class="row"><span class="k">预设</span><span class="v">{PRESET_LABEL[app.enginePreset] ?? app.enginePreset}</span></div>
      <div class="row"><span class="k">线程</span><span class="v">{app.engineThreads === 0 ? '自动' : app.engineThreads}</span></div>
      <div class="row" title="实际计算设备(引擎构建结果)"><span class="k">设备</span><span class="v">{app.engineDevice === 'gpu' ? 'GPU (Vulkan)' : 'CPU'}</span></div>
      <div class="row" title="纯 Rust · 自研引擎"><span class="k">后端</span><span class="v">高性能 QPPOCR 引擎</span></div>
      <div class="row path" title={app.engineModelsDir}>
        <span class="k">模型目录</span><span class="v">{app.engineModelsDir}</span>
      </div>
      {#if settings.threads !== app.engineThreads}
        <p class="note">线程数改动将在重启后生效(进程级线程池启动时锁定)</p>
      {/if}
    </div>
  </div>
</div>

<style>
  .stats {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow-y: auto;
  }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    padding: 12px 14px 8px;
    border-bottom: 1px solid var(--border-subtle);
    flex: none;
  }
  .title {
    font-size: 13px;
    font-weight: 650;
  }
  .sub {
    font-size: 11px;
    font-family: var(--font-mono);
    color: var(--text-faint);
  }
  .body {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .total {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  .total .num {
    font-size: 30px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--accent);
    letter-spacing: -0.02em;
  }
  .total .unit {
    font-size: 12px;
    color: var(--text-faint);
  }
  .stack {
    display: flex;
    height: 10px;
    border-radius: 5px;
    overflow: hidden;
    gap: 1px;
    background: var(--bg-hover);
  }
  .seg {
    transition: width var(--speed-slow) var(--ease-out);
  }
  .seg.other {
    background: color-mix(in srgb, var(--text-faint) 55%, transparent);
  }
  .other-dot {
    background: color-mix(in srgb, var(--text-faint) 55%, transparent);
  }
  .other-dot.warn {
    background: var(--warning);
  }
  .warn-note {
    font-size: 10.5px;
    color: var(--warning);
  }
  .legend {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .legend li {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
    color: var(--text-secondary);
  }
  .legend li.zero {
    opacity: 0.38;
  }
  .legend i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    flex: none;
  }
  .legend .label {
    flex: 1;
  }
  .legend .ms {
    font-family: var(--font-mono);
    color: var(--text-primary);
    min-width: 44px;
    text-align: right;
  }
  .legend .pct {
    font-family: var(--font-mono);
    color: var(--text-faint);
    min-width: 34px;
    text-align: right;
  }
  .counts {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
  }
  .cell {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 10px 4px;
    border-radius: var(--radius-md);
    background: var(--bg-hover);
  }
  .cell b {
    font-size: 16px;
    font-family: var(--font-mono);
    color: var(--text-primary);
  }
  .cell span {
    font-size: 10.5px;
    color: var(--text-faint);
  }
  .engine {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 12px;
    border-radius: var(--radius-md);
    background: var(--bg-hover);
    font-size: 11.5px;
  }
  .engine .row {
    display: flex;
    justify-content: space-between;
    gap: 10px;
  }
  .engine .k {
    color: var(--text-faint);
    flex: none;
  }
  .engine .v {
    color: var(--text-primary);
    text-align: right;
    word-break: break-all;
  }
  .engine .row.path .v {
    font-family: var(--font-mono);
    font-size: 10.5px;
  }
  .note {
    font-size: 10.5px;
    color: var(--warning);
  }
  .placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100px;
    color: var(--text-faint);
    font-size: 12.5px;
  }
</style>
