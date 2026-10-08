<script lang="ts">
  /** 纸张视图:识别结果按原始坐标重排成一张"电子纸"。
   *  页面 = 原图 1:1 比例;每行文字落在框的位置,字号≈框高;
   *  竖排文本自动 writing-mode;真实 DOM 文本可拖选复制。 */
  import Icon from '$lib/components/Icon.svelte';
  import ModeSwitch from '$lib/components/ModeSwitch.svelte';
  import { api } from '$lib/api';
  import { app, setFidelity, toast } from '$lib/state/app.svelte';
  import { getActiveItem, reRecognize } from '$lib/state/images.svelte';
  import { onMount } from 'svelte';
  import type { TextLine } from '$lib/types';

  const active = $derived(getActiveItem());
  const result = $derived(active?.outcome?.result);
  const lines = $derived<TextLine[]>(result?.lines ?? []);
  const page = $derived(active ? { w: active.item.w, h: active.item.h } : null);

  /** zoom <= 0 表示"适应窗口",实际缩放系数在 effective 里算 */
  let zoom = $state(0);
  let viewport: HTMLDivElement;

  let vw = $state(0);
  let vh = $state(0);

  const effective = $derived.by(() => {
    if (!page || vw <= 0) return 1;
    if (zoom > 0) return zoom;
    return Math.min((vw - 56) / page.w, (vh - 56) / page.h);
  });

  /** 字符宽度系数(以 em 为单位):CJK≈1 格,数字/字母约 0.58,空格 0.3 */
  function charUnits(ch: string): number {
    const cp = ch.codePointAt(0) ?? 0;
    if (cp === 0x20) {
      return 0.3;
    } else if (
      (cp >= 0x2e80 && cp <= 0x9fff) ||
      (cp >= 0x3400 && cp <= 0x4dbf) ||
      (cp >= 0xf900 && cp <= 0xfaff) ||
      (cp >= 0xff01 && cp <= 0xff60) ||
      (cp >= 0x3000 && cp <= 0x303f) ||
      (cp >= 0x2010 && cp <= 0x2027)
    ) {
      return 1.02;
    } else {
      return 0.58;
    }
  }

  /** 每行的纸面布局:外接框 + 逐字坐标(engine 2307909)或估算兜底。 */
  const layout = $derived.by(() => {
    return lines.map((l, i) => {
      const xs = l.pts.map((p) => p[0]);
      const ys = l.pts.map((p) => p[1]);
      const x = Math.min(...xs);
      const x2 = Math.max(...xs);
      const y = Math.min(...ys);
      const y2 = Math.max(...ys);
      const w = Math.max(4, x2 - x);
      const h = Math.max(4, y2 - y);
      const vertical = h > w * 1.6;

      // 逐字坐标路径:位置按真实四边形,字号行内统一(字符盒高度均值,
      // 避免逐字取字号造成"一大一小"的毛刺感;行间大小差异是原文事实,保留)
      const chars: {
        text: string;
        x: number;
        y: number;
        w: number;
        h: number;
        space: boolean;
      }[] = [];
      let cfs = 0;
      if (l.chars && l.chars.length > 0) {
        const boxes = l.chars.map((c) => {
          // 四点包围盒:不依赖角点顺序(竖排行的角点序是转正后的)
          const cxs = c.pts.map((p) => p[0]);
          const cys = c.pts.map((p) => p[1]);
          return {
            text: c.text,
            x0: Math.min(...cxs),
            x1: Math.max(...cxs),
            y0: Math.min(...cys),
            y1: Math.max(...cys),
          };
        });
        // 行内统一字号:字符盒"短边"的中位数(横排=高,竖排=宽)
        const shorts = boxes
          .filter((b) => b.text !== ' ')
          .map((b) => (vertical ? b.x1 - b.x0 : b.y1 - b.y0))
          .sort((a, b) => a - b);
        const med = shorts.length ? shorts[shorts.length >> 1] : (vertical ? w : h);
        cfs = med * 0.86;
        // 字号还必须被"相邻字符起点间距"约束:每字步进≈字号×宽度系数,
        // 大于间距时相邻字符会重叠(中文夹数字/字母的行最常见)。
        // 取全部约束的低分位,个别贴脸的坏盒不至于把整行压到看不清。
        const pitches: number[] = [];
        for (let k = 0; k + 1 < boxes.length; k++) {
          const pitch = vertical ? boxes[k + 1].y0 - boxes[k].y0 : boxes[k + 1].x0 - boxes[k].x0;
          const u = charUnits(boxes[k].text);
          if (pitch > 1 && u > 0) pitches.push(pitch / u);
        }
        if (pitches.length) {
          pitches.sort((a, b) => a - b);
          cfs = Math.min(cfs, pitches[Math.min(pitches.length - 1, pitches.length >> 3)]);
        }
        cfs = Math.max(3, cfs);
        for (const b of boxes) {
          chars.push({
            text: b.text,
            // 字符盒原点定位;span 定宽高 + flex 居中,字形落在盒心
            x: b.x0 - x,
            y: b.y0 - y,
            w: Math.max(2, b.x1 - b.x0),
            h: Math.max(2, b.y1 - b.y0),
            space: b.text === ' ',
          });
        }
      }

      // 原向模式的行倾角:横排取 TL→TR 边的角度(竖排字符已按真实坐标直立,无需旋转)
      const angle = vertical ? 0 : Math.atan2(l.pts[1][1] - l.pts[0][1], l.pts[1][0] - l.pts[0][0]);

      // 估算兜底路径(引擎无 chars 时):字号需保证"字数×字宽 ≤ 框宽"。
      // 框高取左右边长均值(抗旋转;y 跨度对倾斜行会虚高)
      let units = 0;
      for (const ch of l.text) units += charUnits(ch);
      const eL = Math.hypot(l.pts[3][0] - l.pts[0][0], l.pts[3][1] - l.pts[0][1]);
      const eR = Math.hypot(l.pts[2][0] - l.pts[1][0], l.pts[2][1] - l.pts[1][1]);
      const edge = (eL + eR) / 2 || h;
      const fsBox = (vertical ? w : edge) * 0.82;
      const fsFit = units > 0 ? (vertical ? h : w) / units : fsBox;
      const fs = Math.max(4, Math.min(fsBox, fsFit));
      // 180° 判定以逐字坐标的阅读方向为准(cls 对竖排条有 90°/180° 歧义)。
      // ⚠ 竖排的阅读惯例分语种:CJK 上→下(首字在下 = 倒置);拉丁竖排惯例
      // 下→上(首字在下 = 正常,实测 img-001 的竖排 No native dependency)。
      // 横行首字在右 = 倒置;无 chars(直提)回落标志。
      const hasCJK = /[⺀-鿿豈-﫿　-〿！-｠]/.test(l.text);
      let flip = l.rotation === 180;
      if (chars.length >= 2) {
        const c0 = chars[0];
        const cN = chars[chars.length - 1];
        if (vertical) {
          const firstBelow = c0.y + c0.h / 2 > cN.y + cN.h / 2;
          flip = hasCJK ? firstBelow : !firstBelow;
        } else {
          flip = c0.x + c0.w / 2 > cN.x + cN.w / 2;
        }
      }
      return { i, x, y, w, h, fs, cfs, angle, flip, vertical, chars, text: l.text, conf: l.confidence };
    });
  });

  function fit() {
    zoom = 0;
  }

  function setZoom(z: number) {
    zoom = Math.min(6, Math.max(0.08, z));
  }

  onMount(() => {
    const ro = new ResizeObserver(() => {
      const r = viewport.getBoundingClientRect();
      vw = r.width;
      vh = r.height;
    });
    ro.observe(viewport);
    // 滚轮缩放(显式非被动)
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      setZoom(effective * Math.exp(-e.deltaY * 0.0016));
    };
    viewport.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      ro.disconnect();
      viewport.removeEventListener('wheel', onWheel);
    };
  });

  function hover(i: number | null) {
    app.focus = i !== null && active ? { id: active.item.id, line: i } : null;
  }

  // ---- 右键菜单:复制选中 / 复制此行 / 复制全文 ----
  let ctxMenu = $state<{ x: number; y: number; line: number | null } | null>(null);

  function onContextMenu(e: MouseEvent) {
    const el = (e.target as HTMLElement)?.closest?.('.line');
    if (!el) return;
    e.preventDefault();
    e.stopPropagation();
    const idx = Number(el.getAttribute('data-line'));
    ctxMenu = { x: e.clientX, y: e.clientY, line: Number.isNaN(idx) ? null : idx };
  }

  function selectedText(): string {
    return window.getSelection()?.toString() ?? '';
  }

  async function ctxCopy(text: string, tip: string) {
    const sel = selectedText();
    ctxMenu = null;
    try {
      await api.copyText(text);
      toast('success', tip);
    } catch (e) {
      toast('error', String(e));
    }
  }

  function allText(): string {
    return lines
      .map((l) => l.text)
      .filter(Boolean)
      .join('\n');
  }

  async function copyAll() {
    try {
      await api.copyText(allText());
      toast('success', '已复制全文');
    } catch (e) {
      toast('error', String(e));
    }
  }

  const pct = $derived(Math.round(effective * 100));
</script>

<svelte:window
  onclick={() => (ctxMenu = null)}
  onkeydown={(e) => e.key === 'Escape' && (ctxMenu = null)}
/>

<div class="paper-stage" bind:this={viewport} oncontextmenu={onContextMenu}>
  {#if page && lines.length}
    <div class="toolbar">
      <ModeSwitch />
      <span class="sep"></span>
      <button class="icon-btn" onclick={() => setZoom(effective / 1.25)} title="缩小">
        <Icon name="zoomOut" size={15} />
      </button>
      <span class="pct">{pct}%</span>
      <button class="icon-btn" onclick={() => setZoom(effective * 1.25)} title="放大">
        <Icon name="zoomIn" size={15} />
      </button>
      <button class="icon-btn" onclick={fit} title="适应窗口">
        <Icon name="fit" size={15} />
      </button>
      <button
        class="icon-btn"
        class:active={app.fidelity}
        onclick={() => setFidelity(!app.fidelity)}
        title={app.fidelity ? '原向:按原始角度/方向渲染(点击恢复阅读版式)' : '阅读版式(点击切换为按原始角度/方向渲染)'}
      >
        <Icon name="rotateCw" size={15} />
      </button>
      <span class="sep"></span>
      <button class="btn" onclick={copyAll}>
        <Icon name="copy" size={13} />
        复制全文
      </button>
      {#if active}
        <button
          class="btn rerun"
          title="重新识别这张(结果与耗时原地更新)"
          onclick={() => reRecognize(active.item.id)}
        >
          <Icon name="refresh" size={13} />
          重新识别
        </button>
      {/if}
    </div>

    <div class="scroll">
      <div class="sizer" style="width:{page.w * effective}px; height:{page.h * effective}px">
        <div
          class="sheet selectable"
          style="width:{page.w}px; height:{page.h}px; zoom: {effective}"
        >
    {#each layout as l (l.i)}
            <div
              class="line"
              class:vertical={l.vertical && l.chars.length === 0}
              class:focused={app.focus?.line === l.i && app.focus?.id === active?.item.id}
              style="left:{l.x}px; top:{l.y}px; width:{l.w}px; height:{l.h}px; font-size:{l.fs}px; transform: {app.fidelity ? (l.flip ? `rotate(180deg)` : (l.angle ? `rotate(${l.angle}rad)` : 'none')) : 'none'}; transform-origin: {l.flip ? 'center' : '0 0'}"
              title="{l.text}
置信度 {(l.conf * 100).toFixed(0)}%"
              data-line={l.i}
              onmouseenter={() => hover(l.i)}
              onmouseleave={() => hover(null)}
            >
              {#if l.chars.length > 0}
                {#each l.chars as c, ci (ci)}
                  <span
                    class="ch"
                    class:sp={c.space}
                    style="left:{l.flip && !app.fidelity && !l.vertical ? l.w - c.x - c.w : c.x}px; top:{l.flip && !app.fidelity && l.vertical ? l.h - c.y - c.h : c.y}px; width:{c.w}px; height:{c.h}px; font-size:{l.cfs}px"
                  >{c.text}</span>
                {/each}
              {:else}{l.text}{/if}
            </div>
          {/each}
        </div>
      </div>
    </div>

    <!-- 左下角信息徽标,与图片画布的缩放标签对称 -->
    <div class="stats-chip" title="引擎端到端耗时">
      {lines.length} 行 · <b>{result?.timings.totalMs.toFixed(1)}ms</b>{#if pct !== 100}&nbsp;·
        {pct}%{/if}
    </div>

    {#if ctxMenu}
      <div class="ctx-menu" style="left:{ctxMenu.x}px; top:{ctxMenu.y}px">
        {#if selectedText()}
          <button onclick={() => ctxCopy(selectedText(), '已复制选中文字')}>
            <Icon name="copy" size={13} />
            复制选中
          </button>
        {/if}
        {#if ctxMenu.line !== null}
          <button onclick={() => ctxCopy(lines[ctxMenu!.line!]?.text ?? '', '已复制该行')}>
            <Icon name="type" size={13} />
            复制此行
          </button>
        {/if}
        <button onclick={() => ctxCopy(allText(), '已复制全文')}>
          <Icon name="fileText" size={13} />
          复制全文
        </button>
      </div>
    {/if}
  {:else if active && (active.phase === 'running' || active.phase === 'queued')}
    <div class="placeholder">
      <span><Icon name="spinner" size={15} spinning /></span>
      识别中…
    </div>
  {:else if active && active.phase === 'error'}
    <div class="placeholder err selectable">{active.outcome?.error ?? '识别失败'}</div>
  {:else}
    <div class="placeholder">识别完成后,这里就是一张可以选中复制的"纸"</div>
  {/if}
</div>

<style>
  .paper-stage {
    position: absolute;
    inset: 0;
    z-index: 5;
    display: flex;
    flex-direction: column;
    background: var(--bg-canvas);
    background-image: radial-gradient(
      color-mix(in srgb, var(--text-faint) 14%, transparent) 1px,
      transparent 1px
    );
    background-size: 22px 22px;
  }
  
  .toolbar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-subtle);
    background: color-mix(in srgb, var(--bg-panel) 70%, transparent);
    flex: none;
  }
  .pct {
    min-width: 44px;
    text-align: center;
    font-size: 11.5px;
    font-family: var(--font-mono);
    color: var(--text-secondary);
  }
  .sep {
    width: 1px;
    height: 16px;
    margin: 0 6px;
    background: var(--border-subtle);
  }
  .stats-chip {
    position: absolute;
    left: 12px;
    bottom: 12px;
    z-index: 10;
    display: inline-flex;
    align-items: center;
    gap: 4px;
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
    pointer-events: none;
  }
  .stats-chip b {
    color: var(--success);
    font-weight: 700;
  }
  .ctx-menu {
    position: fixed;
    z-index: 300;
    min-width: 148px;
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
  .ctx-menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    border-radius: var(--radius-sm);
    font-size: 12.5px;
    color: var(--text-primary);
    text-align: left;
  }
  .ctx-menu button:hover {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .btn.rerun:hover {
    color: var(--accent);
  }

  .scroll {
    flex: 1;
    overflow: auto;
    display: grid;
    place-items: center;
    padding: 28px;
  }
  .sizer {
    position: relative;
    flex: none;
  }
  .sheet {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: top left;
    background: var(--paper-bg, #fbfaf6);
    color: var(--paper-ink, #26241f);
    border-radius: 3px;
    box-shadow:
      0 1px 2px rgb(0 0 0 / 0.18),
      0 10px 34px rgb(0 0 0 / 0.3);
    font-family: Georgia, 'Source Han Serif SC', 'Noto Serif SC', 'SimSun', serif;
    line-height: 1;
    overflow: hidden;
  }
  .line {
    position: absolute;
    display: flex;
    align-items: center;
    justify-content: flex-start;
    white-space: nowrap;
    overflow: visible;
    cursor: text;
    transition: background var(--speed-fast);
    user-select: text;
    -webkit-user-select: text;
  }
  .sheet ::selection {
    background: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .line.vertical {
    writing-mode: vertical-rl;
    justify-content: flex-start;
  }
  .line.focused {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-radius: 2px;
  }
  .ch {
    position: absolute;
    display: flex;
    align-items: center;
    justify-content: center;
    line-height: 1;
    overflow: visible;
  }
  .ch.sp {
    color: transparent;
  }

  .placeholder {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--text-faint);
    font-size: 13px;
    padding: 0 24px;
    text-align: center;
  }
  .placeholder.err {
    color: var(--danger);
    word-break: break-all;
  }
</style>
