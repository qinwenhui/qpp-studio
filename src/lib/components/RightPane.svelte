<script lang="ts">
  /** 右侧内容面板:纸张 ↔ 列表 双模式(开关内嵌在各视图工具条里)。
   *  无识别结果时整块隐藏(主界面只有一个内容面板,识别完成才展开双栏)。 */
  import { app } from '$lib/state/app.svelte';
  import { getActiveItem } from '$lib/state/images.svelte';
  import PaperStage from '$lib/components/PaperStage.svelte';
  import ResultsPanel from '$lib/components/Panels/ResultsPanel.svelte';

  const active = $derived(getActiveItem());
  /** 有结果(成功或失败)才显示面板 */
  const has = $derived(!!active?.outcome);
</script>

<div class="pane" class:hidden={!has}>
  {#if app.paneMode === 'paper'}
    <PaperStage />
  {:else}
    <ResultsPanel />
  {/if}
</div>

<style>
  .pane {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    border-left: 1px solid var(--border-subtle);
    background: var(--bg-panel);
  }
  .pane.hidden {
    display: none;
  }
</style>
