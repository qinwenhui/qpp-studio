/** 画布命令总线:ToolBar → CanvasStage(注册方为空时安全跳过)。 */

export interface CanvasBus {
  fit: (() => void) | null;
  zoomIn: (() => void) | null;
  zoomOut: (() => void) | null;
  oneToOne: (() => void) | null;
  rotateCW: (() => void) | null;
  rotateCCW: (() => void) | null;
  /** 当前显示旋转角度(重新识别时烘焙进引擎输入) */
  getRotation: (() => number) | null;
}

export const canvasBus: CanvasBus = {
  fit: null,
  zoomIn: null,
  zoomOut: null,
  oneToOne: null,
  rotateCW: null,
  rotateCCW: null,
  getRotation: null,
};
