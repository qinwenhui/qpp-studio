<script lang="ts">
  /** 轻提示栈。 */
  import Icon from './Icon.svelte';
  import { app } from '$lib/state/app.svelte';
</script>

<div class="toasts">
  {#each app.toasts as t (t.id)}
    <div class="toast {t.level}" role="status">
      <Icon
        name={t.level === 'error' ? 'info' : t.level === 'success' ? 'check' : 'sparkleSmall'}
        size={14}
      />
      <span>{t.message}</span>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: absolute;
    left: 50%;
    bottom: 18px;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    z-index: 100;
    pointer-events: none;
  }
  .toast {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    max-width: 480px;
    padding: 8px 14px;
    border-radius: 999px;
    font-size: 12.5px;
    color: var(--text-primary);
    background: color-mix(in srgb, var(--bg-elevated) 92%, transparent);
    backdrop-filter: blur(10px);
    border: 1px solid var(--border-subtle);
    box-shadow: var(--shadow-md);
    animation: rise-in var(--speed) var(--ease-spring);
  }
  .toast.success {
    color: var(--success);
  }
  .toast.error {
    color: var(--danger);
  }
</style>
