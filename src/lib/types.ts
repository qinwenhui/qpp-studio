/** 镜像 src-tauri/src/dto.rs 的 camelCase DTO。 */

/** 单字符坐标:四角点 TL/TR/BR/BL,原图坐标 */
export interface CharSpan {
  text: string;
  pts: [number, number][];
}

export interface TextLine {
  text: string;
  confidence: number;
  rotation: number;
  pts: [number, number][];
  /** 逐字坐标(与 text 逐字符对齐,空格含真实空白区间框) */
  chars?: CharSpan[];
  /** 本行来自区域重试的第二遍(≠ 必然更好,低置信时需更强警示) */
  retried?: boolean;
}

export interface Timings {
  detPreMs: number;
  detInferMs: number;
  detPostMs: number;
  cropMs: number;
  clsMs: number;
  recPreMs: number;
  recInferMs: number;
  recPostMs: number;
  totalMs: number;
}

export interface OcrResult {
  lines: TextLine[];
  workW: number;
  workH: number;
  numBoxes: number;
  numMerged: number;
  numDecluttered: number;
  numDetRetried: number;
  numFlipped: number;
  numUnread: number;
  timings: Timings;
}

export interface OcrOutcome {
  ok: boolean;
  error?: string;
  result?: OcrResult;
}

export interface ImageItem {
  id: string;
  name: string;
  path: string;
  w: number;
  h: number;
  origin: 'file' | 'clipboard' | 'screenshot' | 'pdf';
  addedAt: number;
  mediaToken: string;
  thumbToken: string;
  /** PDF 有文本层时可直提 */
  canExtract?: boolean;
}

export interface ItemOutcome {
  item: ImageItem;
  outcome: OcrOutcome;
}

export interface EngineStatus {
  ready: boolean;
  error?: string;
  tier: string;
  preset: string;
  threads: number;
  modelsDir: string;
}

export interface Settings {
  theme: string;
  tier: string;
  preset: string;
  threads: number;
  hotkey: string;
  modelsDir?: string;
  batchConcurrency: number;
  /** 方向纠正(默认开) */
  orientation: boolean;
  /** 增强对比(默认关) */
  enhanceContrast: boolean;
  /** 检测放大倍数(默认 1,水印/小字图用 2) */
  upscale: number;
  /** 截图时隐藏主窗口(工具栏截图按钮右键选择;默认不隐藏) */
  shotHide: boolean;
}

/** PDF 页面信息 */
export interface PdfPageInfo {
  pageCount: number;
  currentPage: number;
}

/** PDF 页面渲染结果 */
export interface PdfPage {
  mediaToken: string;
  w: number;
  h: number;
  page: number;
}

export interface InitInfo {
  settings: Settings;
  engine: EngineStatus;
  version: string;
  platform: string;
}

export interface HistoryEntry {
  id: string;
  name: string;
  path: string;
  w: number;
  h: number;
  origin: string;
  at: number;
  lineCount: number;
  totalMs: number;
  textPreview: string;
  outcome: OcrOutcome;
  thumbToken?: string;
}

export interface ShotMonitor {
  url: string;
  w: number;
  h: number;
  dpr: number;
}

export interface ToastMsg {
  id: number;
  level: 'info' | 'success' | 'error';
  message: string;
}

export type Phase = 'new' | 'queued' | 'running' | 'done' | 'error';

export interface ItemState {
  item: ImageItem;
  phase: Phase;
  outcome?: OcrOutcome;
  /** PDF 专属:总页数、当前显示页、后台识别进度(已识别页数,非 PDF 为 null) */
  pdfPages?: { count: number; current: number; ocrDone?: number };
  canExtract?: boolean;
}

/** PDF 每页识别完成事件 */
export interface PdfPageDone {
  id: string;
  page: number;
  outcome: OcrOutcome;
  done: number;
  total: number;
}
