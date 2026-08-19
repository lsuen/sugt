# SUTAI

朴素的 AI 时代中控台：一个本地运行的 AI 网关与技能管理桌面应用。集中管理多家 LLM 服务商的 API Key，通过本地 OpenAI / Anthropic 兼容代理，让 Claude Code、Codex、OpenCode 等 AI 编码客户端统一接入本地网关访问任意上游模型；同时提供可发现、可安装、可挂载的技能（Skills）中控台。

- **一套 Key，处处可用**：所有 AI 客户端的 Base URL 与鉴权，收拢到本机一个网关。
- **上游随意换**：官方 API、国内服务商（魔搭、火山引擎、硅基流动等）、自建模型，切换无需改动任何客户端。
- **技能不乱丢**：统一发现、安装、挂载，换机器也不重来。

| | |
| --- | --- |
| 版本 | 1.1.1 |
| 作者 | 孙文龙 · [异常设计](https://github.com/lsuen) |
| 开源 | [Apache-2.0](LICENSE)，全仓库开源 |
| 当前下载 | [GitHub Releases](https://github.com/lsuen/sugt/releases) |

English: [README.en.md](README.en.md)

## 版本与授权

| 版本 | 有效期 | 说明 |
| --- | --- | --- |
| 公开版 | Windows 半年（180 天） | 到期后提示前往 GitHub 更新软件，功能不限制；macOS 无时间限制 |
| 自用版 / 开发版 | 无限制 | 作者自用，不对外分发 |

公开版即完整功能版本：本地 AI 网关、协议互转、客户端接管、技能中控台全部可用。Windows 公开版在半年后周期性提示更新软件，前往 [GitHub Releases](https://github.com/lsuen/sugt/releases) 下载新版即可继续使用，不影响数据与配置。

## 为什么做这个

- **Key 一堆**：OpenAI、Anthropic、魔搭、火山……散落在各个客户端，泄漏、轮换、失效都难管理
- **Agent 一堆**：Claude Code、Codex、OpenCode、自写脚本，每个都要单独配置 Base URL 与鉴权
- **Skills 散养**：没有统一发现与挂载机制，换机器就得重新配置一遍

SUTAI 把这三件事收拢到一个本机小工具里：起一个兼容网关，一键把 Agent 指过来；用技能中控台统一发现、安装、挂载 Skills。

## 核心能力

### 本地 AI 网关（Gateway）

- 在 Tauri 进程内启动 Axum HTTP 服务器，兼容 **OpenAI Chat Completions**、**Anthropic Messages**、**OpenAI Responses** 三种协议
- **协议互转**：Anthropic 与 OpenAI 之间的请求 / 响应双向转换，含 SSE 流式
- **多上游与失败回退**：按 `failover` 配置顺序自动 fallback 到下一个服务商
- **配置热加载**：修改 `config.toml` 后下次请求自动生效，无需重启
- **端口自动回退**：端口被占用时自动尝试下一个可用端口
- **健康检查与流量统计**：`/health`、`/v1/models`、`/v1/_sugt/traffic` 等内置端点

### 模型管理

- 内置国内服务商目录（魔搭、火山引擎、硅基流动等），标准 Base URL 与模型列表开箱即用
- 多服务商管理：启用 / 禁用、默认模型切换、连通性测试

### Agent 一键接管

- 自动发现本机已安装的 Agent 客户端
- 一键设置环境变量与启动脚本，让 Claude Code / Codex / OpenCode 等客户端指向本地网关；取消接管时自动清理

### 技能中控台（Skills）

- 从 Git 仓库 / Gitee / 自定义源刷新技能索引
- 技能安装、卸载、挂载、卸载挂载，库始终保留，换机器不丢
- 支持多 Agent 挂载，Markdown 渲染技能面板

### 流量悬浮窗

- 置顶 + 半透明 + 鼠标穿透，实时显示今日 Token 交互
- 支持横竖排布局、尺寸调节与一键恢复默认

### 网关守护

- 看门狗每 20 秒检测网关可达性，不可达时自动拉起
- 独立 `sugt-cli serve` 进程管理（PID 文件 + 无窗口后台进程）

## 界面预览

控制台：启停、流量、接入点。

![控制台](docs/images/client.png)

模型：多服务商、默认切换、测连通。

![模型配置](docs/images/client-llm.png)

客户端：发现本机 Agent，一键接管 / 取消。

![客户端接管](docs/images/client-agents.png)

技能：本地挂载 + 仓库发现入库。

![本地技能](docs/images/client-skills-local.png)

![发现技能](docs/images/client-skills-store.png)

## 国际化（Internationalization）

SUTAI 内置 **中文 / English** 双语支持，所有界面文本均通过统一的字典层渲染，不依赖任何第三方 i18n 库（零额外依赖）。

**语言切换方式**：打开「设置」→「语言」，在「跟随系统 / 中文 / English」三者间选择：

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

前往 [GitHub Releases](https://github.com/lsuen/sugt/releases) 下载 Windows x64 安装包，安装即可运行。

配置默认位于 `Documents\.sugt\`，可通过环境变量 `SUGT_CONFIG_DIR` 覆盖。

### CLI

```cmd
sutai-cli status
sutai-cli serve --host 127.0.0.1 --port 8787
sutai-cli env        # 接管管理
sutai-cli test       # 连通性测试
```

## 从源码构建

需要：Windows 10/11、Node 18+、Rust 1.78+、WebView2 运行时。

```cmd
npm install
npm run check        # TypeScript 类型检查
npm run dev:app      # 启动 Tauri 桌面应用（开发模式）
```

生产构建：

```cmd
npm run build        # 前端生产构建
npx tauri build      # 完整桌面应用打包
cd src-tauri && cargo build --release --bin sugt-cli   # 独立 CLI
```

## 技术栈

| 层 | 选型 |
| --- | --- |
| 桌面壳 | Tauri 2.9（WebView2） |
| 前端 | React 18 + TypeScript 5.7 + Vite 6 |
| 后端 | Rust (edition 2021) + Axum 0.7 |
| 网关 | 内嵌 Axum HTTP 服务器，OpenAI / Anthropic 协议互转 |
| CLI | `sugt-cli`（独立二进制，支持 `serve` / `status` / `test` / `env` / `init`） |

## 反馈与参与

有问题或建议，欢迎开 [Issues](https://github.com/lsuen/sugt/issues)。项目持续打磨中，Star 是对作者最大的鼓励。

## License

[Apache-2.0](LICENSE) · [NOTICE](NOTICE)
