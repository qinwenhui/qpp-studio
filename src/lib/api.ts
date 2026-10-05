/** invoke 包装 + media:// URL 构建 + 平台判断。 */

import { invoke } from '@tauri-apps/api/core';
import type {
  DeviceBench,
  EngineStatus,
  HistoryEntry,
  HwInfo,
  ImageItem,
  InitInfo,
  ItemOutcome,
  OcrOutcome,
  PdfPage,
  Settings,
  ShotMonitor,
} from './types';

const isWindows = navigator.userAgent.includes('Windows');

/** 是否运行在 Tauri webview 里(普通浏览器预览时降级)。 */
export const hasTauri =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/**
 * Windows/WebView2 上自定义协议实际是 http://media.localhost/<token>，
 * macOS/Linux 是 media://localhost/<token>（wry 已知行为，CSP 两种都放行）。
 */
export function mediaUrl(token: string): string {
  return isWindows ? `http://media.localhost/${token}` : `media://localhost/${token}`;
}

export const api = {
  appInit: () => invoke<InitInfo>('app_init'),
  engineStatus: () => invoke<EngineStatus>('engine_status'),
  hwInfo: () => invoke<HwInfo>('hw_info'),
  deviceBenchmark: () => invoke<DeviceBench>('device_benchmark'),

  pickImages: () => invoke<ImageItem[]>('pick_images'),
  addFiles: (paths: string[]) => invoke<ImageItem[]>('add_files', { paths }),
  readClipboardImage: () => invoke<ImageItem>('read_clipboard_image'),

  ocrImage: (id: string, rotation = 0) =>
    invoke<OcrOutcome>('ocr_image', { id, rotation }),
  batchStart: (ids: string[]) => invoke<void>('batch_start', { ids }),
  batchCancel: () => invoke<void>('batch_cancel'),

  screenshotBegin: () => invoke<void>('screenshot_begin'),
  screenshotFinish: (mon: number, x: number, y: number, w: number, h: number) =>
    invoke<void>('screenshot_finish', { mon, x, y, w, h }),
  screenshotCancel: () => invoke<void>('screenshot_cancel'),
  shotWindowMonitor: (label: string) => invoke<ShotMonitor>('shot_window_monitor', { label }),
  shotResultData: () => invoke<ItemOutcome | null>('shot_result_data'),
  closeShotResult: () => invoke<void>('close_shot_result'),
  focusMain: () => invoke<void>('focus_main'),

  settingsGet: () => invoke<Settings>('settings_get'),
  settingsSet: (settings: Settings) => invoke<Settings>('settings_set', { settings }),

  historyList: (offset = 0, limit = 100) =>
    invoke<HistoryEntry[]>('history_list', { offset, limit }),
  historyDelete: (id: string) => invoke<void>('history_delete', { id }),
  historyClear: () => invoke<void>('history_clear'),
  historyReopen: (id: string) => invoke<ItemOutcome>('history_reopen', { id }),

  removeItem: (id: string) => invoke<void>('remove_item', { id }),
  removeItems: (ids: string[]) => invoke<void>('remove_items', { ids }),
  clearItems: () => invoke<void>('clear_items'),

  exportContent: (
    content: string,
    fmt: 'txt' | 'json' | 'md',
    defaultName: string,
  ) => invoke<string>('export_content', { content, fmt, defaultName }),
  copyText: (text: string) => invoke<void>('copy_text', { text }),
  revealPath: (path: string) => invoke<void>('reveal_path', { path }),
  openUrl: (url: string) => invoke<void>('open_url', { url }),

  pdfPageInfo: (id: string) =>
    invoke<[number, number, number, number] | null>('pdf_page_info', { id }),
  pdfPause: (id: string) => invoke<number | null>('pdf_pause', { id }),
  pdfResume: (id: string) => invoke<boolean>('pdf_resume', { id }),
  pdfRecognizePage: (id: string, page: number) =>
    invoke<void>('pdf_recognize_page', { id, page }),
  pdfPageOutcome: (id: string, page: number) =>
    invoke<OcrOutcome | null>('pdf_page_outcome', { id, page }),
  pdfSetMode: (id: string, extract: boolean) =>
    invoke<boolean>('pdf_set_mode', { id, extract }),
  pdfOcrPage: (id: string, page: number) => invoke<void>('pdf_ocr_page', { id, page }),
  pdfRenderPage: (id: string, page: number, dpi?: number) =>
    invoke<PdfPage>('pdf_render_page', { id, page, dpi }),
  pdfOcrRange: (id: string, startPage: number, endPage: number) =>
    invoke<void>('pdf_ocr_range', { id, startPage, endPage }),
  pdfExtractAll: (id: string) =>
    invoke<unknown[]>('pdf_extract_all', { id }),
  pdfExportMerged: (id: string, fmt: 'txt' | 'json' | 'md') =>
    invoke<string>('pdf_export_merged', { parentId: id, fmt }),
};
