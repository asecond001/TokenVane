# TokenVane

[简体中文](#简体中文) · [English](#english)

---

## 简体中文

TokenVane 是一款桌面端与 iOS 监测工具，用于统计 AI 编程工具的 Token 消耗、折算费用与订阅配额余量。

它以高密度界面汇总多款编码工具的实时账单与速率窗口，全部数据均通过合规渠道采集。

### 核心特性

- **高密度桌面工作台（Windows）**
  - 提供概览、趋势、月度账单（含 365 天热力图）、模型单价、订阅配额与生成速度拆解。
  - 常驻系统托盘，实时展示今日花费、工具占比、近期活跃会话与订阅窗口余量。
  - 支持深浅色主题跟随系统、`USD` / `CNY` 实时换算、多机汇总与单机筛选。

- **主流工具与自定义日志接入**
  - 内置支持 **Claude Code**、**Codex**、**Antigravity**、**Kimi Code**、**Workbuddy**、**DeepSeek Harness (`dsh`)**、**QCode** 与 **MiniMax Code**。
  - 支持在界面中配置兼容 OpenAI / Anthropic 格式或 JSON 指针的本地日志规则，并提供单行样本实时验证。

- **合规数据采集**
  - **本地日志读取**：通过共享读锁增量尾随各工具落盘的 JSONL 日志，全程保持只读访问。
  - **本地 OTLP 接收**：可选监听 `127.0.0.1` 回环端口，接收按官方规范导出的 OpenTelemetry 遥测数据。
  - **官方公开接口**：配置保存在系统凭据库中的官方 API Key 后，可定期查询余额与官方配额窗口，并自动将套餐加入订阅监控。

- **自托管多端同步与原生 iOS 客户端**
  - 基于 Cloudflare Worker 与 Durable Objects 实现多台电脑的用量汇总同步。
  - 仅同步聚合后的用量统计指标，支持开启 `AES-256-GCM` 端到端加密（E2EE），使服务端仅保存密文。
  - **iOS / iPadOS 原生客户端（`apps/ios`）**：采用 `SwiftUI` 与 `Swift Charts` 构建，通过 `UniFFI` 复用 Rust 核心计费与解密逻辑，支持桌面/锁屏小组件（`WidgetKit`）与灵动岛实时动态（`Live Activities`）展示 5h / 7d 配额窗口倒计时。

### 快速开始

#### 环境要求

- Rust (`stable-x86_64-pc-windows-msvc`) 与 MSVC Build Tools
- Node.js 20+ 与 `pnpm`
- Windows WebView2 Runtime

#### 常用命令

```powershell
# 安装依赖
pnpm install

# Rust 静态检查与测试
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline

# 前端与 Worker 类型检查及单元测试
pnpm typecheck
pnpm test

# 构建前端与桌面端安装包 (.msi / NSIS .exe)
pnpm build
pnpm desktop:build

# 运行 iOS 工程与跨语言绑定本地校验
pnpm run ios:check:local

# 运行 100 万行 JSONL 冷启动与增量续读压测
cargo run --release --offline -p tokvane-collectors --example bench_million_lines
```

打包产物路径：
- 可执行文件：`target/release/tokvane-desktop.exe`
- MSI 安装包：`target/release/bundle/msi/TokenVane_<版本号>_x64_en-US.msi`
- NSIS 安装包：`target/release/bundle/nsis/TokenVane_<版本号>_x64-setup.exe`

### 自托管云端同步（Cloudflare Worker）

部署 `services/sync-worker` 即可在多台电脑与手机间同步实时配额：

1. 登录 Cloudflare 并设置同步口令 `SYNC_TOKEN`：
   ```powershell
   cd services/sync-worker
   npx wrangler login
   npx wrangler secret put SYNC_TOKEN
   ```
2. 部署 Worker：
   ```powershell
   npx wrangler deploy
   curl https://<你的-worker-子域名>.workers.dev/healthz
   ```
3. 在桌面端 **偏好设置 -> 云端多端同步** 中填入 Worker 地址与 `SYNC_TOKEN`，可按需启用 **端到端加密（E2EE）** 并设置加密口令。iOS 端可通过扫码或粘贴配对信息接入。

### 自动更新与发布签名

- **持续集成与发布**：推送到 `main` 或提交 PR 时，`.github/workflows/ci.yml` 会执行前端、Sync Worker 与 Rust 检查；推送 `v*` 标签时，`.github/workflows/release-desktop.yml` 会构建签名安装包并生成 `latest.json` 更新清单。
- **版本同步**：运行 `pnpm version:bump 0.x.y` 同步工作区各模块版本号，提交后推送 `v0.x.y` 标签触发发版。
- **自定义发布签名**：
  1. 运行 `pnpm tauri signer generate` 生成 Minisign 密钥对。
  2. 将私钥存入 GitHub Secrets 的 `TAURI_SIGNING_PRIVATE_KEY`（及可选的 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）。
  3. 将公钥填入 `apps/desktop/src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。

---

## English

TokenVane is a desktop and iOS monitor for tracking token consumption, cost estimates, and subscription quota windows across AI coding tools.

It consolidates real-time billing and rate-limit metrics into a clean, high-density workspace using compliant data collection channels.

### Key Features

- **High-Density Desktop Workspace (Windows)**
  - Displays overview metrics, trends, monthly bills with a 365-day heatmap, model pricing, subscription quotas, and generation speed breakdowns.
  - Lives in the system tray for quick access to daily spend, tool distribution, active sessions, and remaining quota windows.
  - Supports system theme sync, real-time `USD` / `CNY` conversion, multi-device aggregation, and single-device filtering.

- **Built-in Tool Integrations & Custom Log Rules**
  - Out-of-the-box support for **Claude Code**, **Codex**, **Antigravity**, **Kimi Code**, **Workbuddy**, **DeepSeek Harness (`dsh`)**, **QCode**, and **MiniMax Code**.
  - Custom local log rules compatible with OpenAI / Anthropic schemas or JSON Pointers, complete with single-line live testing in the UI.

- **Compliant Telemetry Collection**
  - **Read-Only Local Logs**: Incrementally tails standard JSONL logs using shared read locks.
  - **Local OTLP Receiver**: Optionally listens on `127.0.0.1` to receive standard OpenTelemetry exports.
  - **Official Public APIs**: Queries account balances and quota windows using official API keys stored in the OS credential store, automatically adding detected plans to subscription monitoring.

- **Self-Hosted Sync & Native iOS App**
  - Aggregates usage across multiple machines through Cloudflare Worker and Durable Objects.
  - Transmits aggregated usage metrics exclusively, with optional `AES-256-GCM` end-to-end encryption (E2EE) so the server stores ciphertext only.
  - **Native iOS / iPadOS App (`apps/ios`)**: Built with `SwiftUI` and `Swift Charts`, sharing core Rust billing and decryption logic via `UniFFI`. Includes Home/Lock Screen widgets (`WidgetKit`) and Dynamic Island `Live Activities` for 5h / 7d window countdowns.

### Quick Start

#### Prerequisites

- Rust (`stable-x86_64-pc-windows-msvc`) and MSVC Build Tools
- Node.js 20+ and `pnpm`
- Windows WebView2 Runtime

#### Common Commands

```powershell
# Install dependencies
pnpm install

# Rust lints and tests
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline

# Frontend and Worker typecheck and unit tests
pnpm typecheck
pnpm test

# Build frontend and desktop installers (.msi / NSIS .exe)
pnpm build
pnpm desktop:build

# Verify iOS project and UniFFI bindings locally
pnpm run ios:check:local

# Run 1M-line JSONL cold-start and incremental tail benchmark
cargo run --release --offline -p tokvane-collectors --example bench_million_lines
```

Build artifacts:
- Executable: `target/release/tokvane-desktop.exe`
- MSI installer: `target/release/bundle/msi/TokenVane_<version>_x64_en-US.msi`
- NSIS installer: `target/release/bundle/nsis/TokenVane_<version>_x64-setup.exe`

### Self-Hosted Cloud Sync (Cloudflare Worker)

Deploy `services/sync-worker` to sync usage summaries across desktops and iOS:

1. Log in to Cloudflare and configure your `SYNC_TOKEN` secret:
   ```powershell
   cd services/sync-worker
   npx wrangler login
   npx wrangler secret put SYNC_TOKEN
   ```
2. Deploy the Worker:
   ```powershell
   npx wrangler deploy
   curl https://<your-worker-subdomain>.workers.dev/healthz
   ```
3. Open **Preferences -> Cloud Multi-Device Sync** on desktop, enter the Worker URL and `SYNC_TOKEN`, and optionally enable **End-to-End Encryption (E2EE)** with a passphrase. Pair the iOS app via QR code or pairing string.

### Auto-Update & Release Signing

- **CI & Release Workflows**: Pushing to `main` or opening a PR runs frontend, Sync Worker, and Rust checks via `.github/workflows/ci.yml`. Pushing a `v*` tag triggers `.github/workflows/release-desktop.yml` to build signed installers and generate the `latest.json` update manifest.
- **Version Management**: Run `pnpm version:bump 0.x.y` to synchronize versions across all packages, then commit and push the `v0.x.y` tag.
- **Custom Signing Keys**:
  1. Generate a Minisign keypair with `pnpm tauri signer generate`.
  2. Save the private key to GitHub Secrets as `TAURI_SIGNING_PRIVATE_KEY` (and optional `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`).
  3. Set the public key in `apps/desktop/src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.
