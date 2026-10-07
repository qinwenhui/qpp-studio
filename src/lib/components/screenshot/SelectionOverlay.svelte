<script lang="ts">
  /** 截图覆盖层:冻结画面 + 暗罩 + 拖框选区。挂在 screenshot.html。 */
  import { onMount } from 'svelte';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { api } from '$lib/api';
  import type { ShotMonitor } from '$lib/types';

  const label = getCurrentWebview().label;
  const mon = Number(label.replace(/^shot-/, ''));

  let info = $state<ShotMonitor | null>(null);
  let error = $state('');
  let sel = $state<{ x1: number; y1: number; x2: number; y2: number } | null>(null);
  let started = false;

  // 覆盖窗口是 Rust 在登记截图会话之前建的,本 webview 的 JS 可能先跑起来,
  // 此时 shotWindowMonitor 会报「截图会话已结束」。短轮询到会话就绪为止——
  // 固定延迟是赌时序,慢了就变成用户看到报错、框选失灵。
  const loadInfo = async () => {
    for (let i = 0; i < 20; i++) {
      try {
        info = await api.shotWindowMonitor(label);
        return;
      } catch (e) {
        error = String(e);
      }
      await new Promise((r) => setTimeout(r, 25));
    }
  };

  onMount(() => {
    void loadInfo();
  });

  const rect = $derived(
    sel
      ? {
          left: Math.min(sel.x1, sel.x2),
          top: Math.min(sel.y1, sel.y2),
          width: Math.abs(sel.x2 - sel.x1),
          height: Math.abs(sel.y2 - sel.y1),
        }
      : null,
  );

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    started = true;
    sel = { x1: e.clientX, y1: e.clientY, x2: e.clientX, y2: e.clientY };
  }

  function onPointerMove(e: PointerEvent) {
    if (!started || !sel) return;
    sel = { ...sel, x2: e.clientX, y2: e.clientY };
  }

  function onPointerUp() {
    if (!started || !sel || !info) return;
    started = false;
    const dpr = info.dpr || window.devicePixelRatio || 1;
    const x = Math.round(Math.min(sel.x1, sel.x2) * dpr);
    const y = Math.round(Math.min(sel.y1, sel.y2) * dpr);
    const w = Math.round(Math.abs(sel.x2 - sel.x1) * dpr);
    const h = Math.round(Math.abs(sel.y2 - sel.y1) * dpr);
    sel = null;
    if (w >= 4 && h >= 4) {
      // Rust 侧会关掉本窗口;无需等待
      void api.screenshotFinish(mon, x, y, w, h).catch(() => {});
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      void api.screenshotCancel();
    }
  }

  /** 右键 = 取消(与 Esc 等价) */
  function onContextMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    void api.screenshotCancel();
  }
</script>

<svelte:window
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onkeydown={onKeydown}
/>

<div class="overlay" oncontextmenu={onContextMenu}>
  {#if info}
    <img
      class="frozen"
      src={info.url}
      style="width: 100vw; height: 100vh"
      alt=""
      draggable="false"
    />
    <div class="mask" class:has-sel={!!rect}>
      {#if rect}
        <div class="sel" style="left:{rect.left}px; top:{rect.top}px; width:{rect.width}px; height:{rect.height}px">
          <span class="size">
            {Math.round(rect.width * (info.dpr || 1))} × {Math.round(rect.height * (info.dpr || 1))}
          </span>
        </div>
      {/if}
    </div>
    <div class="hint">
      拖拽框选识别区域 · <kbd>Esc</kbd> / 右键 取消
    </div>
  {:else if error}
    <div class="err">{error}</div>
  {:else}
    <div class="loading">正在冻结屏幕…</div>
  {/if}
</div>

<style>
  :global(html),
  :global(body) {
    overflow: hidden;
    background: #000;
    user-select: none;
  }
  .overlay {
    position: fixed;
    inset: 0;
    cursor: crosshair;
    overflow: hidden;
  }
  .frozen {
    position: absolute;
    inset: 0;
    object-fit: fill;
    pointer-events: none;
  }
  .mask {
    position: absolute;
    inset: 0;
    background: rgb(0 0 0 / 0.35);
    transition: background var(--speed-fast, 120ms);
  }
  .mask.has-sel {
    background: transparent;
  }
  /* 经典 box-shadow 挖洞遮罩:选区外的四边都是“阴影” */
  .sel {
    position: absolute;
    border: 1.5px solid #2dd4a7;
    background: transparent;
    box-shadow: 0 0 0 100000px rgb(0 0 0 / 0.35);
    cursor: crosshair;
  }
  .size {
    position: absolute;
    left: 4px;
    top: calc(100% + 6px);
    padding: 2px 8px;
    border-radius: 6px;
    background: rgb(20 24 30 / 0.85);
    color: #e8f5ef;
    font-family: 'Cascadia Code', Consolas, monospace;
    font-size: 11.5px;
    white-space: nowrap;
  }
  .hint {
    position: absolute;
    top: 18px;
    left: 50%;
    transform: translateX(-50%);
    padding: 7px 16px;
    border-radius: 999px;
    background: rgb(20 24 30 / 0.82);
    color: #dfe7ef;
    font-size: 12.5px;
    pointer-events: none;
    backdrop-filter: blur(6px);
  }
  .hint kbd {
    padding: 1px 6px;
    border-radius: 4px;
    background: rgb(255 255 255 / 0.14);
    font-family: 'Cascadia Code', Consolas, monospace;
    font-size: 11px;
  }
  .err,
  .loading {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: #9aa7b8;
    font-size: 13px;
    font-family: system-ui, sans-serif;
  }
</style>
