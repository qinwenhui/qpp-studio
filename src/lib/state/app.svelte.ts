/** 全局应用状态:视图、toast、引擎状态、拖拽、画布联动。 */

import type { ToastMsg } from '$lib/types';

export type View = 'records' | 'stats' | 'settings';

/** 右侧内容面板模式:纸张(按位置重排)| 列表(阅读序文本行) */
export type PaneMode = 'paper' | 'list';

export const app = $state({
  /** 右侧抽屉当前视图 */
  view: 'records' as View,
  toasts: [] as ToastMsg[],

  /** 引擎 */
  engineReady: false,
  engineError: '',
  engineTier: 'tiny',
  enginePreset: 'balanced',
  engineThreads: 0,
  engineDevice: 'cpu',
  engineModelsDir: '',
  version: '',
  platform: 'windows',

  /** 拖拽导入 */
  dropActive: false,

  /** 批量导入进度(解码建缩略图,大批量需要数秒) */
  importing: null as { done: number; total: number } | null,

  /** 画布:识别框显示开关(UI 偏好,localStorage 持久化) */
  showBoxes: localStorage.getItem('qpp.showBoxes') !== '0',

  /** hover 联动:文本行 ↔ 识别框 */
  focus: null as { id: string; line: number } | null,
  /** 点击选中(比 hover 更强的高亮) */
  selected: null as { id: string; line: number } | null,

  /** 批量进行中 */
  batchRunning: false,
  /** 本轮批量开始/结束时刻(unix ms) */
  batchStartedAt: 0,
  batchEndedAt: 0,
  /** 已就绪(初始化完成) */
  booted: false,

  /** 遮罩弹窗:当前打开的弹窗(null = 无) */
  modal: null as 'paper' | null,

  /** 右侧功能抽屉(记录/统计/设置)是否展开 */
  drawer: false,

  /** 右侧内容面板模式(纸张/列表),默认列表 */
  paneMode: 'list' as PaneMode,

  /** 纸张「原向」模式:按原始角度/方向渲染(忠于版式核对用) */
  fidelity: localStorage.getItem('qpp.fidelity') === '1',
});

let toastSeq = 1;

export function toast(level: ToastMsg['level'], message: string) {
  const id = toastSeq++;
  app.toasts.push({ id, level, message });
  setTimeout(() => {
    const i = app.toasts.findIndex((t) => t.id === id);
    if (i >= 0) app.toasts.splice(i, 1);
  }, level === 'error' ? 4200 : 2400);
}

export function setShowBoxes(on: boolean) {
  app.showBoxes = on;
  localStorage.setItem('qpp.showBoxes', on ? '1' : '0');
}

export function setFidelity(on: boolean) {
  app.fidelity = on;
  localStorage.setItem('qpp.fidelity', on ? '1' : '0');
}

export function setView(view: View) {
  app.view = view;
  app.drawer = true;
  if (view === 'records') {
    void import('$lib/state/history.svelte').then((m) => m.refreshHistory());
  }
}

/** 抽屉开关:点当前视图的图标 = 收起,点其他 = 切换并展开。 */
export function toggleView(view: View) {
  if (app.drawer && app.view === view) {
    app.drawer = false;
  } else {
    setView(view);
  }
}
