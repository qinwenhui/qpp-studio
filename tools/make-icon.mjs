// QPP Studio 应用图标生成器 — 零依赖(仅 Node 内置 zlib)。
// 设计:深色圆角方底 + 青绿渐变放大镜 + 三条"文本行" + 识别星光。
// SDF 光栅化 + 3x 超采样抗锯齿。输出 tools/app-icon.png (1024×1024 RGBA)。
//
// 用法: node tools/make-icon.mjs

import { deflateSync, crc32 } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const SIZE = 1024;
const SS = 3; // 每轴超采样次数

// ---------- 调色 ----------
const lerp = (a, b, t) => a + (b - a) * t;
const mix = (c1, c2, t) => [
  lerp(c1[0], c2[0], t),
  lerp(c1[1], c2[1], t),
  lerp(c1[2], c2[2], t),
];
// 对角线性渐变
const gradient = (c1, c2) => (x, y) => mix(c1, c2, Math.min(1, Math.max(0, (x + y) / 2 / SIZE)));

const BG_TOP = [15, 20, 27]; // #0f141b
const BG_BOT = [27, 36, 48]; // #1b2430
const ACCENT_A = [52, 211, 153]; // #34d399
const ACCENT_B = [34, 211, 238]; // #22d3ee
const BAR_DIM = [148, 163, 184]; // slate-400
const BAR_HOT = [241, 245, 249]; // 近白

// ---------- SDF ----------
const sdRoundRect = (px, py, cx, cy, hw, hh, r) => {
  const qx = Math.abs(px - cx) - (hw - r);
  const qy = Math.abs(py - cy) - (hh - r);
  const ox = Math.max(qx, 0);
  const oy = Math.max(qy, 0);
  return Math.hypot(ox, oy) + Math.min(Math.max(qx, qy), 0) - r;
};
const sdRing = (px, py, cx, cy, R, t) => Math.abs(Math.hypot(px - cx, py - cy) - R) - t / 2;
const sdCapsule = (px, py, ax, ay, bx, by, r) => {
  const pax = px - ax,
    pay = py - ay,
    bax = bx - ax,
    bay = by - ay;
  const h = Math.min(1, Math.max(0, (pax * bax + pay * bay) / (bax * bax + bay * bay)));
  return Math.hypot(pax - bax * h, pay - bay * h) - r;
};

// ---------- 图层 ----------
const LENS = { cx: 448, cy: 528, R: 235, stroke: 52 };
// 镜柄:圆心向 45° 右下
const handleAngle = Math.PI / 4;
const HX1 = LENS.cx + Math.cos(handleAngle) * (LENS.R - 18);
const HY1 = LENS.cy + Math.sin(handleAngle) * (LENS.R - 18);
const HX2 = LENS.cx + Math.cos(handleAngle) * (LENS.R + 208);
const HY2 = LENS.cy + Math.sin(handleAngle) * (LENS.R + 208);
const HANDLE_R = LENS.stroke / 2 + 6;

// 镜内三条"文本行"
const BARS = [
  { x: LENS.cx - 118, y: LENS.cy - 78, w: 226, h: 17, hot: true },
  { x: LENS.cx - 118, y: LENS.cy - 8, w: 168, h: 17, hot: false },
  { x: LENS.cx - 118, y: LENS.cy + 62, w: 196, h: 17, hot: false },
];

// 星光(右上)
const SPARK = { cx: 772, cy: 286, rays: 62, dot: 15, width: 11 };

const AA = () => 1.0; // 像素单位抗锯齿宽度
const smoothstep = (a, b, x) => {
  const t = Math.min(1, Math.max(0, (x - a) / (b - a || 1e-9)));
  return t * t * (3 - 2 * t);
};

// ---------- 渲染 ----------
function render() {
  const data = Buffer.alloc(SIZE * SIZE * 4);
  for (let py = 0; py < SIZE; py++) {
    for (let px = 0; px < SIZE; px++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const x = px + (sx + 0.5) / SS;
          const y = py + (sy + 0.5) / SS;
          const s = sdfAt2(x, y);
          r += s.color[0];
          g += s.color[1];
          b += s.color[2];
          a += s.alpha;
        }
      }
      const n = SS * SS;
      const i = (py * SIZE + px) * 4;
      data[i] = Math.round(r / n);
      data[i + 1] = Math.round(g / n);
      data[i + 2] = Math.round(b / n);
      data[i + 3] = Math.round((a / n) * 255);
    }
  }
  return data;
}

// sdfAt 的可重入版本(避免闭包污染)
function sdfAt2(x, y) {
  let c = gradient(BG_TOP, BG_BOT)(x, y);
  let al = 1 - smoothstep(0, AA(), sdRoundRect(x, y, SIZE / 2, SIZE / 2, SIZE / 2 - 8, SIZE / 2 - 8, 232));
  const lay = (top, cov) => {
    if (cov <= 0) return;
    c = [
      c[0] * (1 - cov) + top[0] * cov,
      c[1] * (1 - cov) + top[1] * cov,
      c[2] * (1 - cov) + top[2] * cov,
    ];
    al = al + (1 - al) * cov;
  };

  const sparkD = Math.min(
    sdCapsule(x, y, SPARK.cx, SPARK.cy - SPARK.rays, SPARK.cx, SPARK.cy + SPARK.rays, SPARK.width),
    sdCapsule(x, y, SPARK.cx - SPARK.rays, SPARK.cy, SPARK.cx + SPARK.rays, SPARK.cy, SPARK.width),
  );
  lay([255, 255, 255], 1 - smoothstep(0, AA(), sparkD));

  const acc = gradient(ACCENT_A, ACCENT_B)(x, y);
  const lensD = Math.min(
    sdRing(x, y, LENS.cx, LENS.cy, LENS.R, LENS.stroke),
    sdCapsule(x, y, HX1, HY1, HX2, HY2, HANDLE_R),
  );
  lay(acc, 1 - smoothstep(0, AA(), lensD));

  for (const bar of BARS) {
    const d = sdRoundRect(x, y, bar.x + bar.w / 2, bar.y, bar.w / 2, bar.h, bar.h);
    lay(bar.hot ? BAR_HOT : BAR_DIM, 1 - smoothstep(0, AA(), d));
  }
  return { color: c, alpha: al };
}

// ---------- PNG 编码 ----------
function chunk(type, buf) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(buf.length);
  const typeBuf = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuf, buf])) >>> 0);
  return Buffer.concat([len, typeBuf, buf, crc]);
}

function encodePng(width, height, rgba) {
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  // 每行前加 filter 0
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0;
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  const idat = deflateSync(raw, { level: 9 });
  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", idat),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---------- main ----------
const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, "app-icon.png");
const rgba = render();
mkdirSync(here, { recursive: true });
writeFileSync(out, encodePng(SIZE, SIZE, rgba));
console.log(`✓ 图标已生成: ${out} (${SIZE}×${SIZE})`);
