# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

SUGT（su gateway）是一个本地 AI 网关桌面应用。把多个 LLM 服务商的 API Key 集中管理，通过本地 OpenAI/Anthropic 兼容代理让 Claude Code、Codex、OpenCode 等客户端统一走本地网关访问上游模型服务。

- **产品形态**：Tauri 2.9 桌面应用（GUI）+ CLI（`sugt-cli`）
- **前端**：React 18 + TypeScript 5.7 + Vite 6
- **后端**：Rust (edition 2021) + Axum 0.7 HTTP 网关 + Tauri IPC commands
- **平台**：Windows 10/11 x64（WebView2 运行时必需）
- **配置目录**：`Documents\.sugt\`（`SUGT_CONFIG_DIR` 环境变量可覆盖）

## 构建与开发命令

```bash
# 前端开发（仅 Vite dev server）
npm run dev

# 完整 Tauri 桌面应用开发
npm run dev:app
# 商店版（含技能中控台）
npm run dev:app:store

# TypeScript 类型检查
npm run check

# Rust 单元测试
cd src-tauri && cargo test --lib

# 前端生产构建
npm run build

# Tauri 完整构建
npx tauri build --no-bundle
# CLI 独立构建
cd src-tauri && cargo build --release --bin sugt-cli

# 打包发布（PowerShell 脚本）
npm run package:release:self          # 自用版
npm run package:release:trial         # 试用版
npm run package:release:all           # 全部变体
```

## 产品线与授权体系

两个正交维度：

| 维度 | 取值 | 说明 |
|------|------|------|
| 产品线（编译期） | `feature` / `store` | 通过 `SUGT_PRODUCT` 环境变量控制；`store` 包含技能中控台 |
| 授权变体（运行时） | `dev` / `self` / `trial` | 由打包脚本注入的 `build_id` 和 `trial-state.json` 决定 |

- `product.rs` 定义产品线枚举，`trial.rs` 管理授权检查与试用到期
- 公开仓库不含试用注入脚本和技能中控台完整源码

## 核心架构

### 双二进制

- `sugt`（`src/main.rs`）：Tauri GUI 桌面应用，入口 `app::run()`，注册所有 Tauri commands，启动托盘、看门狗、技能仓预热
- `sugt-cli`（`src/bin/cli.rs`）：CLI 工具，支持 `init`、`status`、`serve`（独立网关）、`test`、`env`（接管管理）子命令

### 网关（Gateway）

`gateway.rs` 是核心模块，在 Tauri 进程内启动 Axum HTTP 服务器：

- **路由**：`/health`、`/v1/models`、`/v1/_sugt/traffic`、`/v1/messages`（Anthropic）、`/*path`（OpenAI 兼容通配）
- **请求流程**：`gateway_auth::validate_client_request` 验证 API Key → 按 `active_provider_id` 选择主模型 → 失败时按 `failover` 配置顺序 fallback 到下一个 provider
- **协议转换**：`anthropic_adapter.rs` 负责 Anthropic Messages → OpenAI Chat Completions 的请求/响应双向转换（含 SSE 流式）；`openai_responses_adapter.rs` 处理 OpenAI Responses API
- **配置热加载**：`maybe_reload_config()` 在每次请求时检查 `config.toml` 的 mtime 并自动重载
- **端口 fallback**：`gateway_listen::bind_with_fallback` 在端口被占用时自动尝试下一个端口

### 网关守护（Daemon + Watchdog）

- `gateway_daemon.rs`：通过 PID 文件管理独立 `sugt-cli serve` 进程的启停；`spawn_detached` 在 Windows 上用 `CREATE_NO_WINDOW | DETACHED_PROCESS` 创建无窗口后台进程
- `gateway_watchdog.rs`：每 20 秒检测网关可达性，不可达时自动拉起（先嵌入式启动，失败则走 detached 路径），并同步端口变化到配置和启动脚本

### 模型管理（model.rs + provider_catalog.rs）

- `AppConfig` 是顶层配置结构（TOML 序列化），包含 `providers` 列表、`active_provider_id`、`failover` 配置、网关设置等
- `ProviderConfig` 描述单个模型：协议（OpenAI/Anthropic）、Base URL、API Key、Model Name、启用/禁用状态
- `provider_catalog.rs` 内置国内服务商目录（魔搭、火山、硅基流动等），提供标准 Base URL 和模型列表接口

### Agent 接管（clients.rs + takeover_profiles.rs + agent_discover.rs）

- **接管机制**：通过设置环境变量（`ANTHROPIC_BASE_URL`、`OPENAI_BASE_URL` 等）和写入启动脚本（`.cmd` 文件），让 Claude Code/Codex 等客户端将请求指向本地网关
- `takeover_profiles.rs`：管理接管模板（内置 + 自定义），每个模板定义需要的环境变量、清理变量、settings 目录、skill 目录
- `agent_discover.rs`：扫描本机进程/PATH/环境变量发现已安装的 Agent 客户端
- `agent_advanced.rs`：高级 Agent 设置（自定义 settings 路径等）

### 技能中控台（store/ 模块，仅 store 产品线）

- `store/repos.rs`：Git 技能仓库管理（添加、刷新、GitHub 代理），通过 `git clone/pull` 同步技能索引
- `store/catalog.rs`：技能目录（staged/mounted 状态追踪），支持多 Agent 挂载
- `store/install.rs`：技能安装/卸载/挂载/卸载操作
- `store/hub.rs`：技能发现 API（Gitee/自定义仓库）
- `store/plugins.rs`：插件面板（Markdown 渲染）
- `store/settings.rs`：GitHub 代理等设置

### 前端（src/）

- `main.tsx`：主界面入口，包含 Tab 导航（仪表盘/模型/客户端/技能/设置/日志/关于），通过 `invoke()` 调用 Tauri commands
- 各面板组件独立：`ProviderModal.tsx`（模型配置）、`TakeoverProfilesSection.tsx`（接管管理）、`SkillsPanel.tsx`（技能面板）、`TrafficPanel.tsx`（流量统计）、`SettingsPanel.tsx`（设置）、`GatewayAccessBanner.tsx`（网关接入贴片）
- `store/` 子目录：技能中控台相关 UI 组件
- `providerPresets.ts`：内置服务商预设数据

### Tauri Commands 通信模式

GUI 通过 `invoke()` 调用 Rust 端注册的命令（`app.rs` 中 `generate_handler!` 列出全部命令）。`AppRuntime`（`commands.rs`）作为 Tauri managed state，持有 `GatewayState`、`AppPaths`、`AppConfig`（`Arc<RwLock<>>`）。所有命令函数签名遵循 `#[tauri::command]` 规范。

## 配置系统

- 配置文件：`config.toml`（TOML 格式），位于 `Documents\.sugt\`
- `model.rs` 中 `AppConfig` 为完整配置结构，`config.rs` 负责加载/保存/规范化
- 环境变量初始化：`SUGT_DEFAULT_BASE_URL`、`SUGT_DEFAULT_API_KEY`、`SUGT_DEFAULT_MODEL` 可在首次初始化时创建默认 provider
- 配置热加载：网关运行时修改 `config.toml` 后下次请求自动生效（无需重启）

## 关键依赖

- **Rust**：`axum 0.7`（HTTP 框架）、`reqwest 0.12`（HTTP 客户端，rustls-tls）、`tokio 1.41`（异步运行时）、`tauri 2.9`（桌面壳）、`tower-http`（CORS/追踪中间件）、`clap 4.5`（CLI 参数解析）
- **前端**：`react 18`、`@tauri-apps/api 2.9`、`lucide-react`（图标）、`vite 6`