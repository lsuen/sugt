# SUGT

SUGT（su gateway）是运行在本机的 AI 网关与 Agent 配套工具。它提供 OpenAI / Anthropic 兼容接入点，统一转发上游大模型请求，并为 Claude Code、OpenAI Codex、OpenCode 等客户端提供一键接管能力。完整发行版另含技能中控台（仓库刷新、技能发现与挂载）。

| 项 | 说明 |
| --- | --- |
| 产品名 | SUGT |
| 定位 | 本地 AI 网关 |
| 当前版本 | 1.0.0 |
| 作者 | 孙文龙 |
| 设计 | 异常设计 |
| 公开许可 | Apache License 2.0（功能版核心） |
| 闭源模块 | 技能中控台、试用绑定与定向发版注入 |

配置与日志默认目录：`Documents\.sugt\`（可通过环境变量 `SUGT_CONFIG_DIR` 覆盖）。国内服务商 Base URL / 模型约定见 [docs/provider-vendors.md](docs/provider-vendors.md)。开源边界见 [docs/open-source.md](docs/open-source.md)。从源码构建见 [docs/build.md](docs/build.md)。

## 能力概览

- 本地网关：监听本机端口，转发 OpenAI Chat Completions、Anthropic Messages，以及 Codex 使用的 OpenAI Responses API。
- 协议适配：Anthropic 与 OpenAI 互转；对支持原生 Responses 的上游优先透传，失败时回退 Chat Completions。
- 模型管理：常见服务商预设、启用/禁用、默认切换、Base URL 自适配、连接测试；可选实验性 OpenCode Zen 通道。
- Agent 接管：为 Claude Code / Codex / OpenCode 等写入指向本地网关的环境与配置；支持发现本机 Agent、管理 Skills / 插件 / 配置路径。
- 技能中控台（完整发行版）：仓库刷新、技能发现与入库、挂载到各 Agent 技能目录；可选 GitHub 克隆代理。源码不随公开仓发布。
- 桌面端 + CLI：Tauri GUI 负责日常配置与托盘运行；`sugt-cli` 提供状态、起停与环境安装命令。
- 试用治理（定向分发构建）：试用构建可绑定机器与截止日期；自用构建无有效期限制。

## 产品线与构建变体

产品线由编译变量 `SUGT_PRODUCT` 决定，与试用/自用变体正交：

| `SUGT_PRODUCT` | 说明 |
| --- | --- |
| `feature`（默认） | 功能版：网关、模型配置、客户端接管 |
| `store` | 全功能版：在功能版基础上包含技能中控台 |

打包脚本写入的变体：

| 变体 | 说明 |
| --- | --- |
| `trial` | 试用版，默认 90 天，首次运行绑定本机 |
| `self` | 自用版，无试用期限制 |

分支与发版约定见 [docs/BRANCHING.md](docs/BRANCHING.md)。

## 环境要求

- Windows 10 / 11 x64
- Node.js 18+
- Rust 1.78+（`cargo`、`tauri-cli`）
- Git for Windows（刷新技能仓库时需要）
- WebView2 Runtime（运行 GUI；Windows 10/11 通常已预装）

## 开发

```cmd
npm install
npm run dev:app              REM 功能版 GUI
npm run dev:app:store        REM 全功能版 GUI（SUGT_PRODUCT=store）
npm run check                REM TypeScript 检查
```

Rust 单元测试：

```cmd
cd src-tauri
cargo test --lib
```

说明：若环境变量 `SUGT_PRODUCT=store` 残留，库会按全功能版编译。功能版回归测试前请清除该变量。

GUI 须通过 `tauri build` 或 `npm run package:release*` 构建。仅执行 `cargo build --release --bin sugt` 可能导致前端资源未嵌入，界面无法正常加载。

## CLI

```cmd
sugt-cli status
sugt-cli serve --host 127.0.0.1 --port 8787
sugt-cli env install --start-gateway
```

## 打包与分发

完整树下的便携包输出到 `release/`（已加入 `.gitignore`）。作者侧规则见私有树 `scripts/RELEASE.md`。

常用命令（私有完整树）：

```cmd
npm run package:release:store:trial:zip    REM 全功能试用（90 天）+ ZIP
npm run package:release:store:self:zip     REM 全功能自用 + ZIP
npm run package:release:self:zip           REM 功能版自用 + ZIP
npm run package:release:trial:zip          REM 功能版试用（90 天）+ ZIP
npm run package:release:wizard             REM 交互式选择
```

示例产物目录：

```text
release/SUGT-1.0.0-store-trial-windows-x64/
├── SUGT.exe
├── sugt-cli.exe
├── SUGT.ico
├── README.txt
├── VERSION.txt
└── manifest.json
```

试用策略默认值见 `scripts/release-settings.json`（当前 `defaultTrialDays` 为 90）。

关于页展示试用截止说明，或「异常设计自用版，无有效期限制」。

## 技能中控台要点（完整发行版）

- 推荐仓库可由发行版预热（例如 Gitee 推荐库）。
- Anthropic 官方技能位于 `anthropics/skills`；国内网络建议配置 GitHub 克隆代理后再刷新。
- 安装目录：`Documents\.sugt\store\skills\`。
- 挂载目录：Claude 为 `~\.claude\skills`；Codex 为 `~\.agents\skills`。
- 插件区域提供只读列表与说明，不提供在线安装。

## 目录结构

```text
src/                  React 前端
src/store/            技能中控台 UI（不进入公开仓）
src-tauri/src/        Rust 后端与网关
src-tauri/src/store/  技能仓库、索引、安装与挂载（不进入公开仓）
scripts/              打包、导出与发布脚本
docs/                 产品与开源说明
.github/workflows/    CI 与功能版 Release 工作流
```

## 开源与许可

功能版核心以 Apache License 2.0 发布，署名异常设计。技能中控台与试用绑定相关实现为专有模块，完整安装包仍可由作者从私有完整树构建。说明见：

- [docs/open-source.md](docs/open-source.md)
- [docs/build.md](docs/build.md)
- [LICENSE](LICENSE)
- [NOTICE](NOTICE)
