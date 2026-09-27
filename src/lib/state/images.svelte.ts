/** 图片条目与识别结果状态。 */

import { api, mediaUrl } from '$lib/api';
import { app, setView, toast } from './app.svelte';
import { canvasBus } from './canvasBus.svelte';
import type { ImageItem, ItemState, OcrOutcome } from '$lib/types';

export const imagesStore = $state({
  items: [] as ItemState[],
  activeId: '',
});

/** 当前激活条目(Svelte 5 禁止跨模块导出 $derived,用函数暴露)。 */
export function getActiveItem(): ItemState | undefined {
  return imagesStore.items.find((i) => i.item.id === imagesStore.activeId);
}

export function setActive(id: string) {
  imagesStore.activeId = id;
}

/** 入库并自动识别(单张直跑,多张批量)。新图立即成为当前查看项。
 * 「当前识别」始终只保留最新一批:精确移除旧条目(新批次刚在 Rust 侧注册,不能整表清空误伤)。 */
export async function addItems(list: ImageItem[]) {
  const wasImporting = app.importing !== null;
  app.importing = null;
  if (!list.length) {
    if (wasImporting) toast('info', '已取消上一次导入(被新的选择取代)');
    return;
  }
  if (!app.batchRunning && imagesStore.items.length > 0) {
    const oldIds = imagesStore.items.map((i) => i.item.id);
    imagesStore.items.length = 0;
    imagesStore.activeId = '';
    await api.removeItems(oldIds);
  }
  for (const item of list) {
    imagesStore.items.push({ item, phase: 'new' });
  }
  setActive(list[0].id);
  if (list.length > 1) {
    // 批量:切到识别记录看进度
    setView('records');
  }
  const ids = list.map((i) => i.id);
  await recognize(ids);
}

/** 批量结束后自动接续待识别的图片(批次进行中拖入的新图)。 */
export async function continuePending() {
  if (app.batchRunning) return;
  const pending = imagesStore.items
    .filter((i) => i.phase === 'new')
    .map((i) => i.item.id);
  if (pending.length) await recognize(pending);
}

/** 等引擎就绪(启动头几秒丢图进来时排队,而不是立刻报错)。 */
async function waitForEngine(timeoutMs = 60000): Promise<string | null> {
  const t0 = Date.now();
  while (!app.engineReady) {
    if (app.engineError) return app.engineError;
    if (Date.now() - t0 > timeoutMs) return '等待引擎就绪超时,请重启应用或检查模型目录';
    await new Promise((r) => setTimeout(r, 400));
  }
  return null;
}

async function recognize(ids: string[]) {
  if (app.batchRunning) {
    // 批次进行中:新图留在待识别,batch://done 后自动接续
    toast('info', '当前批次进行中,新图片将在其完成后自动识别');
    return;
  }
  for (const id of ids) {
    const st = imagesStore.items.find((i) => i.item.id === id);
    if (st) st.phase = 'queued';
  }
  const engineErr = await waitForEngine();
  if (engineErr) {
    toast('error', `引擎未就绪:${engineErr}`);
    for (const id of ids) {
      const st = imagesStore.items.find((i) => i.item.id === id);
      if (st) st.phase = 'error';
    }
    return;
  }
  for (const id of ids) {
    const st = imagesStore.items.find((i) => i.item.id === id);
    if (st) st.phase = 'queued';
  }
  try {
    if (ids.length === 1) {
      await api.ocrImage(ids[0]);
    } else {
      app.batchRunning = true;
      app.batchStartedAt = Date.now();
      await api.batchStart(ids);
    }
  } catch (e) {
    toast('error', String(e));
    for (const id of ids) {
      const st = imagesStore.items.find((i) => i.item.id === id);
      if (st && st.phase === 'queued') st.phase = 'error';
    }
    app.batchRunning = false;
  }
}

/** 事件:单图完成(单张/批量/截图共用)。thumbToken 为识别搭车生成的缩略图令牌。 */
export function applyOutcome(id: string, outcome: OcrOutcome, thumbToken?: string) {
  const st = imagesStore.items.find((i) => i.item.id === id);
  if (st) {
    st.phase = outcome.ok ? 'done' : 'error';
    st.outcome = outcome;
    if (thumbToken) st.item.thumbToken = thumbToken;
  }
}

/** 事件:批量状态推进。 */
export function applyStatus(id: string, phase: string) {
  const st = imagesStore.items.find((i) => i.item.id === id);
  if (st && (phase === 'running' || phase === 'queued')) {
    st.phase = phase;
  }
}

export async function removeItem(id: string) {
  const i = imagesStore.items.findIndex((x) => x.item.id === id);
  if (i >= 0) imagesStore.items.splice(i, 1);
  if (imagesStore.activeId === id) {
    imagesStore.activeId = imagesStore.items[Math.min(i, imagesStore.items.length - 1)]?.item.id ?? '';
  }
  await api.removeItem(id);
}

export async function clearAll() {
  imagesStore.items.length = 0;
  imagesStore.activeId = '';
  await api.clearItems();
}

/** 历史重开/截图完成注入既有结果。
 * `replace` = 遵循「当前识别 = 最新一批」语义:空闲时替换整个队列(截图、历史重开);
 * 批量进行中则追加,不打扰当前批次。 */
export function addItemWithOutcome(
  item: ImageItem,
  outcome: OcrOutcome,
  activate = true,
  replace = true,
) {
  const existing = imagesStore.items.find((i) => i.item.id === item.id);
  if (existing) {
    existing.outcome = outcome;
    existing.phase = outcome.ok ? 'done' : 'error';
  } else {
    if (replace && !app.batchRunning && imagesStore.items.length > 0) {
      const oldIds = imagesStore.items.map((i) => i.item.id);
      imagesStore.items.length = 0;
      imagesStore.activeId = '';
      void api.removeItems(oldIds);
    }
    imagesStore.items.push({ item, phase: outcome.ok ? 'done' : 'error', outcome });
  }
  if (activate) setActive(item.id);
}

/** 重新识别单张:队列保持不动,只重跑这一张,结果/耗时/统计/历史原地更新。
 * `rotation` 非零时烘焙进引擎输入(转正后重新识别),框坐标自动逆变换。
 * 引擎重建中(刚切模型)先排队等待,不瞬时报错。 */
export async function reRecognize(id: string) {
  const st = imagesStore.items.find((i) => i.item.id === id);
  if (!st) return;
  st.phase = 'running';
  try {
    const engineErr = await waitForEngine();
    if (engineErr) {
      st.phase = 'error';
      st.outcome = { ok: false, error: engineErr };
      return;
    }
    const rotation = canvasBus.getRotation?.() ?? 0;
    await api.ocrImage(id, rotation);
  } catch (e) {
    toast('error', String(e));
    st.phase = 'error';
  }
}

export function mediaSrc(item: ImageItem): string {
  return mediaUrl(item.mediaToken);
}
