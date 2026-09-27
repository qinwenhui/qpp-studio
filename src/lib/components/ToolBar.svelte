<script lang="ts">
  /** 画布上方工具栏:导入 / 截图 / 模型档位 / 视图控制。 */
  import Icon from '$lib/components/Icon.svelte';
  import { api } from '$lib/api';
  import { app, toast } from '$lib/state/app.svelte';
  import { addItems } from '$lib/state/images.svelte';
  import { settings, updateSettings } from '$lib/state/settings.svelte';

  const TIERS = [
    { id: 'tiny', label: '极速', full: '极速 Tiny · 6MB' },
    { id: 'small', label: '均衡', full: '均衡 Small · 31MB · 更准' },
    { id: 'medium', label: '精准', full: '精准 Medium · 138MB · 最准' },
  ];

  async function open() {
    try {
      addItems(await api.pickImages());
    } catch (e) {
      toast('error', String(e));
    }
  }

  async function paste() {
    try {
      addItems([await api.readClipboardImage()]);
    } catch (e) {
      toast('error', String(e));
    }
  }

  async function switchTier(id: string, label: string) {
    if (settings.tier === id) return;
    try {
      await updateSettings({ tier: id });
      toast('success', `已切换到「${label}」档,后台加载中…`);
    } catch {
      /* updateSettings 内部已提示 */
    }
  }

  const hotkeyLabel = $derived(
    settings.hotkey
      .split('+')
      .map((k) => (k.length === 1 ? k.toUpperCase() : k.charAt(0).toUpperCase() + k.slice(1)))
      .join(' + '),
  );

  // 截图按钮右键菜单:选择本次是否隐藏主窗口(微信式)
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

<div class="toolbar">
  <div class="group">
    <button class="btn" onclick={open}>
      <Icon name="folderOpen" size={15} />
      打开
    </button>
    <button class="btn" onclick={paste}>
      <Icon name="clipboard" size={15} />
      粘贴
    </button>
    <button
      class="btn"
      onclick={() => api.screenshotBegin()}
      oncontextmenu={onShotContextMenu}
      title={`${hotkeyLabel} · 右键选择是否隐藏本窗口`}
    >
      <Icon name="crop" size={15} />
      截图
      <span class="kbd">{hotkeyLabel}</span>
    </button>

    <span class="sep"></span>

    <div class="tier-switch" role="group" aria-label="模型档位" title="切换即时生效,后台加载新模型">
      <Icon name="zap" size={13} />
      {#each TIERS as t (t.id)}
        <button
          class="tier-btn"
          class:active={settings.tier === t.id}
          title={t.full}
          onclick={() => switchTier(t.id, t.label)}
        >
          {t.label}
        </button>
      {/each}
    </div>
  </div>

  <div class="group right">
    <button
      class="btn"
      onclick={() => (app.drawer = !app.drawer)}
      title="功能面板(结果/记录/统计/设置)"
    >
      <Icon name="sliders" size={14} />
      面板
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
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--toolbar-h);
    padding: 0 10px;
    border-bottom: 1px solid var(--border-subtle);
    background: color-mix(in srgb, var(--bg-panel) 70%, transparent);
    flex: none;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .group.right {
    gap: 4px;
  }
  .kbd {
    margin-left: 4px;
    padding: 1px 6px;
    border-radius: var(--radius-xs);
    border: 1px solid var(--border-subtle);
    background: var(--bg-hover);
    font-size: 10.5px;
    color: var(--text-faint);
    font-family: var(--font-mono);
  }
  .tier-switch {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: 26px;
    padding: 0 4px 0 8px;
    border-radius: 999px;
    border: 1px solid var(--border-subtle);
    background: var(--bg-hover);
    color: var(--text-faint);
  }
  .tier-btn {
    height: 20px;
    padding: 0 10px;
    border-radius: 999px;
    font-size: 11.5px;
    color: var(--text-secondary);
    transition: all var(--speed-fast) var(--ease-out);
  }
  .tier-btn:hover {
    color: var(--text-primary);
  }
  .tier-btn.active {
    background: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
  }
  .sep {
    width: 1px;
    height: 16px;
    margin: 0 4px;
    background: var(--border-subtle);
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
</style>
