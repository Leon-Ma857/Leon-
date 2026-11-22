# Luma

轻量级跨平台桌面（Windows / macOS）快捷截图、原位翻译、OCR 识字、桌面贴图与 AI 视觉效率工具，基于 **Tauri v2 + Rust + React + TypeScript** 构建。

按下全局快捷键即可冻结当前屏幕并框选区域，一步完成识字、翻译、原位覆盖预览、看图问答、拼图或置顶贴图；支持 **AI 实时流式输出（SSE）**、**自定义网络代理与更新源** 以及 **应用内更新**，常驻系统托盘 / 菜单栏，随叫随到。

[![Latest Release](https://img.shields.io/github/v/release/Leon-Ma857/Leon-?label=最新版本&color=2b8a78)](https://github.com/Leon-Ma857/Leon-/releases/latest)
[![Platform](https://img.shields.io/badge/平台-Windows%2010%2F11%20x64-0078D6)](#安装windows-1011-x64)
[![macOS](https://img.shields.io/badge/macOS-Apple%20Silicon%2012%2B-111827)](#安装macos-apple-silicon)

## 核心特性

### 1. 截屏翻译、原位覆盖与实时流式输出
- **微信式冻结选区**：默认全局快捷键 `Alt+Q`（支持按键录制自定义），支持多显示器与 HiDPI 缩放。
- **原位沉浸翻译**：框选后按 `Tab` 直接在选区原位擦除原文并覆盖显示译文，自适应背景色与高对比度文字；支持一键复制、保存或贴图译文图。
- **AI 全链路实时流式输出（SSE）**：文本翻译台、结果浮窗（翻译 / OCR / 问 AI）、截图选区侧栏与悬浮叠堆批量处理均支持逐字实时渲染，首字响应更快；对不支持流式的接口自动兼容回退。
- **多引擎自由切换**：
  - **视觉 AI（推荐）**：一步看图识字并自然意译，内置 OpenAI、通义千问（Qwen-VL）、硅基流动、智谱（GLM-4V）、Google Gemini、本地 Ollama 等 OpenAI 兼容接口预设。
  - **Google 翻译 / DeepL**：本地 OCR（Windows 使用 RapidOCR / Windows OCR，macOS 使用 Apple Vision OCR）提取文字后调用翻译引擎。
- **自定义术语表与提示词**：支持配置专有名词对照表（Glossary）与多组提示词预设，统一专业词汇译法。

### 2. 截图标注、长截图与取色工具
- **标注与批量加选**：矩形、椭圆、箭头、画笔、隐私马赛克、文字标注，支持多区域加选并发处理。
- **像素级精准操作**：23×23 实时放大镜与坐标 / `#HEX` 取色器（未框选时按 `C` 复制色值），选区支持方向键 `1px` 微调与 `Shift+方向键` 调尺寸。
- **实用扩展**：支持**滚动长截图**自动拼接、**选区二维码识别**、**剪贴板图片直译/识字**、**剪贴板文字速译**。

### 3. 悬浮叠堆工作台与桌面贴图
- **悬浮叠堆**：连续框选多张截图收入右下角叠堆，支持**竖排 / 横排 / 宫格拼图导出**、**批量翻译**、**批量识字**、**多图联合问 AI**，并可将结果导出为 Markdown 笔记。
- **桌面贴图**：任意截图或原位译文图可一键置顶贴在桌面，支持滚轮缩放、90° 旋转、水平镜像翻转、透明度切换与一键复位。

### 4. 网络代理、历史管理与签名自动更新
- **网络代理与自定义更新源**：支持实时**跟随 Windows 系统代理**（支持 HTTP / HTTPS / SOCKS5 与系统豁免规则，开关系统代理无需重启）或手动配置代理，自动豁免 `localhost` / `127.0.0.1` 本地 Ollama 直连；支持自定义 HTTPS 更新源镜像与一键连通性测试。
- **悬浮译文卡片 & 历史管理**：默认采用「跟随来源应用层 + 点空白自动关闭」（切到其他软件不遮挡也不丢失，切回原软件自动浮现；按需手动图钉置顶），支持按需合并 PDF/分栏多余换行、在线编辑原文重译、语音朗读；自动持久化保存最近 100 条记录，支持搜索筛选与 Markdown / JSON 导出。
- **后台自动检查与签名更新**：支持浅色 / 深色 / 跟随系统主题、开机自启、配置导入导出（自动排除 API Key）。启动及驻留托盘期间自动检查更新，应用内弹窗展示更新日志与实时下载进度条，安装前强制校验 Tauri 更新公钥签名并拒绝降级。

### 5. 划词翻译、「问 AI」快捷动作与离线 OCR（v0.7.25+）
- **全局划词翻译（不污染剪贴板）**：在设置页录制全局划词快捷键（支持智能单键 `Space` / `Tab` 或 `Alt+E` 等组合键），鼠标选中文字后轻按即可在原文旁紧凑弹出 Apple 风格毛玻璃译文浮窗并自动避让不遮挡正文；浮窗跟随来源应用层（切软件不覆盖其他应用也不丢失，切回原软件自动回到上方，点原软件空白处或桌面自动关闭），读取后完整还原原剪贴板内容，纯文本结果同样可切换「问 AI」追问。
- **「问 AI」自定义快捷动作**：所有「问 AI」输入框上方提供可横向滚动的 Prompt 预设胶囊，一键流式提问；支持行内 `+` 新增、删除、恢复默认与设置页排序，修改后跨窗口实时同步。
- **离线高精度 OCR（RapidOCR ONNX）**：按需下载（约 70 MB，强制 SHA-256 校验）或导入本地 `.7z` 引擎包，支持「智能自动 / 优先 RapidOCR（完全离线）/ 仅 Windows OCR」三种识别策略与多语言模型切换，内置冒烟测试、空闲 5 分钟自动释放内存与表格对齐重建。

---

## 安装（Windows 10/11 x64）

下载发行页中的 `Luma_<版本>_x64-setup.exe`，运行安装程序，然后从开始菜单搜索 **Luma**。安装程序会把 `WebView2Loader.dll` 放到程序旁边。电脑缺少 Microsoft Edge WebView2 Runtime 时，安装程序会联网下载安装。

## 安装（macOS Apple Silicon）

支持 **macOS 12+（Apple Silicon M 系列 / arm64）**：

1. 从 [GitHub Releases](https://github.com/Leon-Ma857/Leon-/releases/latest) 下载 `Luma_<版本>_aarch64.dmg`，打开并将 `Luma.app` 拖入「应用程序（Applications）」文件夹。
2. **首次打开解除 Gatekeeper 隔离提示**：由于当前 macOS 安装包暂未进行 Apple Developer 公证签名，如系统提示“已损坏，无法打开”或“无法验证开发者”，请在终端运行以下命令后重新打开：

```bash
xattr -cr /Applications/Luma.app
```

3. **授予定位与划词所需系统权限**（`系统设置 → 隐私与安全性`）：
   - **录屏与系统录音**：用于区域截图、滚动长截图与像素取色。
   - **辅助功能**：用于全局划词翻译（包括单键 `Space` / `Tab` 触发与非破坏性读取选区文字）及滚动长截图。

---

## 从源码构建（macOS Apple Silicon）

1. 安装 **Xcode Command Line Tools**（`xcode-select --install`）、**Node.js v20+** 与 **Rust 工具链**（`rustup target add aarch64-apple-darwin`）。
2. 安装前端依赖并生成 `.icns` 等图标资源后构建 `.dmg`：

```bash
npm ci
npm run tauri -- build --target aarch64-apple-darwin --bundles dmg
```

---

## 从源码构建（Windows）

1. **Node.js**（建议 v20+）
2. **Rust 工具链**（`rustup` + `cargo` + `stable-x86_64-pc-windows-gnullvm` 或 MSVC Build Tools，安装指南见 [Tauri 官方文档](https://v2.tauri.app/start/prerequisites/)）
3. **Microsoft Edge WebView2 Runtime**（Windows 10/11 通常已自带）

进入项目目录后安装依赖并启动开发环境：

```bash
npm install
npm run tauri dev
```

> 说明：`npm install` 的 `postinstall` 钩子（以及 `npm run tauri` 前置脚本）会自动执行 `scripts/setup-icons.mjs`，在 `src-tauri/icons/` 生成应用所需的 `.png` 与 `.ico` 图标文件。

构建普通安装包不需要私钥。此项目的发行脚本使用 `stable-x86_64-pc-windows-gnullvm` 工具链：

```bash
npm run tauri -- build --bundles nsis
```

### 发布 Windows 更新

本机的 NSIS 安装包和 Tauri 更新签名通过发布脚本生成。在项目目录的 Windows PowerShell 中运行下列命令。发布前同步递增 `package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 和 `src-tauri/tauri.conf.json` 中的版本号。

```bash
powershell.exe -NoProfile -File scripts/release-windows.ps1 -Stage
```

`-Stage` 仅将签名安装包和清单放到本机 `%LOCALAPPDATA%\Luma\Updates`，不会向其他电脑发布。

**联网发布：** 先确定一个长期、匿名可访问的 HTTPS 更新地址，构建时用 `-UpdateFeedUrl` 写入应用。下面是 GitHub Releases 示例：

```bash
powershell.exe -NoProfile -File scripts/release-windows.ps1 -UpdateFeedUrl https://github.com/Leon-Ma857/Leon-/releases/latest/download/latest.json
```

脚本在 `src-tauri/target/release/bundle/nsis` 生成安装包、`.sig` 和 `latest.json`。将 `latest.json` 和本版本的 `Luma_<版本>_x64-setup.exe` 作为同一次正式 Release 的附件，使用 `v<版本>` 格式的标签（例如 `v0.7.24`），两个附件都上传完成后再发布。不要将私钥、密码或本机配置上传到任何发布源。Luma 安装前验证更新签名及签名绑定的版本，拒绝降级。

从 `0.7.23` 起，Luma 在启动约 5 秒后自动联网检查，运行期间每 6 小时检查一次，连接失败时 15 分钟后重试。应用缩在托盘时也会检查。发现新版后会发送 Windows 通知，托盘菜单和设置页会显示可更新的版本；`0.7.24` 起通过应用内弹窗展示更新内容与实时流式下载进度，并支持自定义代理与更新源镜像。

更新公钥内置于 Luma。首次创建密钥时用 `-InitializeSigningKey` 打印对应公钥。密钥保存在 `%LOCALAPPDATA%\LumaReleaseSigning`，密码由 Windows 为当前用户加密。若发布机器已有受 Windows 信任的代码签名证书，可向脚本传入 `-CertificateThumbprint` 完成 Authenticode 签名。

---

## 目录结构

```text
snaptrans/
├── scripts/
│   ├── setup-icons.mjs            # 纯 Node 生成应用 PNG/ICO 图标
│   ├── release-windows.ps1        # Windows NSIS 打包、Tauri 更新签名与清单校验
│   └── test-online-update.mjs     # 本地 HTTPS 更新回归测试夹具
├── src/                           # React + TypeScript 前端（多视图共用单入口）
│   ├── main.tsx                   # 根据 ?view= 路由至主设置/选区/结果/贴图/叠堆窗口
│   ├── appUpdate.ts               # 应用内更新 IPC 封装与下载进度订阅
│   ├── types.ts                   # 前后端共享类型与模型预设
│   ├── styles/global.css          # 浅/深色自适应设计系统
│   ├── components/
│   │   ├── AskQuickActionsBar.tsx # 「问 AI」快捷动作胶囊栏与行内增删
│   │   ├── LocalOcrCard.tsx       # 离线 RapidOCR 安装/切换/卸载与冒烟测试卡片
│   │   └── SelectionShortcutField.tsx # 全局划词快捷键录制与单键智能提示
│   └── views/
│       ├── MainSettings.tsx       # 设置页、代理/更新源配置、文本翻译台与历史记录
│       ├── UpdateButton.tsx       # 顶栏更新入口与实时下载进度更新弹窗
│       ├── OverlayCapture.tsx     # 全屏冻结选区、标注、原位翻译与侧栏流式面板
│       ├── ResultPopup.tsx        # 悬浮译文/识字/问 AI 流式结果卡片
│       ├── PinWindow.tsx          # 桌面置顶贴图窗口
│       └── StackWindow.tsx        # 悬浮叠堆、拼图导出与批量流式分析工作台
└── src-tauri/                     # Tauri v2 + Rust 后端
    ├── Cargo.toml
    ├── tauri.conf.json            # 多窗口与 NSIS 安装器配置
    ├── capabilities/              # 按窗口隔离的 Tauri v2 最小权限 ACL
    └── src/
        ├── lib.rs                 # 应用初始化、系统托盘、全局快捷键与窗口命令
        ├── capture.rs             # xcap 截屏、归一化裁剪与多图拼接
        ├── scroll_capture.rs      # 滚动长截图自动拼接
        ├── selection_capture.rs   # 剪贴板零污染划词读取、单键智能钩子与避让定位
        ├── result_popover.rs      # 结果浮窗来源应用层 Z 序跟踪与点空白自动关闭
        ├── win_ocr.rs             # Windows 原生 OCR 识别与中英混排/下划线修复
        ├── local_ocr.rs           # 离线 RapidOCR ONNX 按需下载、子进程作业管理与排版
        ├── qr.rs                  # 选区二维码解码
        ├── translate.rs           # OpenAI 兼容 SSE 流式引擎、Google/DeepL 与回退逻辑
        ├── updates.rs             # 后台自动检查、代理/镜像回退与签名流式下载安装
        └── config.rs              # config.json 读写、Windows 系统代理动态解析与回环直连
```

## License

[MIT](./LICENSE)
