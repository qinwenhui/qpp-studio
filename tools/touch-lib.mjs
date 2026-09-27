// 强制 lib crate 重编:generate_context! 在 lib.rs 编译期嵌入 dist,
// vite 只改 dist 内容时 cargo 指纹不一定失效 → 旧前端被带进 release。
// beforeBuildCommand 里在 vite build 之后 touch 一下 lib.rs。
import { utimesSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const now = new Date();
utimesSync(join(here, "..", "src-tauri", "src", "lib.rs"), now, now);
console.log("✓ lib.rs touched(强制重新嵌入前端产物)");
