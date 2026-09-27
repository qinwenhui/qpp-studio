<script lang="ts">
  /** 通用遮罩弹窗:暗背景 + 居中内容;ESC / 点背景 / × 关闭。 */
  import Icon from '$lib/components/Icon.svelte';
  import { app } from '$lib/state/app.svelte';
  import type { Snippet } from 'svelte';

  let {
    title,
    width = '86vw',
    height = '88vh',
    children,
  }: {
    title: string;
    width?: string;
    height?: string;
    children: Snippet;
  } = $props();

  function close() {
    app.modal = null;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      close();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="backdrop" onclick={close} onkeydown={() => {}} role="presentation">
  <div
    class="dialog"
    class:dusk={app.modal === null}
    style="width:{width}; height:{height}"
    onclick={(e) => e.stopPropagation()}
    onkeydown={() => {}}
    role="dialog"
    aria-modal="true"
    aria-label={title}
  >
    <header class="head">
      <span class="title">{title}</span>
      <button class="icon-btn close" onclick={close} title="关闭 (Esc)">
        <Icon name="x" size={15} />
      </button>
    </header>
    <div class="body">
      {@render children()}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: absolute;
    inset: 0;
    z-index: 200;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.5);
    backdrop-filter: blur(4px);
    animation: fade-in var(--speed-fast) var(--ease-out);
  }
  .dialog {
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-lg);
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    animation: rise-in var(--speed) var(--ease-spring);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px 10px 16px;
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
  .body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
</style>
