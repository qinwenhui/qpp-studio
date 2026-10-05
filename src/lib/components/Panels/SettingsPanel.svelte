<script lang="ts">
  /** 设置面板:外观 / 引擎 / 截图热键 / 批量 / 关于。 */
  import AccordionSection from './AccordionSection.svelte';
  import Icon from '$lib/components/Icon.svelte';
  import mascotUrl from '$assets/mascot-512.png';
  import { api } from '$lib/api';
  import { app, toast } from '$lib/state/app.svelte';
  import { settings, updateSettings } from '$lib/state/settings.svelte';
  import { THEMES } from '$lib/theme';

  const TIERS = [
    { id: 'tiny', label: '极速 Tiny', desc: '6.3MB · 最快' },
    { id: 'small', label: '均衡 Small', desc: '31MB · 更准' },
    { id: 'medium', label: '精准 Medium', desc: '138MB · 最准' },
  ];
  const PRESETS = [
    { id: 'speed', label: '速度' },
    { id: 'balanced', label: '均衡' },
    { id: 'accuracy', label: '精度' },
    { id: 'special', label: '特殊' },
  ];
  const PRESET_TIPS: Record<string, string> = {
    speed: '识别画布 40px(矮 17%,更快)。区域重试关',
    balanced: '识别画布 48px。区域重试关(引擎新默认:语料上开关重试精度一致,重试却让 9% 图片检测耗时翻倍)',
    accuracy: '识别画布 48px + 区域重试开(低置信区域自动重跑检测,难图可救)+ 杂波判定 0.15',
    special: '监控截图:杂波判定 0.45 + 垂直扩张收缩 0.5 + 关方向分类。白字压栏杆/栅栏专用,⚠ 代价大(语料 93%→82%)',
  };

  let recording = $state(false);
  let pendingHotkey = $state('');

  // 设备实测对比(约数秒:两侧各建引擎+热身+5 轮)
  let benching = $state(false);
  let bench = $state<Awaited<ReturnType<typeof api.deviceBenchmark>> | null>(null);

  async function runDeviceBench() {
    if (benching) return;
    benching = true;
    bench = null;
    try {
      bench = await api.deviceBenchmark();
    } catch (e) {
      toast('error', String(e));
    } finally {
      benching = false;
    }
  }

  // 硬件检测信息(并行策略区展示;挂载时拉一次,设置变化时刷新策略)
  let hw = $state<Awaited<ReturnType<typeof api.hwInfo>> | null>(null);
  $effect(() => {
    // 依赖 tier/device/workersOverride:策略按它们实时计算
    void settings.tier;
    void settings.device;
    void settings.workersOverride;
    api
      .hwInfo()
      .then((h) => (hw = h))
      .catch(() => (hw = null));
  });

  function startRecord() {
    recording = true;
    pendingHotkey = '';
  }

  /** 存储统一用 ctrl 表示主修饰键;macOS 录制时 Cmd(metaKey)落成 ctrl,
   *  注册端(hotkey.rs)按平台把 ctrl 映射为 Cmd 加速键 */
  function comboFromEvent(e: KeyboardEvent): string | null {
    const isMac = app.platform === 'macos';
    const parts: string[] = [];
    if (isMac ? e.metaKey : e.ctrlKey) parts.push('ctrl');
    if (e.altKey) parts.push('alt');
    if (e.shiftKey) parts.push('shift');
    if (!isMac && e.metaKey) parts.push('meta');
    let key = e.key.toLowerCase();
    if (['control', 'alt', 'shift', 'meta'].includes(key)) return null; // 单独按修饰键
    if (key === ' ') key = 'space';
    if (!/^[a-z0-9f]\d*$/.test(key) && !/^f\d{1,2}$/.test(key) && key.length !== 1) {
      return null; // 只收字母/数字/F1-F12
    }
    parts.push(key);
    if (parts.length < 2) return null; // 必须带修饰键,避免吞掉普通按键
    return parts.join('+');
  }

  /** 显示用:mac 上主修饰键叫 Cmd */
  function fmtHotkey(s: string): string {
    return s
      .split('+')
      .map((t) => (app.platform === 'macos' && t === 'ctrl' ? 'cmd' : t))
      .join('+');
  }

  function onKeydown(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape') {
      recording = false;
      return;
    }
    const combo = comboFromEvent(e);
    if (combo) {
      pendingHotkey = combo;
      recording = false;
      void updateSettings({ hotkey: combo });
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="settings">

  <div class="body">
    <AccordionSection title="外观" subtitle={THEMES.find((t) => t.id === settings.theme)?.label}>
      <div class="themes">
        {#each THEMES as t (t.id)}
          <button
            class="theme"
            class:active={settings.theme === t.id}
            onclick={() => updateSettings({ theme: t.id })}
          >
            <span class="swatch" style="background: {t.swatch}"></span>
            <span class="tlabel">{t.label}</span>
          </button>
        {/each}
      </div>
    </AccordionSection>

    <AccordionSection title="引擎" subtitle="切换即时生效">
      <div class="field">
        <span class="k">模型档位</span>
        <div class="tier-cards">
          {#each TIERS as t (t.id)}
            <button
              class="tier"
              class:active={settings.tier === t.id}
              onclick={() => updateSettings({ tier: t.id })}
            >
              <span class="tname">{t.label}</span>
              <span class="tdesc">{t.desc}</span>
            </button>
          {/each}
        </div>
      </div>

      <div class="field">
        <span class="k">识别预设</span>
        <div class="chips">
          {#each PRESETS as p (p.id)}
            <button
              class="chip"
              class:active={settings.preset === p.id}
              onclick={() => updateSettings({ preset: p.id })}
              title={PRESET_TIPS[p.id] ?? ''}
            >
              {p.label}
            </button>
          {/each}
        </div>
        <p class="note">{PRESET_TIPS[settings.preset] ?? ''}</p>
      </div>

      <div class="field">
        <span class="k">计算设备</span>
        <div class="chips">
          <button
            class="chip"
            class:active={settings.device !== 'gpu'}
            onclick={() => updateSettings({ device: 'cpu' })}
          >CPU</button>
          <button
            class="chip"
            class:active={settings.device === 'gpu'}
            disabled={!hw || hw.gpus.length === 0}
            onclick={() => updateSettings({ device: 'gpu' })}
            title={
              !hw || hw.gpus.length === 0
                ? app.platform === 'macos'
                  ? '当前平台不支持 GPU 加速(需 Vulkan,macOS 无 Vulkan loader);CPU 走 ARM NEON 已高度优化'
                  : '未检测到可用的 Vulkan 1.4+ 设备(装/升级显卡驱动后重启应用)'
                : hw.gpus.map((g) => `${g.name} (${g.api})`).join(', ')
            }
          >GPU{hw && hw.gpus.length > 0 ? '' : '(未检测到)'}</button>
        </div>
        <p class="note">
          {#if app.platform === 'macos' && (!hw || hw.gpus.length === 0)}
            GPU 加速需 Vulkan(Windows/Linux);macOS 走 CPU NEON,单张同样毫秒级
          {:else if settings.device === 'gpu'}
            GPU 走 Vulkan 1.4+:tiny 档输出与 CPU 完全一致;small 档存在极个别行的浮点末位差异——需要与 CPU 完全一致请切回 CPU。冷启动首图略慢
          {:else}
            GPU 不一定比 CPU 快(冷启动慢、老核显可能反超);切换后引擎重建,进行中的识别用旧引擎跑完
          {/if}
        </p>
        <div class="bench-row">
          <button
            class="badge-btn"
            onclick={() => runDeviceBench()}
            disabled={benching}
            title="单张延迟实测(合成图,两侧各热身1+5轮取中位);批量吞吐受并行 worker 数影响,大批量场景以实际跑批为准">
            {benching ? '实测中…' : '单张实测对比'}
          </button>
          {#if bench}
            {#if bench.gpuError}
              <span class="bench-err" title={bench.gpuError}>
                GPU 不可用:{bench.gpuError.length > 48 ? `${bench.gpuError.slice(0, 48)}…` : bench.gpuError}
              </span>
            {:else if bench.gpuMs < bench.cpuMs}
              <span class="bench-ok" title="单张热身延迟;批量吞吐还受并行 worker 数影响(GPU 上限低于 CPU,属内存/显存约束)">
                单张:CPU {bench.cpuMs.toFixed(0)}ms · GPU {bench.gpuMs.toFixed(0)}ms → GPU 快 {(bench.cpuMs / bench.gpuMs).toFixed(1)}×
              </span>
            {:else}
              <span class="bench-ok" title="单张热身延迟;批量吞吐还受并行 worker 数影响">
                单张:CPU {bench.cpuMs.toFixed(0)}ms · GPU {bench.gpuMs.toFixed(0)}ms → CPU 更快,建议保持 CPU
              </span>
            {/if}
          {/if}
        </div>
      </div>

      <div class="field">
        <span class="k">
          线程数
          <em class="v">{settings.threads === 0 ? '自动' : settings.threads}</em>
        </span>
        <input
          class="range"
          type="range"
          min="0"
          max="16"
          step="1"
          value={settings.threads}
          oninput={(e) => (settings.threads = +e.currentTarget.value)}
          onchange={() => updateSettings({ threads: settings.threads })}
        />
        <p class="note">首次启动时锁定线程池;改动重启后生效</p>
      </div>

      <div class="field">
        <span class="k">模型目录</span>
        <input
          class="input dir"
          placeholder="默认:随安装包 / 自动探测"
          value={settings.modelsDir ?? ''}
          onchange={(e) => updateSettings({ modelsDir: e.currentTarget.value || undefined })}
        />
      </div>

      <div class="field">
        <label class="switch-row">
          <span class="switch-text">
            <span class="k2">方向纠正</span>
            <span class="desc">自动翻正 180° 倒置的图片(如反拍文档)。开启有轻微性能损耗:每个文本行多一次方向分类,约 +2~25ms/行;关闭后倒置图会识别成乱码</span>
          </span>
          <input
            type="checkbox"
            class="switch"
            checked={settings.orientation}
            onchange={(e) => updateSettings({ orientation: e.currentTarget.checked })}
          />
        </label>
      </div>

      <div class="field">
        <label class="switch-row">
          <span class="switch-text">
            <span class="k2">增强对比</span>
            <span class="desc">提升低对比图片(浅色印章、褪色扫描件)的识别率。正常图片开启可能无益甚至略降准确度,并略增耗时;建议仅在识别效果差时尝试</span>
          </span>
          <input
            type="checkbox"
            class="switch"
            checked={settings.enhanceContrast}
            onchange={(e) => updateSettings({ enhanceContrast: e.currentTarget.checked })}
          />
        </label>
      </div>

      <div class="field">
        <span class="k">
          检测放大
          <em class="v">{settings.upscale}×</em>
        </span>
        <div class="chips">
          {#each [1, 2, 3] as n (n)}
            <button
              class="chip"
              class:active={settings.upscale === n}
              onclick={() => updateSettings({ upscale: n })}
            >
              {n}×
            </button>
          {/each}
        </div>
        <p class="note">把检测器输入放大 N 倍再检测(识别裁剪仍取原图,框坐标不变),小字/水印图开 2× + 增强对比效果显著。代价:检测耗时约 ×N²,批量场景慎用</p>
      </div>
    </AccordionSection>

    <AccordionSection title="截图快捷键" open={false}>
      <div class="field">
        <button
          class="hotkey-box"
          class:recording
          onclick={startRecord}
          title="点击后按下新组合键"
        >
          {#if recording}
            按下新组合键…(Esc 取消)
          {:else}
            <kbd>{fmtHotkey(pendingHotkey || settings.hotkey)}</kbd>
            <span class="rec-hint">点击修改</span>
          {/if}
        </button>
        <p class="note">需包含至少一个修饰键({app.platform === 'macos' ? 'Cmd' : 'Ctrl'}/Alt/Shift);保存后全局生效</p>
      </div>
    </AccordionSection>

    <AccordionSection title="并行与批量" open={false}>
      {#if hw}
        <div class="field">
          <span class="k">本机硬件</span>
          <p class="note hw-info">
            {hw.cpuBrand}<br />
            {hw.physicalCores} 物理核 / {hw.logicalCores} 线程 · {hw.totalMemGb > 0 ? `${hw.totalMemGb} GB 内存` : '内存检测失败'}<br />
            GPU:{hw.gpus.length ? hw.gpus.map((g) => `${g.name} (${g.api})`).join(', ') : '未检测到 Vulkan 设备'}
          </p>
          <p class="note">
            当前策略:{hw.plan.workers > 0
              ? `${settings.workersOverride > 0 ? '手动' : '自动'} ${hw.plan.workers} 进程 × ${hw.plan.threadsEach} 线程 · 小批量并发 ${hw.plan.inprocConcurrency}${hw.plan.clampedByMem ? ' · 已被内存上限压低' : ''}`
              : 'medium 档模型较重,不分治(进程内并发 ' + hw.plan.inprocConcurrency + ')'}
          </p>
        </div>
      {/if}
      <div class="field">
        <span class="k">
          worker 进程数
          <em class="v">{settings.workersOverride === 0 ? '自动' : settings.workersOverride}</em>
        </span>
        <input
          class="range"
          type="range"
          min="0"
          max="8"
          step="1"
          value={settings.workersOverride}
          oninput={(e) => (settings.workersOverride = +e.currentTarget.value)}
          onchange={() => updateSettings({ workersOverride: settings.workersOverride })}
        />
        <p class="note">自动 = 启动时检测 CPU/内存按优化表取最优;手动值仍受内存安全上限约束,对下一批生效。8 张以上图片批量与 ≥8 页 PDF 按此数分进程并行</p>
      </div>
      <div class="field">
        <span class="k">
          小批量并发数
          <em class="v">{settings.batchConcurrency === 0 ? '自动' : settings.batchConcurrency}</em>
        </span>
        <input
          class="range"
          type="range"
          min="0"
          max="4"
          step="1"
          value={settings.batchConcurrency}
          oninput={(e) => (settings.batchConcurrency = +e.currentTarget.value)}
          onchange={() => updateSettings({ batchConcurrency: settings.batchConcurrency })}
        />
        <p class="note">8 张以下批量的进程内并发(单张识别独占引擎,不受影响)。8 张以上自动改走上面的多进程分治</p>
      </div>
    </AccordionSection>

    <AccordionSection title="关于" open={false}>
      <div class="mascot-wrap">
        <img class="mascot" src={mascotUrl} alt="QPP Studio 看板娘" draggable="false" />
        <span class="mascot-cap">QPP Studio · 本地极速 OCR</span>
      </div>
      <div class="about">
        <div class="row"><span>QPP Studio</span><span class="mono">v{app.version || '0.1.0'}</span></div>
        <div class="row"><span>推理引擎</span><span class="mono">qppocr 引擎</span></div>
        <div class="row"><span>平台</span><span class="mono">{app.platform}</span></div>
        <p class="note">本地推理,数据不出机器</p>
      </div>
      <div class="author-card">
        <span class="by">Crafted by</span>
        <span class="author">qinwh</span>
        <div class="links">
          <button
            class="link-chip"
            onclick={() => api.openUrl('https://github.com/qinwenhui/qpp-studio')}
            title="GitHub 仓库"
          >
            <Icon name="github" size={15} filled />
            GitHub
          </button>
          <button
            class="link-chip"
            onclick={() => api.openUrl('https://qinwh.cn')}
            title="作者主页"
          >
            <Icon name="external" size={13} />
            qinwh.cn
          </button>
        </div>
      </div>
    </AccordionSection>
  </div>
</div>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }
  .head {
    padding: 12px 14px 8px;
    border-bottom: 1px solid var(--border-subtle);
    flex: none;
  }
  .title {
    font-size: 13px;
    font-weight: 650;
  }
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 10px;
  }

  .themes {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 8px;
  }
  .theme {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 8px 10px;
    border-radius: var(--radius-md);
    border: 1px solid var(--border-subtle);
    background: var(--bg-hover);
    transition: all var(--speed-fast) var(--ease-out);
    text-align: left;
  }
  .theme:hover {
    border-color: var(--border-strong);
  }
  .theme.active {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .swatch {
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    border: 1px solid rgb(255 255 255 / 0.18);
    box-shadow: var(--shadow-sm);
    flex: none;
  }
  .tlabel {
    font-size: 12px;
    color: var(--text-primary);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .field .k {
    font-size: 12px;
    color: var(--text-secondary);
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }
  .field .k .v {
    font-style: normal;
    font-family: var(--font-mono);
    color: var(--accent);
    font-size: 12px;
  }
  .tier-cards {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
  }
  .tier {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 9px 6px;
    border-radius: var(--radius-md);
    border: 1px solid var(--border-subtle);
    background: var(--bg-hover);
    transition: all var(--speed-fast) var(--ease-out);
  }
  .tier:hover {
    border-color: var(--border-strong);
  }
  .tier.active {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .tname {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-primary);
  }
  .tdesc {
    font-size: 10px;
    color: var(--text-faint);
  }
  .chips {
    display: flex;
    gap: 6px;
  }
  .range {
    width: 100%;
    accent-color: var(--accent);
  }
  .dir {
    width: 100%;
    font-family: var(--font-mono);
    font-size: 11px;
  }
  .note {
    font-size: 10.5px;
    color: var(--text-faint);
    line-height: 1.5;
  }
  .badge-btn {
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 10px;
    border-radius: 999px;
    flex: none;
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    transition: all var(--speed-fast) var(--ease-out);
    white-space: nowrap;
  }
  .badge-btn:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .badge-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .bench-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 4px;
    flex-wrap: wrap;
  }
  .bench-ok {
    font-size: 11px;
    font-family: var(--font-mono);
    color: var(--success);
  }
  .bench-err {
    font-size: 11px;
    color: var(--danger);
  }
  .chips .chip:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .note.hw-info {
    font-family: var(--font-mono);
    color: var(--text-secondary);
  }

  .switch-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    cursor: pointer;
  }
  .switch-text {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .k2 {
    font-size: 12px;
    color: var(--text-primary);
    font-weight: 600;
  }
  .desc {
    font-size: 10.5px;
    color: var(--text-faint);
    line-height: 1.5;
  }
  .switch {
    appearance: none;
    flex: none;
    width: 34px;
    height: 20px;
    margin-top: 2px;
    border-radius: 999px;
    background: var(--bg-active);
    position: relative;
    cursor: pointer;
    transition: background var(--speed-fast) var(--ease-out);
  }
  .switch::after {
    content: '';
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--text-secondary);
    transition: all var(--speed-fast) var(--ease-spring);
  }
  .switch:checked {
    background: var(--accent);
  }
  .switch:checked::after {
    left: 16px;
    background: var(--accent-contrast);
  }

  .hotkey-box {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    height: 38px;
    padding: 0 12px;
    border-radius: var(--radius-md);
    border: 1px dashed var(--border-strong);
    background: var(--bg-input);
    color: var(--text-secondary);
    font-size: 12px;
    transition: all var(--speed-fast);
  }
  .hotkey-box.recording {
    border-color: var(--accent);
    color: var(--accent);
    border-style: solid;
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .hotkey-box kbd {
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--text-primary);
  }
  .rec-hint {
    font-size: 10.5px;
    color: var(--text-faint);
  }

  .mascot-wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 7px;
    padding: 6px 0 2px;
  }
  .mascot {
    width: 216px;
    height: 216px;
    border-radius: var(--radius-lg);
    border: 1px solid var(--border-subtle);
    box-shadow: var(--shadow-lg);
    object-fit: cover;
  }
  .mascot-cap {
    font-size: 10.5px;
    color: var(--text-faint);
    letter-spacing: 0.05em;
  }
  .about {
    display: flex;
    flex-direction: column;
    gap: 7px;
    font-size: 12px;
  }
  .about .row {
    display: flex;
    justify-content: space-between;
    color: var(--text-secondary);
  }
  .mono {
    font-family: var(--font-mono);
    color: var(--text-primary);
    font-size: 11.5px;
  }

  .author-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 14px 12px 10px;
    border-radius: var(--radius-md);
    background: var(--bg-hover);
  }
  .by {
    font-size: 10px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .author {
    font-size: 16px;
    font-weight: 700;
    background: linear-gradient(90deg, var(--accent), color-mix(in srgb, var(--accent) 45%, var(--info)));
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  .links {
    display: flex;
    gap: 8px;
  }
  .link-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 12px;
    border-radius: 999px;
    font-size: 12px;
    color: var(--text-secondary);
    border: 1px solid var(--border-subtle);
    background: var(--bg-panel);
    transition: all var(--speed-fast) var(--ease-out);
  }
  .link-chip:hover {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    background: var(--accent-soft);
  }
</style>
