<script lang="ts">
  /** mac 风自定义标题栏:三圆点 + 标题 + 主题切换 + 引擎状态。整栏为拖拽区。 */
  import { getCurrentWindow, type Window } from '@tauri-apps/api/window';
  import Icon from '$lib/components/Icon.svelte';
  import { hasTauri } from '$lib/api';
  import { app, toast } from '$lib/state/app.svelte';
  import { settings, updateSettings } from '$lib/state/settings.svelte';
  import { THEMES } from '$lib/theme';

  const win: Window | null = hasTauri ? getCurrentWindow() : null;
  let maximized = $state(false);

  const unlisten = win
    ? win
        .onResized(async () => {
          maximized = await win!.isMaximized();
        })
        .catch(() => Promise.resolve(null))
    : Promise.resolve(null);

  $effect(() => {
    return () => {
      void unlisten.then((f) => f?.());
    };
  });

  const TIER_LABEL: Record<string, string> = {
    tiny: '极速',
    small: '均衡',
    medium: '精准',
  };

  function cycleTheme() {
    const idx = THEMES.findIndex((t) => t.id === settings.theme);
    const next = THEMES[(idx + 1 + THEMES.length) % THEMES.length];
    void updateSettings({ theme: next.id }).then(() => {
      toast('info', `主题:${next.label}`);
    });
  }
</script>

<header class="titlebar" data-tauri-drag-region ondblclick={() => win?.toggleMaximize()}>
  <div class="traffic" role="group" aria-label="窗口控制">
    <button class="dot red" onclick={() => win?.close()} title="关闭"><span>×</span></button>
    <button class="dot yellow" onclick={() => win?.minimize()} title="最小化"><span>−</span></button>
    <button class="dot green" onclick={() => win?.toggleMaximize()} title={maximized ? '还原' : '最大化'}>
      <span>{maximized ? '⤢' : '+'}</span>
    </button>
  </div>

  <div class="title" data-tauri-drag-region>
    <span class="name">QPP Studio</span>
    <span class="sep">·</span>
    <span class="sub">基于 QPPOCR 推理引擎</span>
  </div>

  <div class="right" data-tauri-drag-region>
    <button
      class="theme-btn"
      onclick={cycleTheme}
      title={`切换主题(当前:${THEMES.find((t) => t.id === settings.theme)?.label ?? '—'})`}
      aria-label="切换主题"
    >
      <Icon name="palette" size={13} />
    </button>
    {#if app.engineError}
      <span class="engine err" title={app.engineError}>引擎异常</span>
    {:else if app.engineReady}
      <span class="engine ok" title={`模型目录 ${app.engineModelsDir}`}>
        <i class="dot"></i>
        {TIER_LABEL[app.engineTier] ?? app.engineTier} · {app.enginePreset}
      </span>
    {:else}
      <span class="engine loading"><i class="dot"></i>引擎加载中…</span>
    {/if}
  </div>
</header>

<style>
  .titlebar {
    display: flex;
    align-items: center;
    gap: 12px;
    height: var(--titlebar-h);
    padding: 0 14px 0 12px;
    background: color-mix(in srgb, var(--bg-panel) 82%, transparent);
    backdrop-filter: blur(var(--backdrop-blur)) saturate(var(--backdrop-saturate));
    border-bottom: 1px solid var(--border-subtle);
    flex: none;
  }

  .traffic {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .dot {
    position: relative;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    transition: filter var(--speed-fast) var(--ease-out);
  }
  .dot span {
    font-size: 9px;
    line-height: 1;
    color: rgb(0 0 0 / 0.55);
    opacity: 0;
    transition: opacity var(--speed-fast);
    font-family: var(--font-ui);
  }
  .traffic:hover .dot span {
    opacity: 1;
  }
  .red {
    background: var(--traffic-red);
  }
  .yellow {
    background: var(--traffic-yellow);
  }
  .green {
    background: var(--traffic-green);
  }
  .dot:hover {
    filter: brightness(1.15);
  }

  .title {
    flex: 1;
    text-align: center;
    font-size: 11px;
    color: var(--text-faint);
    letter-spacing: 0.02em;
    overflow: hidden;
    white-space: nowrap;
  }
  .title .name {
    color: var(--text-secondary);
    font-weight: 600;
  }
  .title .sep {
    margin: 0 6px;
    opacity: 0.5;
  }

  .right {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 120px;
    justify-content: flex-end;
  }
  .engine {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 9px;
    border-radius: 999px;
    font-size: 11px;
    background: var(--bg-hover);
    color: var(--text-secondary);
  }
  .engine .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
  }
  .engine.ok .dot {
    background: var(--success);
    box-shadow: 0 0 6px var(--success);
  }
  .engine.loading .dot {
    background: var(--warning);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .engine.err {
    color: var(--danger);
  }
  .theme-btn {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 999px;
    color: var(--text-faint);
    transition: all var(--speed-fast) var(--ease-out);
  }
  .theme-btn:hover {
    color: var(--accent);
    background: var(--accent-soft);
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
</style>
