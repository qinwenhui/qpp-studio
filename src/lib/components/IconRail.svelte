<script lang="ts">
  /** 左侧窄图标栏:logo + 主操作 + 面板切换 + 底部引擎状态。 */
  import Icon from '$lib/components/Icon.svelte';
  import LogoMark from '$lib/components/LogoMark.svelte';
  import type { IconName } from '$lib/icons/paths';
  import { api } from '$lib/api';
  import { app, toggleView, toast, type View } from '$lib/state/app.svelte';
  import { addItems, imagesStore } from '$lib/state/images.svelte';
  import { settings, updateSettings } from '$lib/state/settings.svelte';

  const TOOLS: { view: View; icon: IconName; label: string }[] = [
    { view: 'records', icon: 'layers', label: '识别记录' },
    { view: 'stats', icon: 'cpu', label: '性能统计' },
    { view: 'settings', icon: 'gear', label: '设置' },
  ];

  const pendingCount = $derived(
    imagesStore.items.filter((i) => i.phase === 'queued' || i.phase === 'running').length,
  );

  async function openImages() {
    try {
      addItems(await api.pickImages());
    } catch (e) {
      toast('error', String(e));
    }
  }

  // 截图按钮右键菜单:选择本次是否隐藏主窗口(与工具栏截图按钮一致)
  let shotMenu = $state<{ x: number; y: number } | null>(null);

  function onShotContextMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    shotMenu = { x: e.clientX, y: e.clientY };
  }

  async function shotWith(hide: boolean) {
    shotMenu = null;
    await updateSettings({ shotHide: hide });
    api.screenshotBegin();
  }
</script>

<svelte:window onclick={() => (shotMenu = null)} />

<nav class="rail">
  <div class="logo" title="QPP Studio">
    <LogoMark size={26} />
  </div>

  <div class="actions">
    <button class="rail-btn" onclick={openImages} title="打开图片">
      <Icon name="image" size={19} />
    </button>
    <button
      class="rail-btn"
      onclick={() => api.screenshotBegin()}
      oncontextmenu={onShotContextMenu}
      title="截图识别 · 右键选择是否隐藏本窗口"
    >
      <Icon name="crop" size={19} />
    </button>
  </div>

  {#if shotMenu}
    <div class="shot-menu" style="left:{shotMenu.x}px; top:{shotMenu.y}px">
      <button onclick={() => shotWith(true)}>
        <Icon name="eyeOff" size={13} />
        截图(隐藏本窗口)
        {#if settings.shotHide}<span class="cur">当前默认</span>{/if}
      </button>
      <button onclick={() => shotWith(false)}>
        <Icon name="eye" size={13} />
        截图(不隐藏)
        {#if !settings.shotHide}<span class="cur">当前默认</span>{/if}
      </button>
    </div>
  {/if}

  <div class="divider"></div>

  <div class="views">
    {#each TOOLS as t (t.view)}
      <button
        class="rail-btn"
        class:active={app.drawer && app.view === t.view}
        onclick={() => toggleView(t.view)}
        title={t.label}
        aria-label={t.label}
      >
        <Icon name={t.icon} size={19} />
        {#if t.view === 'records' && pendingCount > 0}
          <span class="badge">{pendingCount}</span>
        {/if}
      </button>
    {/each}
  </div>

  <div class="spacer"></div>

  <div class="foot">
    <span
      class="status"
      class:ok={app.engineReady}
      class:err={!!app.engineError}
      title={app.engineError || '引擎就绪'}
    ></span>
  </div>
</nav>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    width: var(--rail-w);
    padding: 8px 0 10px;
    gap: 6px;
    background: color-mix(in srgb, var(--bg-panel) 88%, transparent);
    backdrop-filter: blur(var(--backdrop-blur)) saturate(var(--backdrop-saturate));
    border-right: 1px solid var(--border-subtle);
    flex: none;
    /* backdrop-filter 创建 stacking context:不提级的话,栏内右键菜单
       会被后画的内容面板(画布/纸张)盖住点不了 */
    z-index: 60;
  }

  .logo {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    margin-bottom: 2px;
    color: var(--text-primary);
  }

  .rail-btn {
    position: relative;
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: var(--radius-md);
    color: var(--text-faint);
    transition: all var(--speed-fast) var(--ease-out);
  }
  .rail-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }
  .rail-btn.active {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .rail-btn.active::before {
    content: '';
    position: absolute;
    left: -8px;
    width: 3px;
    height: 18px;
    border-radius: 2px;
    background: var(--accent);
  }
  .badge {
    position: absolute;
    top: 4px;
    right: 4px;
    min-width: 15px;
    height: 15px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--accent);
    color: var(--accent-contrast);
    font-size: 9.5px;
    font-weight: 700;
    line-height: 15px;
    text-align: center;
  }

  .shot-menu {
    position: fixed;
    z-index: 300;
    min-width: 190px;
    padding: 4px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--bg-elevated) 97%, transparent);
    backdrop-filter: blur(12px);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-lg);
    display: flex;
    flex-direction: column;
    gap: 1px;
    animation: rise-in var(--speed-fast) var(--ease-out);
  }
  .shot-menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--radius-sm);
    font-size: 12.5px;
    color: var(--text-primary);
    text-align: left;
  }
  .shot-menu button:hover {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .shot-menu .cur {
    margin-left: auto;
    color: var(--accent);
    font-size: 10px;
  }

  .divider {
    width: 22px;
    height: 1px;
    margin: 4px 0;
    background: var(--border-subtle);
  }

  .spacer {
    flex: 1;
  }

  .foot .status {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--warning);
    animation: pulse 1.4s ease-in-out infinite;
  }
  .foot .status.ok {
    background: var(--success);
    box-shadow: 0 0 8px color-mix(in srgb, var(--success) 60%, transparent);
    animation: none;
  }
  .foot .status.err {
    background: var(--danger);
    animation: none;
  }
  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }
</style>
