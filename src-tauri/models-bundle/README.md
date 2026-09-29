# models-bundle — 打包 staging 目录

`tauri build` 会把本目录的 `models/` 与 `pdfium.dll` 映射进安装包（见 `tauri.conf.json` 的 `bundle.resources`）。

`pdfium.dll` 来自 bblanchon/pdfium-binaries（win-x64），升级时替换本文件并同步
`target/debug/pdfium.dll`、`target/release/pdfium.dll`（dev 运行靠 exe 同目录搜索）。

构建前运行 `python tools/stage-models.py` 从并排的 qppocr 仓库填充：

```
models/
├── tiny/    det.onnx  rec.onnx  dict.txt
├── small/   det.onnx  rec.onnx  dict.txt
├── medium/  det.onnx  rec.onnx            （不进安装包，仅手动放置）
├── cls.onnx
└── dict.txt                     ← = dict_small_medium.txt，补 medium 的字典查找
```

默认只打包 tiny + small + cls（源文件合计约 38.4 MB）。
