# SUGT 开源策略

| 项 | 说明 |
| --- | --- |
| 设计署名 | 异常设计 |
| 作者 | 孙文龙 |
| 模式 | Open Core |
| 公开许可 | Apache License 2.0 |
| 闭源范围 | 技能中控台（商店产品线）、试用绑定与定向发版注入 |
| 架构表述 | 模块化单体；不是微内核 |

## 1. 目标

在保持本机全功能安装包可继续由作者构建与分发的前提下，将网关与 Agent 接管相关能力以源码形式公开，接受外部审阅与贡献。技能发现、仓库刷新、技能入库与挂载所构成的技能中控台，以及试用机绑与发版元数据注入，暂不进入公开仓库。

公开源码与商业/定向分发包正交：

- 公开仓库：可构建功能版（feature）源码树。
- 私有完整树（本仓库 / Codeup）：可构建全功能版（store）与试用/自用变体。

## 2. 许可选择

采用 **Apache License 2.0**。

选择理由：

1. 对网关类基础设施常见，法务审阅成本低。
2. 含明确专利授权与诉讼终止条款，适合可能被企业二次集成的组件。
3. 允许闭源产品在遵守 NOTICE 与归属要求的前提下链接或派生使用公开部分。
4. 与 Open Core 常见实践一致：公开核心 Apache-2.0，增值模块保留专有权。

不采用 GPL/AGPL：避免将桌面端与本地代理链路强制传染为强 copyleft，降低企业试用与贡献门槛。  
不采用仅 Source-Available（如 BSL）作为首发公开许可：首发需要清晰的 OSI 认可许可，便于 GitHub 检索与依赖扫描。

闭源模块不适用 Apache-2.0，由异常设计保留全部权利；二进制分发条款由发版说明单独约定。

## 3. 双仓与产品线

| 维度 | 私有完整树 | 公开树 |
| --- | --- | --- |
| 当前远端 | Codeup `sugt`（origin） | GitHub 公开仓（导出同步） |
| 分支 | `dev-feature` / `dev-store` / `master` | 仅同步功能版可公开子集，默认 `main` |
| `SUGT_PRODUCT=feature` | 可构建 | 目标构建形态 |
| `SUGT_PRODUCT=store` | 可构建全功能包 | 不提供对应源码 |
| 试用 / 自用打包 | 本地 `scripts/package-release*.ps1` | 不公开试用绑定实现细节与注入流水线 |

分支约定见 [BRANCHING.md](./BRANCHING.md)。公开同步清单见 [public-manifest.md](./public-manifest.md)。

## 4. 公开与不公开

### 4.1 公开（功能版核心）

- 本地网关：OpenAI Chat Completions、Anthropic Messages、OpenAI Responses 适配与回退。
- 模型与服务商配置、连接测试、故障转移相关逻辑。
- 客户端环境安装与一键接管（Claude Code / Codex / OpenCode 等）。
- Agent 发现与高级路径管理中属于底座、且不依赖商店目录实现的部分。
- 功能版桌面 UI 与 `sugt-cli`。
- 面向使用者的说明：`README.md`、`docs/provider-vendors.md`、`docs/build.md`、`docs/open-source.md`。
- CI 工作流（类型检查 / 基础测试），不含商店专用密钥与发版注入脚本。

### 4.2 不公开

- 技能中控台：`src/store/`、`src/SkillsPanel.tsx`、`src-tauri/src/store/` 及仅服务于该能力的命令与预热逻辑。
- 商店版信息架构与历史 PRD：`docs/PRD-*.md`、内部功能点/实现备忘（含中文版历史文档）。
- 试用机绑与状态文件协议实现中的防滥用细节（公开树若需可运行，应降级为开发态/无绑定构建）。
- 本地发版通知载荷、钉钉密钥、个人访问令牌、`.secrets/`、`.env*.local`。
- `release/` 二进制产物、调试备忘、备份目录、AI 会话缓存。

「技能中控台」即此前口头所称的技能商店 / 技能控制台：仓库管理、目录索引、安装暂存、向各 Agent 挂载、商店设置与相关 UI。公开叙事可写「支持在完整发行版中管理与挂载技能」，但实现源码不进入公开仓。

## 5. 发版与 Release

| 产物 | 构建位置 | 开源公开 Release | 私有仓定向分发 |
| --- | --- | --- | --- |
| 功能版 portable / zip | 公开树或私有完整树 | 允许 | 允许 |
| 全功能版 / 试用版 zip | 仅私有完整树 | 不允许 | 允许（仓库保持 private 时） |

本地打包入口见 [../scripts/RELEASE.md](../scripts/RELEASE.md)（仅私有完整树）。  
公开树只提供构建说明：[build.md](./build.md)。

当前 GitHub `lsuen/sugt` 为私有仓时，可将全功能试用包挂到该仓 Releases，供定向下载。仓库改为 Public 后，试用包须迁到单独的私有分发渠道。

导出公开树（仅在本机完整树执行）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/export-public-tree.ps1
```

默认输出：`dist-public/sugt/`。同步清单见私有树内 `docs/public-manifest.md`（该文件本身不导出）。

## 6. 已知耦合与后续拆分

当前完整树中，部分 Agent 高级命令仍挂在 `store::commands` 下注册，设置页也存在对商店设置接口的调用。公开导出在剔除商店目录后，**不能**直接等价于可编译的功能版，需完成编译期裁剪（Cargo feature / 前端条件编译）或提供公开侧桩模块。

阶段划分：

1. 策略与清单落地（本文档、私有树 `public-manifest.md`、导出脚本、LICENSE/NOTICE、密钥隔离）。
2. 代码边界硬化：商店实现改为可选 feature；功能版默认关闭；公开树可独立通过 `npm run check` 与 `cargo test`。
3. 首次公开推送与功能版 GitHub Release 流程固化。

在阶段 2 完成前，不以「可从公开源码一键复现功能版二进制」对外宣传。

## 7. 安全与凭证

- GitHub Personal Access Token 仅存放于本机 `.secrets/github.token`，不得提交。
- 禁止在仓库根目录使用名为 `.github` 的**文件**存放密钥；`.github/` 目录仅用于工作流与社区模板。
- 推送含 Actions 工作流的提交时，classic PAT 需要具备 `repo` 与 `workflow` 权限。
- 若令牌曾写入聊天记录、截图或误提交历史，应在 GitHub 立即轮换并作废旧令牌。
- 令牌权限应按需最小化；完成发版后可收回 `admin:*` 等与本仓库无关的宽权限。
