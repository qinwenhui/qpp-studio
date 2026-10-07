# macOS (Apple Silicon) 移植设计与开发方案

> 2026-10 制定。前置状态:v0.2.0 已发版(Windows),qppocr 0.3.1(CPU NEON + GPU Vulkan)已接入。
> 本方案目标:ARM Mac 原生 dmg,CI 构建,功能与 Windows 同源;视觉交互按 mac 惯例重做关键层,拒绝「Windows 套皮」。

## 0. 原则

- 平台差异收敛在少数模块(平台分支/配置),**不做两套 UI 代码**
- mac 上追求「像 mac 应用」:原生窗口装饰、菜单栏、Cmd 快捷键、材质主题
- 本地无 mac,**一切验证靠 CI 迭代**(workflow_dispatch 手动触发调试,接受分钟级 round-trip)
- 范围控制:下方「不做清单」同样重要

## 1. 窗口与系统 UI(核心差异层)

### 1.1 标题栏
现状:全局 `decorations:false + transparent` 自绘标题栏(Windows 风格圆点按钮),即「防 mac」的观感来源。
方案(平台分支):
- **Windows**:保持现状不动
- **macOS**:`titleBarStyle: "Overlay"`(tauri v2,macOS 专属窗口选项)+ transparent——内容延伸到标题栏下,**系统交通灯悬浮**,右侧保留我们的 🎨(主题)/⚡(引擎状态)与标题;左侧布局为交通灯让位(约 70pt padding)
- 实现:tauri.conf.json 的 windows 配置无法按平台分值 → 在 Rust 侧启动时按 `cfg!(target_os)` 设置 window 属性;前端 TitleBar.svelte 按 `platform` 状态(isMac)切换左区渲染(不画假圆点)

### 1.2 原生菜单栏(mac 惯例刚需)
tauri v2 Menu API 构建:
- 应用菜单:关于 QPP Studio / 设置…(Cmd+,) / 退出(Qmd+Q)
- 文件:打开图片…(Cmd+O) / 粘贴(Cmd+V) / 截图识别(Cmd+Shift+O)
- 编辑:撤销/剪切/复制/粘贴/全选(WKWebView 大部分自动响应,注册占位即可)
- 窗口:最小化/缩放
- Windows 上不建菜单(保持现状)。菜单事件走已有 Event/命令通道
- 窗口标题显示当前文档名(图片名/PDF 名)

### 1.3 快捷键体系 Ctrl→Cmd
- 应用内快捷键(前端 keydown):统一封装 `matchShortcut(e, 'mod+o')`,`mod` = mac ? metaKey : ctrlKey;涉及 Ctrl+O/V/1..3 与 PDF 翻页无冲突(方向键通用)
- 全局热键:注册时按平台选 accelerator(`Ctrl+Shift+O` / `Cmd+Shift+O`),设置页录制组件显示平台对应修饰键名
- tauri-plugin-global-shortcut 在 mac 用 RegisterEventHotKey,**无需辅助功能权限**

### 1.4 材质与主题
- `window-vibrancy` crate:mac 侧 `apply_vibrancy(NSVisualEffectMaterial)`;主题→材质映射(仅 mac):
  - 深色科技 → UnderWindowBackground(dark)| 浅色精致 → Light | 通透玻璃 → Titlebar/HudWindow
  - 可爱粉彩 / 古典纸墨 → 不启用材质,纯色背景(材质只给玻璃系主题)
- `windowfx.rs` 扩平台分支;主题切换时 mac 重应用材质(现有 apply_theme_effect 挂钩点)
- 字体栈:`--font` 加 `-apple-system, "PingFang SC"` 前置(mac 回退顺序),Windows 不变

## 2. 平台差异代码层(跑起来的前提)

| # | 问题 | 修法 |
|---|---|---|
| 1 | rfd 同步对话框要求 mac 主线程(pick_images 现在在 spawn_blocking) | 改 `rfd::AsyncFileDialog`(内部调度主线程,双平台通用) |
| 2 | pdfium:Windows 靠 exe 同目录 dll(LoadLibrary 语义);mac dyld 不搜 exe 目录,resources 落 `Contents/Resources/` | 下载 bblanchon `macos-arm64/libpdfium.dylib` 入 `models-bundle/`;pdf.rs 的 PDFIUM 初始化平台分支:mac 用 `Pdfium::bind_to_library(resource_dir 路径)`;resources 映射保持目录级(两平台各自用各自的文件) |
| 3 | `bundle.targets: ["nsis"]` 写死 | 改 `["nsis", "dmg", "app"]` 或按平台在 CI 传 `--target` 过滤 |
| 4 | 截图权限 | Info.plist `NSScreenCaptureDescription`(用途文案);被拒绝时截图按钮置灰+指引 |
| 5 | GPU | **零改动**:mac 无 Vulkan loader → `list_devices()` 空 → GPU 选项自动置灰;文案补「当前平台不支持(需 Vulkan)」;CPU 走 NEON(引擎 0.3.1 已备) |
| 6 | media:// 协议 | 已双形态兼容(`http://media.localhost` / `media://localhost`,记忆已记录) |
| 7 | 设置/历史/路径 | 均跨平台 API,无需动 |

## 3. CI

- **release.yml 加 `macos-14` job**(原生 arm64,公开仓库免费):checkout → node 20 → rust stable → `node tools/stage-models.mjs`(QPPOCR_MODELS_REPO 浅克隆路径已支持)→ 下载 pdfium dylib → `npm run tauri build` → dmg 上传同一 Release
- **ci.yml 加 mac job**:cargo check/clippy/test(hw 优化表单测) + svelte-check
- 签名:v1 未签名 dmg(README 注明「右键打开」绕 Gatekeeper);预留 `APPLE_CERTIFICATE` secrets 流程,后续接入公证
- 调试:workflow_dispatch 手动触发,产出 artifact 不发 Release

## 4. mac 专属体验机会(做)/范围控制(不做)

做:材质主题、原生菜单、交通灯 overlay 标题栏、Cmd 快捷键、字体栈、文档名窗口标题。
不做(防范围膨胀):Dock 进度徽章(无稳定 API)、NSScrollView 物理滚动(webview 注入不了)、Touch Bar(已死)、菜单栏常驻托盘(v2 再议)、iCloud/Handoff。

## 5. 实施阶段

- **阶段 1「跑起来」**:rfd async + pdfium mac 分支 + targets + Info.plist + ci.yml mac job(cargo check 过)→ 产出可启动的未签名 app(workflow_dispatch artifact)
- **阶段 2「像 mac」**:标题栏 overlay + 原生菜单 + Cmd 快捷键 + 字体栈 + windowfx 材质
- **阶段 3「发版」**:release.yml mac job + dmg 进 Release + README 安装说明(未签名右键打开)+ 设置页 GPU 文案平台化
- 每阶段以 CI 绿灯 + 截图(用户在 mac 上验收)为完成标准

## 6. 风险

- 无本地 mac:迭代全靠 CI,慢但可行;遇到只能在真机复现的问题(权限弹窗、材质效果)依赖用户验收
- pdfium dylib 加载路径(@executable_path/../Resources)是常见坑,阶段 1 优先验证
- WKWebView 与 WebView2 的 CSS 差异(backdrop-filter 已知 OK;未知项按出现逐个修)
- rfd Async 行为差异(filter 一致,预期低风险)

## 7. 与引擎团队的接口

- 无需引擎改动(NEON 已备,GPU 降级天然安全)
- 若未来引擎出 Metal 后端,`DeviceChoice` 接入点已预留(应用侧 parse_device 扩枚举即可)
