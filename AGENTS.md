# AGENTS.md — HiScreen 项目规范

本规范约束本仓库（HiScreen，基于 tauri-bootstrap-demo 脚手架派生），AI 编码助手必须遵守。
**基线：脚手架 v0.1.0**。基座 = Rust + Tauri 2 + Vue 3 + Vite + TS + Naive UI 的无边框 Fluent 桌面应用，开箱含标题栏 / 状态栏 / 关于弹窗 / 主题与 i18n / 设置持久化 / CI 发布；HiScreen 将脚手架的菜单栏替换为**左侧导航栏**，并加入屏幕分享业务（采集 / 推流 / 观众页）。

---

## 0. HiScreen 业务架构（脚手架规范之外的部分）

- **推流链路**：`capture.rs`（scrap 采集 → jpeg-encoder 编码）与 `audio.rs`（WASAPI loopback / cpal → 48k 单声道 i16，20ms 分片）把二进制帧写入 `hub.rs` 的 tokio broadcast 通道；`server/mod.rs`（axum）向每个 WS 观众转发。二进制帧首字节为类型：`1`=JPEG 视频、`2`=音频；控制面为 JSON 文本（meta / notice / ping / pong / stats）
- **观众页**：自包含 `server/viewer.html`（include_str! 内嵌），深色 UI + 简单双语（按浏览器语言），视频 canvas 绘制、AudioContext 调度播放（默认静音，点击解锁）、统计（FPS/分辨率/RTT）、截图、全屏、通知横幅、断线重连
- **状态轮询**：前端 `stores/share.ts` 每 2s 调 `server_status` + `get_logs`（单一轮询器），不做 Tauri 事件推送
- **线程生命周期**：采集/音频线程由 `MediaState` 的 `Mutex<Option<Handle>>` 托管，Drop 即置 stop 标志并 join；scrap 的 `frame()` 为 0ms 超时轮询（WouldBlock），停止分享无阻塞风险
- **日志**：Rust `hub.log()` 写入 200 条环形缓冲；日志文案为中文（数据层，不走 i18n）
- **事件顺序约定**：`share_start` 会先停旧线程再建新线程，并同步 `hub.share` + `broadcast_meta`；观众端始终以 meta 为准
- **预览与推流互斥**：分享进行中 `capture_preview` 直接拒绝（后端检查 `MediaState.capture`，前端 `loadPreview` 检查 `status.sharing`），避免与推流线程抢占同一输出的 DXGI duplication——预览轮询由 ShareView 持有（1Hz），停止分享后自动恢复

## 1. 技术栈（固定，不得替换）

- Vue 3 `<script setup>` + TS strict + Vite 7 + Naive UI 2.x + vue-i18n 11；Rust + Tauri 2.12
- **不用** vue-router / pinia / axios / tailwind：视图用条件渲染，跨组件状态用 `src/stores/` 的 `reactive` 模块单例
- 业务依赖：scrap 0.5（采集）、jpeg-encoder（编码）、axum + tokio（服务）、qrcode（二维码 SVG）、base64；Windows 音频走 windows crate 直调 WASAPI（**不再引入 wasapi crate**），Unix 音频走 cpal
- 基线依赖：`vite ^7` / `@vitejs/plugin-vue ^6` / `vue-tsc ^3` / `typescript ^5.9` / `@tauri-apps/api ^2` / `naive-ui ^2.41` / `vue-i18n ^11` / Node ≥ 20.19 / Rust stable (MSVC)
- 代码注释用英文；界面文案一律走 i18n（观众页 viewer.html 与 Rust 日志除外）

## 2. 目录约定

```
src/
├─ main.ts            # 引导顺序固定：loadSettings → applyTheme → setLanguage → mount
├─ App.vue            # NConfigProvider 接线（主题/语言）+ 骨架（TitleBar/Sidebar/五视图/StatusBar + 弹窗）+ initShare()
├─ components/        # 全部 Vue 组件（五视图：ShareView/AudienceView/NetworkView/QualityView/LogView；AudiencePanel 在开始分享页与观众页复用）
├─ lib/               # 纯逻辑：i18n / theme / tauri(api 封装) / icons / appInfo / dialogDrag
├─ stores/            # settings.ts（持久化设置）、share.ts（运行时分享状态+轮询）、ui.ts（瞬时 UI 状态）
├─ theme/fluent.ts    # Naive UI GlobalThemeOverrides（fluentLight / fluentDark）
└─ styles/global.css  # Fluent 设计令牌（CSS 变量）+ 基础样式 + 通用覆盖（含 .card/.view-title 等业务样式）
public/locales/       # 语言包，一种语言一个 JSON
scripts/              # gen-icon.mjs（显示器+播放图标）、sync-version.mjs
src-tauri/src/        # main.rs → lib.rs(Builder+MediaState) → config.rs → hub.rs → capture.rs → audio.rs → commands/ → server/(mod.rs+viewer.html)
```

新组件进 `components/`，纯函数进 `lib/`，状态进 `stores/`，IPC 封装进 `lib/tauri.ts` 的 `api` 对象，**组件里不得直接 `invoke()`**。

## 3. 质量门槛与构建

- **门槛①** `npm run build`（vue-tsc strict + vite build）零错误；**门槛②** `cargo check` 零警告。任何改动完成前两者全绿；UI 改动自测 **浅/深 × 中/英** 四组合
- `build.bat <mode>`：`release`（NSIS/MSI）/ `dev` / `debug` / `exe`（仅调试 exe，最快）/ `check`（typecheck + cargo check）
- **端口**：每个项目用互不冲突的独立端口（strictPort），`vite.config.ts` 的 `port`、`tauri.conf.json` 的 `devUrl`、`build.bat` 的端口检查三处一致
- Windows 停后台 dev server：`netstat -ano | grep :<port>` 找 PID 后 `taskkill //PID <pid> //F`（只杀 npm 包装进程会残留 vite 子进程占端口；杀前用进程命令行确认归属）
- CI（`.github/workflows/release.yml`）：推 `v*` 标签或手动 `workflow_dispatch` 触发；四平台矩阵 Windows x64 / Linux x64 / macOS arm64 / **macOS Intel 在 macos-latest(arm64) 上交叉编译**；构建前 `node scripts/sync-version.mjs <tag版本>`；无需额外 secrets

## 4. 项目命名与版本号（强制）

- **项目名区分大小写，是唯一命名来源**：项目目录名、`package.json` name、`Cargo.toml` package.name、`tauri.conf.json` productName、**交付物可执行文件名**、便携 zip 名，全部与提供的项目名逐字符一致（不得擅自改小写、加前缀后缀）
  - 可执行文件名由 **Cargo `package.name`** 决定（如 `tauri-bootstrap-demo.exe`）；productName 决定安装包名；`[lib] name` 用下划线形式 `<项目名>_lib`，并同步 `main.rs` 的调用
- **版本号唯一来源：`src-tauri/Cargo.toml` 的 `[package] version`**，改版本只改这一处
  - `tauri.conf.json` **禁止写 `version`**（自动回退 Cargo 版本 → 程序集信息 / 安装包跟随）
  - 前端**禁止硬编码版本号**，显示一律 `api.appVersion()`（IPC 读 `package_info()`）
  - `package.json` 版本由 `scripts/sync-version.mjs` 镜像（挂 `predev`/`prebuild`）；该脚本**无参=镜像，带参=设定**（支持 `v` 前缀与预发布号，CI 用）
- 官网、作者等非版本元信息放 `src/lib/appInfo.ts`

## 5. 主题系统（Windows 11 Fluent）

- **双令牌文件必须一致**：`styles/global.css`（`:root` 与 `[data-theme="dark"]` CSS 变量，自绘界面用）+ `theme/fluent.ts`（fluentLight/fluentDark，Naive 组件用）。fluent.ts 中可直接引用 CSS 变量（如 `var(--bg-layer)`）实现自动跟随
- **组件禁止写死颜色**；默认浅色，主题值只允许 `"light" | "dark"`，应用方式 = `dataset.theme` + `lib/theme.ts` 的 `resolvedTheme`
- 核心取值：accent 浅 `#0067c0`（hover `#0078d4`/pressed `#005ba1`，accent 上文字白）/ 深 `#60cdff`（文字 `#003a5c`）；success `#0f7b0f`/`#6ccb5f`；warning `#9d5d00`/`#ffb02e`；danger `#c42b1c`/`#ff99a4`；窗口底 `rgba(243,243,243,.86)`/`rgba(32,32,32,.86)`，弹层 `#ffffff`/`#2b2b2b`；控件圆角 4px、弹层 8px；字体 Segoe UI Variable Text 栈；标题栏 40px / 菜单栏 36px / 状态栏 30px
- **已知坑**：Naive Dropdown 的行高/字号由 `size` 预设内联写入弹层根元素，优先级高于主题覆盖——调行高必须 `size="large"`；宽度/圆角类（`borderRadius`、`optionPrefixWidth`）可正常覆盖。同理，覆盖 Naive 内置定位样式时注意其双类选择器特异性（如 `body .n-message-container.n-message-container--bottom-right`）

## 6. i18n

- 语言包 `public/locales/<locale>.json`（按 `app/nav/share/audience/network/quality/log/about/settings/status` 分节），运行时 `fetch` 加载；偏好值 `"system" | "zh-CN" | "en-US"`，`system` 按 `navigator.language` 解析（`zh*`→zh-CN），回退 zh-CN；持久化读回的值必须过 `normalizeLangOption` / `normalizeThemeOption`
- **组件中禁止硬编码界面文案**；语言下拉中语言名用原生写法；消息文本避免 `@` 与 `|`
- 新增语言三步：① 新 locale JSON ② `i18n.ts` 的 `LANGUAGE_PREFERENCES` ③ `AboutDialog.vue` 的 `languageOptions`
- 语言包不在 vite 模块图内，`vite.config.ts` 的 `localeReloadPlugin` 监听并整页刷新（勿删）

## 7. 窗口与 UI

- 窗口：`decorations:false` + 不透明（`transparent:false`，无 Mica，底色走 `--bg-window` 实色）+ 居中，1080×720 / min 800×560；全局禁用 WebView2 默认右键菜单——在 `lib.rs` setup 中经 `with_webview` 调 `ICoreWebView2Settings6::SetAreDefaultContextMenusEnabled(false)`（Windows）
- 标题栏：整栏拖拽，右侧为 ℹ 关于按钮（打开「关于」弹窗）+ 最小化/最大化/关闭 三钮，关闭钮 hover `#c42b1c` 白字；**侧边栏**（替代脚手架菜单栏）：172px 左栏，五项导航（开始分享/观众管理/网络设置/音频与质量/日志），激活项 accent 左缘 3px 指示条；状态栏：服务状态点 + 端口 + 观众数 + 版本号（浏览器环境隐藏版本）
- 弹窗：**一律用统一基础组件 `components/DialogCard.vue` 创建**（内部封装 `NModal` + `useDraggableCard`，含 Fluent 卡片样式、标题栏、关闭钮、footer 分隔线；遮罩点击与 Esc 一律不关闭，关闭仅能通过标题栏 X 或 footer 插槽按钮）。能力经 props 控制（**挂载时读取，运行时不响应变更**）：`draggable`（标题栏拖拽移动，默认 true）、`resizable`（边缘/四角缩放，默认 true，最小尺寸 `minWidth`/`minHeight`，视口四周留 24px）；`title`/`aria-label`/`close-label` 走 props，正文默认插槽、`footer` 具名插槽。不得各自手写 NModal + 拖拽/缩放接线。「关于」弹窗（`:draggable="false" :resizable="false"`）承载版本信息与**主题/语言切换**（继承自脚手架设置弹窗，独立设置弹窗已删除）
- **toast（操作反馈）**：右下角弹出（`NMessageProvider placement="bottom-right" :duration="4000" :max="3" keep-alive-on-hover`）；样式 = Fluent 卡片（fluent.ts `Message` 覆盖，值引用 CSS 变量）+ 左缘 3px 类型色条（global.css）。**类型语义强制**：操作成功 `message.success`（绿）、提醒/提示 `message.info`（蓝 accent）、操作失败 `message.error`（红）、警告 `message.warning`（黄）
- 图标一律 `Icon.vue` + `lib/icons.ts` 行内 SVG（24 网格 1.6 描边），不引第三方图标库；`body` 上 `user-select:none`；滚动条用 global.css 的 Fluent 细样式

## 8. 设置持久化

- Rust `config.rs`：`AppConfig`（`#[serde(default)]`，含 language/theme/port/quality/fps/audio/autoStart/serverAutoStart，数值字段 `sanitized()` 钳制）+ `AppState` Mutex 托管，存 `{app_data_dir}/config.json`，命令 `get_config`/`save_config`
- 前端 `stores/settings.ts`：启动加载→归一化→应用；`persistSettings()` 异步持久化；质量/帧率/音频变更还需 `applyShareParams()` 推给 Hub 原子量
- 新增设置项四处同步：`config.rs` 字段 → settings store → 对应视图控件 → normalize/clamp 函数；浏览器环境读写必须 try/catch 静默降级，**不得阻塞启动**

## 9. IPC 与插件

- 新增命令三步：`commands/mod.rs` 写 `#[tauri::command]` → `lib.rs` 注册 → `lib/tauri.ts` 的 `api` 封装
- 现有命令：`get_config` `save_config` `app_version` `list_displays` `capture_preview` `server_status` `server_start` `server_stop` `update_share_params` `share_start` `share_stop` `broadcast_notice` `kick_viewer` `get_logs` `clear_logs`
- **防火墙策略**：应用不修改系统防火墙（`add_firewall_rule` 已删除，勿再实现类似命令）；连接问题在「网络设置」页给出手工放行指引，由用户自行配置
- 所有前端代码必须兼容纯浏览器运行（`isTauri()` 分支 + try/catch），保证无 Tauri 时可独立调试与可视化验证（显示器/观众数据走 mock）
- 外部链接用 `openExternal()`（Tauri 内走 opener 插件，浏览器降级 `window.open`）
- 新增插件四处接线：Cargo.toml → `lib.rs` `.plugin()` → capabilities 权限（最小化）→ 前端封装

## 10. 从脚手架同步改动（历史参考）

本项目由 tauri-bootstrap-demo 派生，派生时的适配清单（后续如重新对齐脚手架可参考）：

1. 全局改名：package.json / Cargo.toml（package.name=`HiScreen`、lib=`HiScreen_lib`）/ tauri.conf.json（productName、identifier `com.hicode.hiscreen`、窗口 title）/ index.html / 语言包 `app.name`
2. 端口：vite `1530`（tauri devUrl 与 build.bat 检查三处一致）；观众服务端口 7870（config 默认值）
3. 图标：`scripts/gen-icon.mjs` 改为显示器+播放 glyph → `npm run gen:icon`
4. 结构：MenuBar/WelcomeView 删除，新增 Sidebar + 五视图 + stores/share；Rust 端新增 hub/capture/audio/server
5. CI：releaseName / 便携 zip 名 / Linux 追加 libasound2-dev + pkg-config

## 11. 其他

- 不提交 `node_modules`、`dist`、`src-tauri/target`、`src-tauri/gen/schemas`
- 结构性改动同步更新 `README.md`（面向人）与本文件（面向代理）
- npm 安装时 esbuild postinstall 可能被 allowScripts 拦截——无害；构建异常时 `npm install-scripts approve esbuild` 后重装
- `Cargo.toml` 的 windows 目标依赖（`webview2-com` / `windows`）版本必须与 tauri 自身的 WebView2 依赖一致，否则 COM 类型无法统一编译失败
