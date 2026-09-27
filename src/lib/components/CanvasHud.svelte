<script lang="ts">
  /** 图片画布的浮动控制条:识别框开关 / 缩放 / 旋转。悬浮于画面右上角。 */
  import Icon from '$lib/components/Icon.svelte';
  import { app, setShowBoxes } from '$lib/state/app.svelte';
  import { canvasBus } from '$lib/state/canvasBus.svelte';
</script>

<div class="hud" role="toolbar" aria-label="画布视图控制">
  <button
    class="hud-btn"
    class:active={app.showBoxes}
    onclick={() => setShowBoxes(!app.showBoxes)}
    title={app.showBoxes ? '隐藏识别框' : '显示识别框'}
  >
    <Icon name={app.showBoxes ? 'eye' : 'eyeOff'} size={15} />
  </button>
  <span class="hud-sep"></span>
  <button class="hud-btn" onclick={() => canvasBus.rotateCCW?.()} title="向左旋转 90°">
    <Icon name="rotateCcw" size={15} />
  </button>
  <button class="hud-btn" onclick={() => canvasBus.rotateCW?.()} title="向右旋转 90°">
    <Icon name="rotateCw" size={15} />
  </button>
  <span class="hud-sep"></span>
  <button class="hud-btn" onclick={() => canvasBus.oneToOne?.()} title="原始尺寸 1:1">
    <Icon name="ratioOneToOne" size={14} />
  </button>
  <button class="hud-btn" onclick={() => canvasBus.zoomOut?.()} title="缩小">
    <Icon name="zoomOut" size={15} />
  </button>
  <button class="hud-btn" onclick={() => canvasBus.zoomIn?.()} title="放大">
    <Icon name="zoomIn" size={15} />
  </button>
  <button class="hud-btn" onclick={() => canvasBus.fit?.()} title="适应窗口">
    <Icon name="fit" size={15} />
  </button>
</div>

<style>
  .hud {
    position: absolute;
    top: 10px;
    right: 10px;
    z-index: 10;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--bg-elevated) 82%, transparent);
    backdrop-filter: blur(10px);
    border: 1px solid var(--border-subtle);
    box-shadow: var(--shadow-md);
    animation: rise-in var(--speed) var(--ease-out);
  }
  .hud-btn {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    transition: all var(--speed-fast) var(--ease-out);
  }
  .hud-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }
  .hud-btn.active {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .hud-sep {
    width: 1px;
    height: 16px;
    margin: 0 3px;
    background: var(--border-subtle);
  }
</style>
