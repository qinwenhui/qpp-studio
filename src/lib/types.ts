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
  /** 文本层直提(非 OCR):框线是全宽近似,画布不显示;UI 标注来源 */
  extracted?: boolean;
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
  /** 实际计算设备:cpu | gpu */
  device: string;
  modelsDir: string;
}

export interface Settings {
  theme: string;
  tier: string;
  preset: string;
  /** 计算设备:cpu | gpu */
  device: string;
  threads: number;
  hotkey: string;
  modelsDir?: string;
  batchConcurrency: number;
  /** worker 进程数手动覆盖,0=自动(硬件优化表) */
  workersOverride: number;
  /** 方向纠正(默认开) */
  orientation: boolean;
  /** 增强对比(默认关) */
  enhanceContrast: boolean;
  /** 检测放大倍数(默认 1,水印/小字图用 2) */
  upscale: number;
  /** 截图时隐藏主窗口(工具栏截图按钮右键选择;默认不隐藏) */
  shotHide: boolean;
}

/** 硬件检测出的并行策略(按当前 tier + workersOverride 实时计算) */
export interface ParallelPlan {
  workers: number;
  threadsEach: number;
  inprocConcurrency: number;
  /** 内存闸允许的最大 worker 数(u32::MAX = 内存未知不设限) */
  memCap: number;
  clampedByMem: boolean;
}

/** 单块 GPU 检测信息 */
export interface GpuInfo {
  name: string;
  /** API 版本串,如 "vulkan 1.4" */
  api: string;
}

/** 硬件检测信息(hw_info 命令) */
export interface HwInfo {
  cpuBrand: string;
  physicalCores: number;
  logicalCores: number;
  /** 总内存 GB;0 = 探测失败 */
  totalMemGb: number;
  /** GPU 清单;空 = 无可用 Vulkan 设备 */
  gpus: GpuInfo[];
  plan: ParallelPlan;
}

/** 设备实测对比结果(device_benchmark 命令) */
export interface DeviceBench {
  cpuMs: number;
  gpuMs: number;
  cpuLines: number;
  gpuLines: number;
  /** GPU 失败时的引擎报错(非空时 gpuMs/gpuLines 为 0) */
  gpuError?: string;
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
  /** 该页是否已有识别结果(按需模式据此触发单页识别) */
  recognized: boolean;
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
  /** PDF 专属:总页数、当前显示页、识别进度/模式(非 PDF 为 null) */
  pdfPages?: {
    count: number;
    current: number;
    ocrDone: number;
    /** 按需模式:翻到哪页识别哪页(大文档默认) */
    onDemand?: boolean;
    /** 文本层直提模式(整册;可切换) */
    extract?: boolean;
    /** 识别状态机:idle(按需未跑)| running | paused | done */
    ocrState?: 'idle' | 'running' | 'paused' | 'done';
  };
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
