// 从 qppocr 仓库填充 src-tauri/models-bundle/models/（打包资源）。
// 关键一步：把 dict_small_medium.txt 复制为根 dict.txt —— 引擎字典查找顺序
// {tier}/dict.txt → models/dict.txt → ppocr_keys.txt → 内嵌，medium 档没有
// 自己的字典，必须靠根 dict.txt 兜底。
//
// 模型来源(按优先级):
//   1. 并排的 qppocr 仓库(本地开发:../qppocr/models)
//   2. QPPOCR_MODELS_REPO 浅克隆(CI:如 https://github.com/qinwenhui/qppocr.git)
//
// 默认三档全装（tiny+small+medium+cls）；--slim 只装 tiny+small（安装包省 ~100MB）。
//
// 用法: node tools/stage-models.mjs [--slim]

import { copyFileSync, mkdirSync, existsSync, statSync, rmSync } from "node:fs";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const dest = join(here, "..", "src-tauri", "models-bundle", "models");
const slim = process.argv.includes("--slim");

// 定位模型源目录;本地没有则按 QPPOCR_MODELS_REPO 浅克隆(CI 路径)
let src = join(here, "..", "..", "qppocr", "models");
if (!existsSync(src)) {
  const repo = process.env.QPPOCR_MODELS_REPO ?? "https://github.com/qinwenhui/qppocr.git";
  const clone = join(here, ".qppocr-clone");
  rmSync(clone, { recursive: true, force: true });
  console.log(`本地无并排仓库,浅克隆模型源: ${repo}`);
  execSync(
    `git clone --depth 1 --filter=blob:none --sparse "${repo}" "${clone}"`,
    { stdio: "inherit" },
  );
  execSync(`git -C "${clone}" sparse-checkout set models`, { stdio: "inherit" });
  src = join(clone, "models");
}

if (!existsSync(src)) {
  console.warn(`⚠ 找不到引擎模型目录: ${src}`);
  console.warn("  → 跳过模型装配,产出不含模型的安装包(用户自行放置 models/)");
  console.warn("  本地开发:把 qppocr 仓库与本项目并排放置,或在 QPPOCR_MODELS_REPO 指向含模型的仓库");
  mkdirSync(dest, { recursive: true });
  process.exit(0);
}

const copy = (from, to) => {
  mkdirSync(dirname(to), { recursive: true });
  copyFileSync(from, to);
  const mb = (statSync(to).size / 1048576).toFixed(1);
  console.log(`  ✓ ${to.replace(dest, "models")}  (${mb} MB)`);
};

console.log(`staging: ${src} → ${dest}`);
for (const tier of ["tiny", "small", ...(slim ? [] : ["medium"])]) {
  copy(join(src, tier, "det.onnx"), join(dest, tier, "det.onnx"));
  copy(join(src, tier, "rec.onnx"), join(dest, tier, "rec.onnx"));
}
// 字典一律用验证过行数的档内文件:tiny/dict.txt(6904 行)、small/dict.txt(18708 行)。
// ⚠ 引擎仓库根目录的 dict_small_medium.txt 实测少一行(18707),会导致
//   medium 档 rec 字符表校验失败(类别数 18710 ≠ 18709),绝不能用它兜底。
copy(join(src, "tiny", "dict.txt"), join(dest, "tiny", "dict.txt"));
copy(join(src, "small", "dict.txt"), join(dest, "small", "dict.txt"));
if (!slim) {
  copy(join(src, "small", "dict.txt"), join(dest, "medium", "dict.txt"));
}
copy(join(src, "small", "dict.txt"), join(dest, "dict.txt"));
copy(join(src, "cls.onnx"), join(dest, "cls.onnx"));

if (slim) {
  console.log("  ✓ slim 模式:medium/ 不打包");
}
console.log("✓ staging 完成");
