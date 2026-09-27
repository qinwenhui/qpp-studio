<script lang="ts">
  /** 画布:DPR 感知缩放/平移 + 识别四边形绘制与双向命中联动。 */
  import { onMount } from 'svelte';
  import CanvasHud from '$lib/components/CanvasHud.svelte';
  import { mediaUrl } from '$lib/api';
  import { app } from '$lib/state/app.svelte';
  import { getActiveItem, imagesStore } from '$lib/state/images.svelte';
  import { canvasBus } from '$lib/state/canvasBus.svelte';
  import EmptyState from '$lib/components/EmptyState.svelte';
  import Icon from '$lib/components/Icon.svelte';
  import type { OcrResult, TextLine } from '$lib/types';

  let container: HTMLDivElement;
  let canvas = $state<HTMLCanvasElement>();
  let ctx: CanvasRenderingContext2D | null = null;

  /** 视图变换:screen = image * scale + t;rotation 为显示旋转(0/90/180/270,顺时针) */
  const view = $state({ scale: 1, tx: 0, ty: 0 });
  let rotation = $state(0);

  /** 旋转后的内容尺寸(画布坐标系看到的宽高) */
  function contentDims(): { rw: number; rh: number } {
    if (!imgEl) return { rw: 0, rh: 0 };
    const iw = imgEl.naturalWidth;
    const ih = imgEl.naturalHeight;
    return rotation % 180 === 0 ? { rw: iw, rh: ih } : { rw: ih, rh: iw };
  }

  /** 内容坐标 → 图像坐标(旋转的逆变换) */
  function contentToImage(cx: number, cy: number): [number, number] {
    if (!imgEl) return [cx, cy];
    const iw = imgEl.naturalWidth;
    const ih = imgEl.naturalHeight;
    switch (rotation) {
      case 90:
        return [cy, ih - cx]; // 前向:image(x,y) → content(ih-y, x)
      case 180:
        return [iw - cx, ih - cy];
      case 270:
        return [iw - cy, cx]; // 前向:image(x,y) → content(y, iw-x)
      default:
        return [cx, cy];
    }
  }

  let imgEl = $state<HTMLImageElement | null>(null);
  let imgLoadedId = ''; // 当前已加载图片的 item id
  let raf = 0;
  let dragging = $state(false);
  let lastPx = 0;
  let lastPy = 0;
  let hoverLine = -1;

  const dpr = () => window.devicePixelRatio || 1;
  const active = $derived(getActiveItem());
  const result = $derived<OcrResult | undefined>(active?.outcome?.result);
  const lines = $derived<TextLine[]>(result?.lines ?? []);

  // ---- 图片加载 ----
  $effect(() => {
    const item = active?.item;
    if (!item) {
      imgEl = null;
      imgLoadedId = '';
      requestDraw();
      return;
    }
    if (imgLoadedId === item.id) return;
    const el = new Image();
    el.src = mediaUrl(item.mediaToken);
    imgLoadedId = item.id;
    el.decode().then(
      () => {
        if (imgLoadedId === item.id) {
          imgEl = el;
          fit();
        }
      },
      () => {
        if (imgLoadedId === item.id) {
          imgEl = null;
          requestDraw();
          // 失败可诊断:把 URL 报给用户而不是无声空白
          import('$lib/state/app.svelte').then((m) =>
            m.toast('error', `图片加载失败:${el.src.slice(0, 80)}`),
          );
        }
      },
    );
  });

  // ---- 尺寸 ----
  $effect(() => {
    if (!container) return;
    const ro = new ResizeObserver(() => {
      syncSize();
      // 面板展开/收起或窗口变化时重新适配,否则图片停留在旧布局的居中位置,
      // 视觉上"偏右被结果面板压住"
      fit();
    });
    ro.observe(container);
    return () => ro.disconnect();
  });

  function syncSize() {
    if (!canvas || !container) return;
    const r = container.getBoundingClientRect();
    const d = dpr();
    canvas.width = Math.max(1, Math.round(r.width * d));
    canvas.height = Math.max(1, Math.round(r.height * d));
    canvas.style.width = `${r.width}px`;
    canvas.style.height = `${r.height}px`;
  }

  // ---- 画布命令注册 ----
  $effect(() => {
    canvasBus.fit = fit;
    canvasBus.zoomIn = () => zoomAt(centerX(), centerY(), 1.25);
    canvasBus.zoomOut = () => zoomAt(centerX(), centerY(), 1 / 1.25);
    canvasBus.oneToOne = oneToOne;
    canvasBus.rotateCW = () => {
      rotation = (rotation + 90) % 360;
      fit();
    };
    canvasBus.rotateCCW = () => {
      rotation = (rotation + 270) % 360;
      fit();
    };
    canvasBus.getRotation = () => rotation;
    return () => {
      canvasBus.fit = null;
      canvasBus.zoomIn = null;
      canvasBus.zoomOut = null;
      canvasBus.oneToOne = null;
      canvasBus.rotateCW = null;
      canvasBus.rotateCCW = null;
      canvasBus.getRotation = null;
    };
  });

  function centerX() {
    return container ? container.clientWidth / 2 : 0;
  }
  function centerY() {
    return container ? container.clientHeight / 2 : 0;
  }

  // ---- 视图操作 ----
  function fit() {
    if (!imgEl || !container) return;
    const { rw, rh } = contentDims();
    const cw = container.clientWidth;
    const ch = container.clientHeight;
    if (!rw || !rh || !cw || !ch) return;
    const margin = 28;
    const s = Math.min((cw - margin) / rw, (ch - margin) / rh);
    view.scale = s;
    view.tx = (cw - rw * s) / 2;
    view.ty = (ch - rh * s) / 2;
    requestDraw();
  }

  function oneToOne() {
    if (!imgEl || !container) return;
    const { rw, rh } = contentDims();
    view.scale = 1;
    view.tx = (container.clientWidth - rw) / 2;
    view.ty = (container.clientHeight - rh) / 2;
    requestDraw();
  }

  function zoomAt(mx: number, my: number, factor: number) {
    const next = view.scale * factor;
    if (next < 0.02 || next > 40) return;
    view.tx = mx - (mx - view.tx) * factor;
    view.ty = my - (my - view.ty) * factor;
    view.scale = next;
    requestDraw();
  }

  // ---- 指针交互 ----
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const rect = canvas!.getBoundingClientRect();
    const factor = Math.exp(-e.deltaY * 0.0016);
    zoomAt(e.clientX - rect.left, e.clientY - rect.top, factor);
  }

  // 滚轮缩放:显式非被动监听(随 canvas 元素出现而挂载)
  $effect(() => {
    const el = canvas;
    if (!el) return;
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  });

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    dragging = true;
    lastPx = e.clientX;
    lastPy = e.clientY;
    canvas?.setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    const rect = canvas!.getBoundingClientRect();
    if (dragging) {
      view.tx += e.clientX - lastPx;
      view.ty += e.clientY - lastPy;
      lastPx = e.clientX;
      lastPy = e.clientY;
      requestDraw();
      return;
    }
    // 命中检测(先到内容坐标,再逆旋转回图像坐标)
    if (!imgEl || !lines.length) {
      setHover(-1);
      return;
    }
    const cx = (e.clientX - rect.left - view.tx) / view.scale;
    const cy = (e.clientY - rect.top - view.ty) / view.scale;
    const [ix, iy] = contentToImage(cx, cy);
    let hit = -1;
    for (let i = lines.length - 1; i >= 0; i--) {
      if (pointInQuad(ix, iy, lines[i].pts)) {
        hit = i;
        break;
      }
    }
    setHover(hit);
  }

  function setHover(i: number) {
    if (hoverLine === i) return;
    hoverLine = i;
    app.focus = i >= 0 && active ? { id: active.item.id, line: i } : null;
    requestDraw();
  }

  function onPointerUp(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    canvas?.releasePointerCapture(e.pointerId);
  }

  function onClick(e: MouseEvent) {
    if (!active) return;
    app.selected = hoverLine >= 0 ? { id: active.item.id, line: hoverLine } : null;
    requestDraw();
  }

  function onLeave() {
    setHover(-1);
  }

  // ---- 几何 ----
  function pointInQuad(x: number, y: number, pts: [number, number][]): boolean {
    let sign = 0;
    for (let i = 0; i < 4; i++) {
      const [ax, ay] = pts[i];
      const [bx, by] = pts[(i + 1) & 3];
      const cross = (bx - ax) * (y - ay) - (by - ay) * (x - ax);
      if (cross !== 0) {
        const s = cross > 0 ? 1 : -1;
        if (sign === 0) sign = s;
        else if (s !== sign) return false;
      }
    }
    return true;
  }

  /** 置信度 → 颜色(红 0.5 → 绿 0.95)。 */
  function confColor(c: number): string {
    const t = Math.min(1, Math.max(0, (c - 0.5) / 0.45));
    const hue = 8 + t * 132;
    const sat = 72 - t * 8;
    const light = 56 - t * 6;
    return `hsl(${hue} ${sat}% ${light}%)`;
  }

  // ---- 绘制 ----
  function requestDraw() {
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      draw();
    });
  }

  function draw() {
    if (!canvas) return;
    // canvas 可能被模板销毁重建(队列清空的瞬间):ctx 必须属于当前元素,否则画在死画布上
    if (!ctx || ctx.canvas !== canvas) {
      ctx = canvas.getContext('2d');
      syncSize();
    }
    if (!ctx) return;
    const d = dpr();
    ctx.setTransform(d, 0, 0, d, 0, 0);
    ctx.clearRect(0, 0, canvas.width / d, canvas.height / d);

    if (!imgEl) return;
    ctx.save();
    ctx.translate(view.tx, view.ty);
    ctx.scale(view.scale, view.scale);
    ctx.imageSmoothingEnabled = view.scale < 2.5;
    // 旋转:坐标系从「内容」切回「图像」,之后一律用图像坐标绘制
    const iw = imgEl.naturalWidth;
    const ih = imgEl.naturalHeight;
    if (rotation === 90) {
      ctx.translate(ih, 0);
      ctx.rotate(Math.PI / 2);
    } else if (rotation === 180) {
      ctx.translate(iw, ih);
      ctx.rotate(Math.PI);
    } else if (rotation === 270) {
      ctx.translate(0, iw);
      ctx.rotate(-Math.PI / 2);
    }

    // 阴影底托
    ctx.shadowColor = 'rgba(0,0,0,0.45)';
    ctx.shadowBlur = 24 / view.scale;
    ctx.shadowOffsetY = 6 / view.scale;
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, imgEl.naturalWidth, imgEl.naturalHeight);
    ctx.shadowColor = 'transparent';
    ctx.shadowBlur = 0;
    ctx.shadowOffsetY = 0;

    ctx.drawImage(imgEl, 0, 0);

    // 识别框
    const focused = app.focus && app.focus.id === active?.item.id ? app.focus.line : -1;
    const selected = app.selected && app.selected.id === active?.item.id ? app.selected.line : -1;
    if (app.showBoxes && lines.length) {
      for (let i = 0; i < lines.length; i++) {
        const l = lines[i];
        const isFocus = i === focused || i === hoverLine;
        const isSel = i === selected;
        const quad = l.pts;

        if (isFocus || isSel) {
          ctx.beginPath();
          ctx.moveTo(quad[0][0], quad[0][1]);
          for (let k = 1; k < 4; k++) ctx.lineTo(quad[k][0], quad[k][1]);
          ctx.closePath();
          ctx.fillStyle = isSel ? 'rgba(255,255,255,0.22)' : 'rgba(255,255,255,0.12)';
          ctx.fill();
        }

        ctx.beginPath();
        ctx.moveTo(quad[0][0], quad[0][1]);
        for (let k = 1; k < 4; k++) ctx.lineTo(quad[k][0], quad[k][1]);
        ctx.closePath();
        ctx.lineWidth = (isSel ? 2.4 : isFocus ? 2 : 1.4) / view.scale;
        ctx.strokeStyle = confColor(l.confidence);
        ctx.stroke();

        if (isSel) {
          ctx.setLineDash([6 / view.scale, 4 / view.scale]);
          ctx.lineWidth = 1.2 / view.scale;
          ctx.strokeStyle = 'rgba(255,255,255,0.9)';
          ctx.stroke();
          ctx.setLineDash([]);
        }
      }
    }
    ctx.restore();
  }

  // 响应式重绘:视图/数据/联动/canvas 出现 变化
  $effect(() => {
    void view.scale;
    void view.tx;
    void view.ty;
    void rotation;
    void app.showBoxes;
    void app.focus?.line;
    void app.focus?.id;
    void app.selected?.line;
    void lines.length;
    void imgLoadedId;
    void canvas; // canvas 元素随首张图片出现 → 触发首绘
    requestDraw();
  });

  onMount(() => {
    syncSize();
    requestDraw();
  });

  const zoomPct = $derived(Math.round(view.scale * 100));
</script>

<div class="stage" bind:this={container}>
  {#if imagesStore.items.length === 0}
    <EmptyState />
  {:else}
    <CanvasHud />
  {/if}
  <!-- canvas 常驻:队列清空瞬间也不卸载,否则销毁重建会丢 2D 上下文 -->
  <canvas
    bind:this={canvas}
    class:grabbing={dragging}
    class:veiled={imagesStore.items.length === 0}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onclick={onClick}
    onpointerleave={onLeave}
    ondblclick={fit}
  ></canvas>

  {#if imagesStore.items.length > 0}
    {#if active && (active.phase === 'running' || active.phase === 'queued')}
      <div class="running-chip">
        <Icon name="spinner" size={14} spinning />
        识别中…
      </div>
    {/if}

    {#if imgEl}
      <div class="zoom-chip">
        {zoomPct}% · {imgEl.naturalWidth}×{imgEl.naturalHeight}
        {#if rotation !== 0}
          · ↻{rotation}°
        {/if}
        {#if lines.length}
          · {lines.length} 行
        {/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .stage {
    position: absolute;
    inset: 0;
    overflow: hidden;
    background: var(--bg-canvas);
    /* 细网格点阵,呼应深色科技风 */
    background-image: radial-gradient(
      color-mix(in srgb, var(--text-faint) 14%, transparent) 1px,
      transparent 1px
    );
    background-size: 22px 22px;
  }
  canvas {
    position: absolute;
    inset: 0;
    cursor: grab;
    touch-action: none;
  }
  canvas.veiled {
    display: none;
  }
  canvas.grabbing {
    cursor: grabbing;
  }
  .zoom-chip,
  .running-chip {
    position: absolute;
    left: 12px;
    bottom: 12px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 26px;
    padding: 0 10px;
    border-radius: 999px;
    font-size: 11.5px;
    font-family: var(--font-mono);
    color: var(--text-secondary);
    background: color-mix(in srgb, var(--bg-elevated) 82%, transparent);
    backdrop-filter: blur(8px);
    border: 1px solid var(--border-subtle);
    animation: rise-in var(--speed) var(--ease-out);
  }
  .running-chip {
    left: 50%;
    top: 14px;
    bottom: auto;
    transform: translateX(-50%);
    color: var(--accent);
  }
</style>
