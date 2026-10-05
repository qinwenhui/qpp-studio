/** 设置状态:加载/更新/副作用(主题即时应用)。 */

import { api } from '$lib/api';
import { applyTheme } from '$lib/theme';
import { app, toast } from './app.svelte';
import type { Settings } from '$lib/types';

const DEFAULTS: Settings = {
  theme: 'macos-glass',
  tier: 'tiny',
  preset: 'speed',
  device: 'cpu',
  threads: 0,
  hotkey: 'ctrl+shift+o',
  modelsDir: undefined,
  batchConcurrency: 0,
  workersOverride: 0,
  orientation: true,
  enhanceContrast: false,
  upscale: 1,
  shotHide: false,
};

export const settings = $state<Settings>({ ...DEFAULTS });

export function applySettings(s: Settings) {
  Object.assign(settings, s);
  applyTheme(settings.theme);
}

export async function loadSettings() {
  try {
    const info = await api.appInit();
    applySettings(info.settings);
    app.version = info.version;
    app.platform = info.platform;
    syncEngine(info.engine);
  } catch (e) {
    toast('error', `初始化失败: ${String(e)}`);
  }
}

export function syncEngine(e: {
  ready: boolean;
  error?: string;
  tier: string;
  preset: string;
  threads: number;
  device?: string;
  modelsDir: string;
}) {
  app.engineReady = e.ready;
  app.engineError = e.error ?? '';
  app.engineTier = e.tier;
  app.enginePreset = e.preset;
  app.engineThreads = e.threads;
  app.engineDevice = e.device ?? 'cpu';
  app.engineModelsDir = e.modelsDir;
}

/** 局部更新;失败时回滚为后端当前值。 */
export async function updateSettings(patch: Partial<Settings>) {
  const next = { ...settings, ...patch };
  try {
    const saved = await api.settingsSet(next);
    Object.assign(settings, saved);
    applyTheme(settings.theme);
  } catch (e) {
    toast('error', String(e));
    // 回滚:重新拉取后端真实状态
    try {
      Object.assign(settings, await api.settingsGet());
      applyTheme(settings.theme);
    } catch {
      /* 忽略 */
    }
  }
}
