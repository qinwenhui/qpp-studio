<script lang="ts">
  /** 空状态:欢迎引导。 */
  import { api } from '$lib/api';
  import { toast } from '$lib/state/app.svelte';
  import { addItems } from '$lib/state/images.svelte';
  import { settings } from '$lib/state/settings.svelte';
  import LogoMark from './LogoMark.svelte';
  import Icon from './Icon.svelte';

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
</script>

<div class="empty">
  <div class="mark"><LogoMark size={56} /></div>
  <h2>把图片放进来,文字立刻出来</h2>
  <p class="hint">本地推理,离线可用,数据不出机器</p>

  <div class="cta">
    <button class="btn primary" onclick={open}>
      <Icon name="folderOpen" size={15} />
      打开图片
    </button>
    <button class="btn" onclick={paste}>
      <Icon name="clipboard" size={15} />
      粘贴图片
    </button>
    <button class="btn" onclick={() => api.screenshotBegin()}>
      <Icon name="crop" size={15} />
      截图识别
    </button>
  </div>

  <p class="tip">也可以直接把图片拖进窗口 · 截图快捷键 {settings.hotkey}</p>
</div>

<style>
  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    animation: fade-in var(--speed-slow) var(--ease-out);
    z-index: 1;
  }
  .mark {
    display: grid;
    place-items: center;
    width: 96px;
    height: 96px;
    margin-bottom: 8px;
    border-radius: var(--radius-xl);
    background: color-mix(in srgb, var(--bg-elevated) 72%, transparent);
    border: 1px solid var(--border-subtle);
    box-shadow: var(--shadow-md);
    color: var(--text-primary);
  }
  h2 {
    font-size: 17px;
    font-weight: 650;
    color: var(--text-primary);
  }
  .hint {
    font-size: 12.5px;
    color: var(--text-faint);
  }
  .cta {
    display: flex;
    gap: 8px;
    margin-top: 14px;
  }
  .cta .btn {
    height: 34px;
    padding: 0 16px;
  }
  .tip {
    margin-top: 18px;
    font-size: 11.5px;
    color: var(--text-faint);
    font-family: var(--font-mono);
  }
</style>
