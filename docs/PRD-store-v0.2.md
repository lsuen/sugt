# SUGT 商店版 PRD（v0.2.0 查看版）

> 状态：草案 · 产品线：`store` · 目标版本：**0.2.0**  
> 关联文档：[分支与版本管理](./BRANCHING.md)

## 1. 背景与目标

SUGT 功能版（`feature`）已具备本地网关、模型配置、Claude/Codex 接管等能力。商店版（`store`）在**同一网关底座**上，面向 **Claude Code 新手**，提供类似 HMCL / Pal 系工具的 **清单 + 启动 companion** 体验：先看清楚 env / 插件 / skill，再逐步支持安装。

**v0.2.0 目标（MVP）**：**只读查看**，不做在线下载与安装。

| 维度 | 功能版 `feature` | 商店版 `store` |
| --- | --- | --- |
| 定位 | 内部网关 + 接管工具 | Claude 新手 companion + 网关 |
| 首版重点 | 持续 polish 网关/接管 | Claude env / 插件 / skill **展示** |
| Codex | 完整接管 | Tab 占位或仅 env 只读（可后置） |
| 安装 | — | UI 预留，逻辑不做 |

## 2. 用户与场景

- **主要用户**：会用 Claude Code CLI、但对 env、插件、skill 路径不熟悉的研发/内部同学。
- **核心场景**：
  1. 打开 SUGT → 客户端 → 点 **Claude** 大标签；
  2. 查看当前系统 env（无接管时看推荐说明，有则看实际值）；
  3. 查看已安装插件、skill 列表；
  4. 对某个 skill 点「用编辑器打开」（编辑器可在设置里配置）。

## 3. 信息架构

```
客户端 Tab
├── [Claude]  ← 商店版主面板（v0.2.0）
│   ├── 环境变量（推荐 / 当前 / 冲突提示）
│   ├── 插件（只读列表）
│   └── Skills（只读列表 + 打开编辑器）
├── [Codex]   ← 占位；v0.2.x 可仅 env 只读
└── 接管操作区（与功能版共用：检查 / 接管 / 修复 / 关闭）
```

交互原则：

- **不替代** Claude Code 内置 Marketplace；未来插件安装走 **官方 CLI/机制**。
- **Skill 源**未来接自建 registry；与插件协议分离。
- 所有「安装 / 更新」按钮 v0.2.0 **disabled**，tooltip 写「后续版本支持」。

## 4. 功能需求（v0.2.0）

### 4.1 环境变量

| ID | 需求 | 优先级 |
| --- | --- | --- |
| ENV-1 | 展示 Claude 相关变量：当前系统值（脱敏） | P0 |
| ENV-2 | 未接管时展示「推荐配置说明」，不伪造已安装状态 | P0 |
| ENV-3 | 与 SUGT 接管目标值对比，标出 missing / 冲突 | P0 |
| ENV-4 | 复用现有 `clients` 扫描逻辑，不重复造轮子 | P0 |

### 4.2 插件（只读）

| ID | 需求 | 优先级 |
| --- | --- | --- |
| PLG-1 | 扫描 Claude Code 官方认可的插件/marketplace 布局（见 §6） | P0 |
| PLG-2 | 列表展示：名称、来源、路径（可折叠） | P0 |
| PLG-3 | 空态：说明如何安装及「在线安装即将推出」 | P1 |
| PLG-4 | 预留「安装 / 更新」入口（disabled） | P1 |

### 4.3 Skills（只读 + 编辑）

| ID | 需求 | 优先级 |
| --- | --- | --- |
| SK-1 | 扫描用户级 skill 目录（见 §6） | P0 |
| SK-2 | 列表展示：名称、路径、简要描述（若有） | P0 |
| SK-3 | 「用编辑器打开」：调用系统默认或用户配置的编辑器 | P0 |
| SK-4 | 设置项：编辑器路径（如 `code`、`cursor`、`notepad++` 等） | P0 |
| SK-5 | 预留「从源安装」（disabled） | P1 |

### 4.4 Codex

| ID | 需求 | 优先级 |
| --- | --- | --- |
| CDX-1 | Tab 可见，内容为「即将支持」或仅 env 只读 | P2 |
| CDX-2 | 不做插件/skill | — |

### 4.5 非目标（v0.2.0 明确不做）

- 在线下载、安装、更新插件或 skill
- 替代 Claude Code GUI / IDE
- 全磁盘 / 多项目自动发现（可留 0.2.x）
- Codex 插件生态

## 5. 未来版本路线图

| 版本 | 商店版内容 |
| --- | --- |
| **0.2.0** | Claude：env + 插件/skill 只读；skill 编辑器 |
| **0.2.x** | 扫描范围扩展、Codex env 只读、体验 polish |
| **0.3.0** | Skill 接源 + 下载安装；插件对接官方安装命令 |
| **0.3+ / 1.0** | 安装闭环、与网关/接管深度联动 |

功能版（`feature`）可并行发 **0.1.x / 0.2.0**，与商店版 **版本号可同号不同产物**（靠 `SUGT_PRODUCT` 区分）。

## 6. 技术约定（待实现前锁定）

### 6.1 扫描路径（Windows，首版）

> 实现前需对照当前 Claude Code 版本文档做一次路径确认。

| 类型 | 建议路径（用户级） |
| --- | --- |
| Claude 配置 | `%USERPROFILE%\.claude\` |
| Skills | `%USERPROFILE%\.claude\skills\` 或项目 `.claude/skills`（v0.2.0 仅用户级） |
| 插件 / Marketplace | 以 Claude Code 官方目录为准，只读解析 |

### 6.2 后端 Command（预留命名）

```
list_claude_env_detail
list_claude_plugins
list_claude_skills
open_skill_in_editor { path, editor? }
get_store_settings / set_store_settings  # 编辑器路径等
```

### 6.3 权限

- 读用户目录：Tauri capability scope 白名单
- 打开编辑器：`shell` scoped 或 `open` 指定可执行文件
- 商店版单独 ACL 条目，功能版编译时不暴露 store 命令（`#[cfg(product_store)]` 或 runtime product 检查）

## 7. 验收标准（v0.2.0）

1. 商店版包（`SUGT-*-store-*`）关于页显示「商店版」。
2. 客户端 Tab 可切换 Claude 面板，展示 env / 插件 / skill 三块（空态正确）。
3. 至少一个 skill 可配置编辑器并成功打开。
4. 功能版包行为与 v0.1.2 一致，无商店 UI 回归。
5. `dev-feature` / `dev-store` 分支与打包脚本可按文档构建两种产物。

## 8. 风险

| 风险 | 缓解 |
| --- | --- |
| Claude CLI 目录随版本变化 | 文档化支持版本；解析失败友好降级 |
| 双分支代码分叉 | 共用 Rust 核心；商店 UI 模块化；定期 merge feature → store |
| 权限扩大 | 只读路径白名单；安装阶段再开 shell |

---

*文档维护：商店版需求变更请同步更新本 PRD 与 [BRANCHING.md](./BRANCHING.md)。*
