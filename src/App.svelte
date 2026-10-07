<script lang="ts">
  /** 主窗口:布局组装 + 后端事件接线 + 全局键鼠。 */
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import type { UnlistenFn } from '@tauri-apps/api/event';

  import TitleBar from '$lib/components/TitleBar.svelte';
  import Icon from '$lib/components/Icon.svelte';
  import RightPane from '$lib/components/RightPane.svelte';
  import IconRail from '$lib/components/IconRail.svelte';
  import ToolBar from '$lib/components/ToolBar.svelte';
  import CanvasStage from '$lib/components/Canvas/CanvasStage.svelte';
  import PdfPageNav from '$lib/components/PdfPageNav.svelte';
  import Drawer from '$lib/components/Drawer.svelte';
  import ThumbStrip from '$lib/components/ThumbStrip.svelte';
  import DropOverlay from '$lib/components/DropOverlay.svelte';
  import Toast from '$lib/components/Toast.svelte';
  import { getActiveItem } from '$lib/state/images.svelte';

  import { api } from '$lib/api';
  import { app, setView, toast } from '$lib/state/app.svelte';
  import { addItems, applyOutcome, applyStatus, addItemWithOutcome, continuePending, applyPdfPageDone, applyPdfOcrDone } from '$lib/state/images.svelte';
  import { applySettings, loadSettings, syncEngine, settings } from '$lib/state/settings.svelte';
  import { refreshHistory, scheduleHistoryRefresh } from '$lib/state/history.svelte';
  import type { Settings, EngineStatus, ImageItem, ItemOutcome, ToastMsg } from '$lib/types';

  /** 耗时格式化:<60s 显示「,耗时 12.3 秒」,否则「,耗时 2 分 5 秒」 */
  function fmtElapsed(ms?: number): string {
    if (ms == null) return '';
    const s = ms / 1000;
    if (s < 60) return `,耗时 ${s.toFixed(1)} 秒`;
    return `,耗时 ${Math.floor(s / 60)} 分 ${Math.round(s % 60)} 秒`;
  }

  onMount(() => {
    let cleanup: (() => void) | undefined;
    void (async () => {
      await loadSettings();
      app.booted = true;

      const unlisteners: Promise<UnlistenFn>[] = [
        // mac 原生菜单栏动作(open/paste/shot/settings),与应用内快捷键同款处理
        listen<string>('app://menu', (e) => {
          if (e.payload === 'open') {
            void openFromPicker();
          } else if (e.payload === 'paste') {
            void pasteFromClipboard();
          } else if (e.payload === 'shot') {
            void api.screenshotBegin().catch((err) => toast('error', String(err)));
          } else if (e.payload === 'settings') {
            setView('settings');
          }
        }),
        listen<EngineStatus>('engine://status', (e) => syncEngine(e.payload)),
        listen<{
          id: string;
          outcome: ItemOutcome['outcome'];
          thumbToken?: string;
          pdfPage?: number;
        }>('ocr://item-done', (e) => {
          applyOutcome(e.payload.id, e.payload.outcome, e.payload.thumbToken, e.payload.pdfPage);
          // 历史记录在这里统一刷新:后端 finalize() 是先写历史再广播本事件,
          // 所以此刻列表已包含新条目。挂在面板组件上是不够的——面板收起时
          // 收不到,重开才看到(这正是「历史列表不实时更新」的原因)。
          scheduleHistoryRefresh();
        }),
        listen<{ id: string; phase: string }>('ocr://item-status', (e) =>
          applyStatus(e.payload.id, e.payload.phase),
        ),
        listen('batch://done', () => {
          app.batchRunning = false;
          app.batchEndedAt = Date.now();
          void continuePending();
          // 批末兜一次终态,确保合并刷新没吞掉最后几条
          void refreshHistory();
        }),
        listen<ItemOutcome>('shot://finished', (e) => {
          addItemWithOutcome(e.payload.item, e.payload.outcome);
        }),
        listen<Settings>('settings://changed', (e) => {
          applySettings(e.payload);
        }),
        listen<ToastMsg>('app://toast', (e) => {
          toast(e.payload.level, e.payload.message);
        }),
        // 自测模式:后端直接注入条目,走完整 UI 流程
        listen<ImageItem[]>('app://add-items', (e) => {
          addItems(e.payload);
        }),
        // PDF 后台识别进度
        listen<import('$lib/types').PdfPageDone>('pdf://page-done', (e) => {
          applyPdfPageDone(e.payload);
        }),
        listen<{
          id: string;
          total: number;
          completed: number;
          name: string;
          elapsedMs?: number;
        }>('pdf://ocr-done', (e) => {
          applyPdfOcrDone(e.payload);
          // completed<total 是被暂停:不打扰(用户刚点的暂停)
          const st = getActiveItem();
          if (st && st.item.id === e.payload.id && e.payload.completed >= e.payload.total) {
            toast('success', 'PDF 识别完成' + fmtElapsed(e.payload.elapsedMs));
          }
        }),
        // 自测模式:走用户同款 add_files 命令(invoke 返回通道)
        listen<string[]>('app://selftest-open', (e) => {
          api
            .addFiles(e.payload)
            .then(addItems)
            .catch((err) => toast('error', String(err)));
        }),
        // 批量导入进度
        listen<{ done: number; total: number }>('ingest://progress', (e) => {
          if (e.payload.done === e.payload.total) {
            // 让进度条先走到 100% 再收掉,否则最后几格会「没走到头就消失」
            setTimeout(() => {
              app.importing = null;
            }, 500);
          } else {
            app.importing = e.payload;
          }
        }),
        // 拖拽导入
        getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === 'enter' || event.payload.type === 'over') {
            app.dropActive = true;
            armDropWatchdog();
          } else {
            app.dropActive = false;
            clearTimeout(dropWatchdog);
            if (event.payload.type === 'drop') {
              const imgRe = /\.(png|jpe?g|bmp|pdf)$/i;
              const paths = event.payload.paths.filter((p) => imgRe.test(p));
              if (paths.length) {
                api
                  .addFiles(paths)
                  .then(addItems)
                  .catch((e) => toast('error', String(e)));
              }
            }
          }
        }),
      ];
      cleanup = () => {
        for (const p of unlisteners) void p.then((f) => f());
      };
    })();
    return () => cleanup?.();
  });

  // ---- 图片/结果 分屏拖拽(有结果才显示右栏) ----
  let splitArea: HTMLDivElement;
  // 旧默认 0.6 视为未手动拖过,迁移到新默认 0.5(图片面板更窄)
  let splitRatio = $state(
    Number(localStorage.getItem('qpp.split') ?? 0.5) === 0.6
      ? 0.5
      : Number(localStorage.getItem('qpp.split') ?? 0.5) || 0.5,
  );
  let splitting = false;
  const hasResult = $derived(!!getActiveItem()?.outcome);

  function startSplit(e: PointerEvent) {
    splitting = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onSplitMove(e: PointerEvent) {
    if (!splitting || !splitArea) return;
    const r = splitArea.getBoundingClientRect();
    splitRatio = Math.min(0.8, Math.max(0.2, (e.clientX - r.left) / r.width));
  }

  function endSplit(e: PointerEvent) {
    if (!splitting) return;
    splitting = false;
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    localStorage.setItem('qpp.split', String(splitRatio));
  }

  $effect(() => {
    const move = (e: PointerEvent) => onSplitMove(e);
    const up = (e: PointerEvent) => endSplit(e);
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
  });

  // 拖拽状态看门狗:leave/drop 事件丢失(WebView2 已知偶发)时 3s 后自动解除遮罩
  let dropWatchdog: ReturnType<typeof setTimeout> | undefined;
  function armDropWatchdog() {
    clearTimeout(dropWatchdog);
    dropWatchdog = setTimeout(() => (app.dropActive = false), 3000);
  }

  /** 打开/粘贴:快捷键与 mac 菜单栏共用 */
  async function openFromPicker() {
    try {
      addItems(await api.pickImages());
    } catch (err) {
      toast('error', String(err));
    }
  }

  async function pasteFromClipboard() {
    try {
      addItems([await api.readClipboardImage()]);
    } catch {
      /* 剪贴板没有图片,安静忽略 */
    }
  }

  async function onKeydown(e: KeyboardEvent) {
    // 输入框内不拦截
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA') return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'o') {
      e.preventDefault();
      void openFromPicker();
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'v') {
      e.preventDefault();
      void pasteFromClipboard();
    } else if ((e.ctrlKey || e.metaKey) && e.key >= '1' && e.key <= '3') {
      e.preventDefault();
      const views = ['records', 'stats', 'settings'] as const;
      setView(views[+e.key - 1]);
    }
  }

  // 当前文档名 → 窗口标题(mac 进 Cmd+Tab/菜单;Windows 任务栏 tooltip)+ mac 标题栏居中
  $effect(() => {
    const name = getActiveItem()?.item.name ?? '';
    app.docName = name;
    void api.setWindowTitle(name || null).catch(() => {});
  });
</script>

<svelte:window onkeydown={onKeydown} onblur={() => (app.dropActive = false)} />

<div class="app">
  <TitleBar />
  <div class="main">
    <IconRail />
    <section class="stage-col">
      <ToolBar />
      <div class="stage-area" bind:this={splitArea}>
        <div class="pane" style="width: {hasResult ? splitRatio * 100 : 100}%">
          <CanvasStage />
        </div>
        <div
          class="splitter"
          class:hidden={!hasResult}
          role="separator"
          aria-orientation="vertical"
          title="拖动调整图片 / 结果比例"
          onpointerdown={startSplit}
        ></div>
        <RightPane />
      </div>
      <PdfPageNav />
      <ThumbStrip />
    </section>
  </div>
  <Drawer />
  <DropOverlay />
  {#if app.importing}
    <div class="importing-chip">
      <span class="spin"><Icon name="spinner" size={13} spinning /></span>
      导入图片 {app.importing.done}/{app.importing.total}
    </div>
  {/if}
  <Toast />
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--bg-app);
    overflow: hidden;
  }
  .importing-chip {
    position: absolute;
    top: calc(var(--titlebar-h) + 14px);
    left: 50%;
    transform: translateX(-50%);
    z-index: 60;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 14px;
    border-radius: 999px;
    font-size: 12.5px;
    color: var(--text-primary);
    background: color-mix(in srgb, var(--bg-elevated) 88%, transparent);
    backdrop-filter: blur(10px);
    border: 1px solid var(--border-subtle);
    box-shadow: var(--shadow-md);
    animation: rise-in var(--speed) var(--ease-out);
  }
  .importing-chip .spin {
    display: inline-flex;
    color: var(--accent);
  }
  .main {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .stage-col {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .stage-area {
    position: relative;
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .pane {
    position: relative;
    min-width: 0;
    overflow: hidden;
  }
  .splitter.hidden {
    display: none;
  }
  .splitter {
    flex: none;
    width: 7px;
    margin: 0 -3px;
    z-index: 20;
    cursor: col-resize;
    position: relative;
  }
  .splitter::after {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    left: 3px;
    width: 1px;
    background: var(--border-subtle);
    transition: background var(--speed-fast);
  }
  .splitter:hover::after,
  .splitter:active::after {
    background: var(--accent);
    width: 2px;
  }
</style>
