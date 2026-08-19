# SUTAI

朴素的 AI 时代中控台：一个本地运行的 AI 网关与技能管理桌面应用。集中管理多个 LLM 服务商的 API Key，通过本地 OpenAI / Anthropic 兼容代理，让 Claude Code、Codex、OpenCode 等 AI 编码客户端统一接入本地网关访问任意上游模型；同时提供可发现、可安装、可挂载的技能（Skills）中控台。

- **一套 Key，处处可用**：所有 AI 客户端的 Base URL 与鉴权，收拢到本机一个网关。
- **上游随意换**：官方 API、国内服务商（魔搭、火山引擎、硅基流动等）、自建模型，切换无需改动任何客户端。
- **技能不乱丢**：统一发现、安装、挂载，换机器也不重来。

| | |
| --- | --- |
| 版本 | 1.1.1 |
| 作者 | 孙文龙 · [异常设计](https://github.com/lsuen) |
| 开源 | [Apache-2.0](LICENSE)，全仓库开源 |
| 当前下载 | [GitHub Releases](https://github.com/lsuen/sugt/releases) |

English: [README.md](README.md)

## 版本与授权

| 版本 | 有效期 | 说明 |
| --- | --- | --- |
| 公开版 | Windows 半年（180 天） | 到期后提示前往 GitHub 更新软件，功能不限制；macOS 无时间限制 |
| 自用版 / 开发版 | 无限制 | 作者专用，不对外分发 |

公开版即完整功能版本：本地 AI 网关、协议互转、客户端接管、技能中控台全部可用。Windows 半年内会周期性提示更新，前往 [GitHub Releases](https://github.com/lsuen/sugt/releases) 下载新版即可继续使用，不影响任何配置与数据。

## 为什么需要

- **Key 分散**：OpenAI、Anthropic、魔搭、火山引擎、本地模型散落在各客户端，泄漏后只能逐一失效，难以管控。
- **Agent 一个接一个**：Claude Code、Codex、OpenCode 各自都要写 Base URL 和鉴权，每次配一套，繁琐易错。
- **Skills 散落**：没有统一发现/挂载/保留机制，换机器就要全部重来。

SUTAI 刻意保持小巧：一个兼容的本地网关、常用 Agent 一键接管，以及一个能真正留住不丢的技能中控台。

## 功能特性

### 本地 AI 网关（Gateway）

- 在 Tauri 进程内启动 Axum HTTP 服务器，同时支持 **OpenAI Chat Completions**、**Anthropic Messages**、**OpenAI Responses** 三种协议
- **协议互转**：Anthropic 与 OpenAI 之间的请求 / 响应双向转换，含 SSE 流式
- **多上游故障切换**：按 `failover` 配置顺序自动回退到下一个服务商
- **配置热加载**：修改 `config.toml` 后下一次请求即生效，无需重启
- **端口兜底**：端口被占用时自动绑定下一个可用端口
- 内置路由：`/health`、`/v1/models`、`/v1/_sugt/traffic`

### 模型管理

- 内置国内服务商目录（魔搭、火山引擎、硅基流动等），提供标准 Base URL 与模型列表
- 启停服务商、切换默认模型、一键测连通性

### 客户端一键接管

- 自动发现本机已安装的 Agent 客户端
- 通过环境变量与启动脚本，让 Claude Code / Codex / OpenCode 指向本地网关；解除接管时自动清理干净

### 技能中控台

- 从 Git 仓库、Gitee、自定义源刷新技能索引
- 安装、卸载、挂载、卸载技能——技能库永远保留
- 同一技能可挂载到多个 Agent；Markdown 渲染的技能面板

### 流量悬浮窗

- 置顶 + 半透明 + 鼠标穿透，实时显示今日 Token 交互
- 支持横竖排布局、尺寸调节与一键恢复默认

### 网关守护

- 看门狗每 20 秒检测网关可达性，不可达时自动拉起
- 独立 `sugt-cli serve` 进程管理（PID 文件 + 无窗口后台进程）

## 界面预览

控制台：启停、流量、接入点。

![控制台](docs/images/client.png)

模型：多服务商、默认切换、测连通性。

![模型配置](docs/images/client-llm.png)

客户端：发现本机 Agent，一键接管 / 取消接管。

![客户端接管](docs/images/client-agents.png)

技能：本地挂载 + 仓库发现入库。

![本地技能](docs/images/client-skills-local.png)

![发现技能](docs/images/client-skills-store.png)

## 国际化（Internationalization）

SUTAI 内置 **中文 / English** 双语支持，所有界面文本均通过统一的字典层渲染，不依赖任何第三方 i18n 库（零额外依赖）。

**语言切换方式**：打开「设置」→「语言」，在「跟随系统 / 中文 / English」三者间选择。

| 选项 | 行为 |
| --- | --- |
| 跟随系统 | 检测系统语言，中文系统显示中文，其余显示英文（默认） |
| 中文 | 固定显示中文 |
| English | 固定显示英文 |

切换后立即刷新界面，所有面板、弹窗、提示、悬浮窗文字实时生效，无遗漏。选择会持久化到本地，重启后保持。

**实现要点**：

- 文案集中在 `src/i18n.ts` 的 `zh` / `en` 两套字典中，组件内通过 `t('namespace.key')` 取值，便于统一维护与扩展
- 支持 `{变量}` 占位插值，动态文本（如 Token 数、服务商名、路径）可安全拼装
- 语言偏好存储在 `localStorage`，与主题等界面偏好一致
- 新增语言只需扩展字典即可，无组件级改动

## 快速开始

### 下载安装

**Windows**：前往 [GitHub Releases](https://github.com/lsuen/sugt/releases) 下载 Windows x64 安装包，安装即可运行。

**macOS（Homebrew）**：已发布预编译 Homebrew cask：

```bash
brew tap lsuen/homebrew-sugt
brew trust lsuen/sugt        # 一次性：信任本开源 tap
brew install --cask sugt
```

> 应用未签名。首次启动时在 Finder 中右键 SUTAI.app 选择「打开」，或执行 `xattr -cr "/Applications/SUTAI.app"`。

配置默认位于 `Documents\.sugt\`，可通过环境变量 `SUGT_CONFIG_DIR` 覆盖。

### CLI

```bash
sugt-cli status
sugt-cli serve --host 127.0.0.1 --port 8787
sugt-cli env        # 接管管理
sugt-cli test       # 连通性测试
```

> macOS 上 CLI 内置于应用包：`SUTAI.app/Contents/MacOS/sugt-cli`。

## 从源码构建

需要：Windows 10/11、Node 18+、Rust 1.78+。macOS 需要 Xcode Command Line Tools 与 Node。

```bash
npm install
npm run check        # TypeScript 类型检查
npm run dev:app      # 启动 Tauri 桌面应用（开发模式）
```

生产构建：

```bash
npm run build        # 前端生产构建
npx tauri build      # 完整桌面应用打包
cd src-tauri && cargo build --release --bin sugt-cli   # 独立 CLI
```

## 技术栈

| 层 | 选型 |
| --- | --- |
| 桌面壳 | Tauri 2.9（Windows 用 WebView2 / macOS 用 WKWebView） |
| 前端 | React 18 + TypeScript 5.7 + Vite 6 |
| 后端 | Rust (edition 2021) + Axum 0.7 |
| 网关 | 内嵌 Axum HTTP 服务器，OpenAI / Anthropic 协议互转 |
| CLI | `sugt-cli`（独立二进制，支持 `serve` / `status` / `test` / `env` / `init`） |

## 反馈与参与

有问题或建议，欢迎开 [Issues](https://github.com/lsuen/sugt/issues)。项目持续打磨中，Star 是对作者最大的鼓励。

## License

[Apache-2.0](LICENSE) · [NOTICE](NOTICE)