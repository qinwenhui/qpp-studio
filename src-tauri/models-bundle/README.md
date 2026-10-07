# models-bundle — 打包 staging 目录

`tauri build` 会把本目录的 `models/` 与 pdfium 库映射进安装包。平台差异收敛在
`tauri.windows.conf.json` / `tauri.macos.conf.json`（按平台合并进 bundle.resources）：

- Windows：`pdfium.dll`（来自 bblanchon/pdfium-binaries win-x64）
- macOS：`libpdfium.dylib`（来自 bblanchon/pdfium-binaries macos-arm64，
  2026-10 取 latest = pdfium 157.0.8086，7.0 MB）

升级时替换对应文件。Windows dev 靠 exe 同目录搜索（tauri 会把 resources 拷到
`target/debug/`）；macOS dev 同样拷到 exe 旁，安装态从 `Contents/Resources/`
按显式路径加载（见 `pdf.rs` 的 `bind_pdfium`）。

构建前运行 `node tools/stage-models.mjs` 从并排的 qppocr 仓库填充：

```
models/
├── tiny/    det.onnx  rec.onnx  dict.txt
├── small/   det.onnx  rec.onnx  dict.txt
├── medium/  det.onnx  rec.onnx            （不进安装包，仅手动放置）
├── cls.onnx
└── dict.txt                     ← = dict_small_medium.txt，补 medium 的字典查找
```

默认只打包 tiny + small + cls（源文件合计约 38.4 MB）。
