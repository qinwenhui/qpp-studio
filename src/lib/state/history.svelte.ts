/** 历史记录状态。 */

import { api, mediaUrl } from '$lib/api';
import { addItemWithOutcome } from './images.svelte';
import { setView, toast } from './app.svelte';
import type { HistoryEntry } from '$lib/types';

export const historyStore = $state({
  entries: [] as HistoryEntry[],
});

let inFlight = false;
let dirty = false;
let timer: ReturnType<typeof setTimeout> | undefined;

export async function refreshHistory() {
  // 上一次还没回来就再拉一遍没意义,标脏等它结束后补一次
  if (inFlight) {
    dirty = true;
    return;
  }
  inFlight = true;
  try {
    const entries = await api.historyList(0, 100);
    historyStore.entries = entries;
    // 缺缩略图的条目(源文件仍在时)按需补生成,不阻塞列表渲染。
    // 补不出来的记进 misses,避免每次刷新都对已删源文件重复发起 IPC。
    for (const e of entries) {
      if (!e.thumbToken && !thumbMisses.has(e.id)) void fetchThumb(e.id);
    }
  } catch (e) {
    toast('error', `读取历史失败: ${String(e)}`);
  } finally {
    inFlight = false;
    if (dirty) {
      dirty = false;
      void refreshHistory();
    }
  }
}

/**
 * 合并式刷新:批量识别时 item-done 会连着来上百条,每条都全量拉列表会把
 * IPC 与渲染打满。这里把一阵突发合并成一次刷新;`batch://done` 再兜一次终态。
 */
export function scheduleHistoryRefresh(delay = 250) {
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => {
    timer = undefined;
    void refreshHistory();
  }, delay);
}

/** 补不出缩略图的条目(源文件已不在),不再重复尝试。 */
const thumbMisses = new Set<string>();

/** 为缺图的历史条目补生成缩略图(后端会落盘,后续列表直接命中)。 */
async function fetchThumb(id: string) {
  try {
    const token = await api.historyThumb(id);
    if (!token) {
      thumbMisses.add(id);
      return;
    }
    const hit = historyStore.entries.find((e) => e.id === id);
    if (hit) hit.thumbToken = token;
  } catch {
    /* 源文件已删除等:保持占位图标,不打扰用户 */
    thumbMisses.add(id);
  }
}

export async function deleteHistory(id: string) {
  const i = historyStore.entries.findIndex((e) => e.id === id);
  if (i >= 0) historyStore.entries.splice(i, 1);
  await api.historyDelete(id);
}

export async function clearHistory() {
  historyStore.entries.length = 0;
  await api.historyClear();
}

export async function reopenHistory(id: string) {
  try {
    const { item, outcome } = await api.historyReopen(id);
    addItemWithOutcome(item, outcome);
  } catch (e) {
    toast('error', String(e));
  }
}

export function thumbSrc(entry: HistoryEntry): string | undefined {
  return entry.thumbToken ? mediaUrl(entry.thumbToken) : undefined;
}

export function timeAgo(ts: number): string {
  const s = Math.max(0, (Date.now() - ts) / 1000);
  if (s < 60) return '刚刚';
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  return `${Math.floor(s / 86400)} 天前`;
}
