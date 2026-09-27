<script lang="ts">
  /** 右侧功能抽屉:悬浮式面板(参考 mac 设置面板风格)。
   *  内容 = 识别记录 / 性能统计 / 设置。识别结果已挪到右内容面板(纸张/列表)。
   *  点击抽屉外(画布/纸张/标题栏等)自动收起;左栏/工具条豁免(它们有自己的开关语义)。 */
  import Icon from '$lib/components/Icon.svelte';
  import { app } from '$lib/state/app.svelte';
  import RecordsPanel from './Panels/RecordsPanel.svelte';
  import StatsPanel from './Panels/StatsPanel.svelte';
  import SettingsPanel from './Panels/SettingsPanel.svelte';

  const TITLES: Record<string, string> = {
    records: '识别记录',
    stats: '性能统计',
    settings: '设置',
  };

  function onGlobalPointerDown(e: PointerEvent) {
    if (!app.drawer) return;
    const el = e.target as HTMLElement | null;
    if (el?.closest?.('.drawer, .rail, .toolbar')) return;
    app.drawer = false;
  }
</script>

<svelte:window onpointerdown={onGlobalPointerDown} />

<aside class="drawer" class:open={app.drawer} aria-label={TITLES[app.view]}>
  <header class="head">
    <span class="title">{TITLES[app.view]}</span>
    <button class="icon-btn close" onclick={() => (app.drawer = false)} title="收起 (面板按钮也可)">
      <Icon name="chevronRight" size={15} />
    </button>
  </header>
  <div class="content">
    {#if app.view === 'records'}
      <RecordsPanel />
    {:else if app.view === 'stats'}
      <StatsPanel />
    {:else}
      <SettingsPanel />
    {/if}
  </div>
</aside>

<style>
  .drawer {
    position: absolute;
    top: 10px;
    right: 10px;
    bottom: 10px;
    z-index: 100;
    width: 340px;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--bg-panel) 97%, transparent);
    backdrop-filter: blur(var(--backdrop-blur)) saturate(var(--backdrop-saturate));
    border: 1px solid var(--border-subtle);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    /* 收起时滑出屏幕右缘(多留 margin 不露出影子) */
    transform: translateX(calc(100% + 26px));
    transition: transform var(--speed-slow) var(--ease-out);
    pointer-events: none;
  }
  .drawer.open {
    transform: translateX(0);
    pointer-events: auto;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 8px 10px 16px;
    border-bottom: 1px solid var(--border-subtle);
    flex: none;
  }
  .title {
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
  }
  .close {
    width: 26px;
    height: 26px;
  }
  .content {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
