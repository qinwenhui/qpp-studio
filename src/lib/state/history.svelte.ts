/** 历史记录状态。 */

import { api, mediaUrl } from '$lib/api';
import { addItemWithOutcome } from './images.svelte';
import { setView, toast } from './app.svelte';
import type { HistoryEntry } from '$lib/types';

export const historyStore = $state({
  entries: [] as HistoryEntry[],
});

export async function refreshHistory() {
  try {
    historyStore.entries = await api.historyList(0, 100);
  } catch (e) {
    toast('error', `读取历史失败: ${String(e)}`);
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
