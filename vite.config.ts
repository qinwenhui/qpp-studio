import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath, URL } from "node:url";

// 多页入口：主窗口 / 截图覆盖层 / 截图结果弹窗
const r = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  plugins: [svelte()],
  resolve: {
    alias: {
      $lib: r("./src/lib"),
      $assets: r("./src/assets"),
    },
  },
  // Tauri 固定使用 5173 端口，被占用时直接报错而不是换端口
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "es2022",
    sourcemap: false,
    rollupOptions: {
      input: {
        main: r("./index.html"),
        screenshot: r("./screenshot.html"),
        "shot-result": r("./shot-result.html"),
      },
    },
  },
  clearScreen: false,
});
