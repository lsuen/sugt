# AgentAdapter 能力矩阵设计

- 日期：2026-11-05
- 范围：SUGT「本机 Agent 接入 / 技能管理」链路
- 定位：先建立唯一事实模型，再让管理能力与 SOUL agent 能力共同消费它
- 作者：sun / 孙文龙

## 1. 为什么要做

用户提出的目标：发现本机 Agent → 复选框确认是否接入 → 方便管理 skill → 接入时找对 skill 目录 → 明确各 Agent「能写哪些配置」（每个 Agent 不一样）。

只读梳理后的事实是：**这条流程的骨架已经存在，但不在 SOUL，且缺少一层共享事实模型**。

| 已有 | 位置 |
| --- | --- |
| `discover_agents` + `add_discovered_agents` | `src-tauri/src/agent_discover.rs`、`commands.rs` |
| 复选框多选、低置信候选补目录 | `src/AgentDiscoverModal.tsx` |
| 声明式自定义接管模板 | `src-tauri/src/takeover_profiles.rs`（`takeover-profiles.json`） |
| 写环境变量、写 Codex 配置、注入自带 skill | `takeover_profiles.rs::apply_profile`、`codex_config.rs`、`sugt_skill.rs` |
| 技能仓库 / 挂载 / 列举 | `src-tauri/src/store/`、`src/SkillsPanel.tsx` |

| 缺口 | 事实 |
| --- | --- |
| SOUL 与链路完全隔离 | `me_agent.rs` 无工具注册表，ACL 只有 9 个 `me_*`，靠「模型回 bash 代码块 → 本机执行」工作 |
| skill 目录四套定义并存 | `agent_discover.rs` 指纹表、`takeover_profiles.rs::collect_skill_dirs`、`store/paths.rs`、`sugt_skill.rs::inject_targets_for_profile` |
| 目录可能凭空生成或被静默丢弃 | `sugt_skill.rs` 不过滤存在性直接 `create_dir_all`；`hub.rs::list_skill_agents` 用 `filter_map` 丢掉无目录的 profile |
| 没有「能写哪些配置」的声明 | `TakeoverProfile` 只有 `env_vars / clear_vars / settings_dirs / settings_path / skill_dirs`，无配置文件 schema |
| 除 Codex 外没有配置文件写入 | 6 个 Agent 只有通用 `OPENAI_*` 环境变量模板 |
| 写入不安全 | `codex_config.rs` 的 TOML 解析失败 `unwrap_or_default()` → 空表 → 整文件覆盖；无备份；释放时不还原 `model` / `preferred_auth_method` / `auth.json` |
| 静默兜底 | `clients.rs::install_for` 未知 target 会悄悄装成 Claude，`"all"` 也只装 Claude |

## 2. 目标与非目标

**目标**

1. 一处定义、四处消费：`agent_adapters.rs` 成为 Agent 元数据的唯一真相源。
2. 显式能力矩阵：每个 Agent 声明「SUGT 当前会写什么」，前端接入前可预览影响范围。
3. 写入可逆：备份 + 快照 + 原子写 + 释放时完整还原；解析失败绝不覆盖用户文件。
4. SOUL 只产出意图与解释，实际写入走确定性通道并保留用户确认。

**非目标（本轮不做）**

- 让 SOUL 直接写配置文件或扩大其 shell 权限。
- 为 6 个 Agent 各自实现官方配置文件的完整读写（仅声明能力，不虚报）。
- skill 的版本/来源/增量更新、首次启动主动弹框、LAN 与接入联动（列 P2）。

## 3. 核心判断

「管理能力优先、其次 agent 能力」成立，但机制不是「两者原理差不多」，而是**两者消费同一份事实模型**：

```
管理能力(客户端 Tab) ─┐
                      ├─→ 确定性执行层 ─→ AgentAdapter 事实模型
SOUL agent 能力 ──────┘   (发现→预览→确认→幂等写入+回滚)
```

管理能力的产出（准确元数据 + 安全写入器）是 agent 能力的**安全前提**；反向不成立。若先提升 SOUL 自主性，等于让模型用 bash 猜 `~/.codex/config.toml` 的结构，越自主破坏面越大。`me_agent.rs` 的 `auto_write` 可跳过确认且工作目录外不拦截，因此必须先收口写入通道。

## 4. 数据结构

`src-tauri/src/agent_adapters.rs`，纯声明 + 纯函数，不做 IO 写入：

```rust
pub struct AgentAdapter {
    pub id: &'static str,
    pub name: &'static str,
    pub commands: &'static [&'static str],       // PATH 探测
    pub settings_dirs: &'static [&'static str],  // 配置目录候选
    pub skill_dirs: &'static [&'static str],     // 个人级技能目录候选
    pub project_skill_dirs: &'static [&'static str], // 项目级（相对仓库根）
    pub config: ConfigCapability,
}

pub struct ConfigCapability {
    pub kind: ConfigKind,                        // UserEnv | CodexToml | None
    pub config_path: Option<&'static str>,       // 相对 home，仅声明可写目标
    pub writable_fields: &'static [&'static str],// SUGT 实际会写的字段
    pub clear_vars: &'static [&'static str],     // 接管前清除的冲突变量
    pub revertable: bool,                        // 释放时能否完整还原
    pub note: &'static str,                      // 面向用户的说明
}
```

`ConfigKind` 区分「SUGT 当前会写」与「该 Agent 官方可配」——不把探测到的配置文件虚报成可写能力。

## 5. 能力矩阵（SUGT 当前实际能力，非官方全集）

| id | 配置载体 | 会写的字段 | skill 目录（个人 / 项目） | 回滚 |
| --- | --- | --- | --- | --- |
| `claude-code` | 用户环境变量 | `ANTHROPIC_BASE_URL`、`ANTHROPIC_AUTH_TOKEN`；清 `ANTHROPIC_API_KEY`、`CLAUDE_CODE_API_KEY` | `~/.claude/skills` / `.claude/skills` | unset 变量 |
| `codex` | `~/.codex/config.toml` + `auth.json` | `model_provider`、`model`、`openai_base_url`、`preferred_auth_method`、`cli_auth_credentials_store`、`web_search`（仅原本缺失时注入）、`model_providers.sugt.{name,base_url,wire_api,requires_openai_auth,capabilities.{remote_compaction,external_web_access}}`、`auth.json.{auth_mode,OPENAI_API_KEY}`；**不写** `mcp_servers` | `~/.agents/skills` / `.agents/skills` | 快照还原 |
| `opencode` | 通用 env（配置文件仅探测不写） | `OPENAI_API_KEY`、`OPENAI_BASE_URL` | `~/.config/opencode/skills`、`~/.opencode/skills`、`~/.claude/skills`、`~/.agents/skills` / `.opencode/skills` | unset |
| `qwen-code` / `aider` / `gemini-cli` | 通用 env | `OPENAI_*` 变体 | `~/.agents/skills`（gemini 另 `~/.gemini/skills`）等 | unset |
| `cursor` / `vscode` | 通用 env | `OPENAI_API_KEY`、`OPENAI_BASE_URL` | `.cursor/skills`、`~/.cursor/skills`、`.agents/skills` 等 | unset |

> 外部资料（2026 年 Agent Skills 开放标准 / 各客户端文档）仅是参考：SKILL.md 格式已统一，但发现路径各家不同，且不同来源对 Codex 的 `~/.codex/skills` 与 `~/.agents/skills` 说法不一致。**落地以本机实际安装版本探测为准**，硬编码候选只能作为兜底，用户可用 `settings_path` / `skill_dirs` 覆盖。

## 6. 目录收敛映射

| 原定义处 | 改后 |
| --- | --- |
| `agent_discover.rs::fingerprints()` 的 `settings_dirs / skill_dirs` | 从 `agent_adapters::by_id()` 派生 |
| `takeover_profiles.rs::builtin_profiles()` 的 `skill_dirs` | 从 adapter 派生，不再手写路径 |
| `takeover_profiles.rs::collect_skill_dirs()` | 用户自定义目录优先 → adapter 候选 → 动态扫描（Claude 插件市场、dev-agents 容器） |
| `store/paths.rs::{claude,codex}_skills_dir()` | 委托 `agent_adapters::canonical_skill_dir(id)` |
| `sugt_skill.rs::inject_targets_for_profile()` | 复用 `collect_skill_dirs`（只取已存在目录 + 用户显式指定），不再凭空 `create_dir_all` 任意路径 |
| `hub.rs::list_skill_agents()` | 无目录时返回 `detected: false` 的条目而非丢弃，并给出 canonical 目录供用户补建 |

## 7. 写入安全协议

1. **解析失败即报错**：`toml::from_str` 不再 `unwrap_or_default()`，返回带路径的 Err，绝不覆盖。
2. **备份**：首次接管前把 `config.toml`、`auth.json` 原样复制到 `<CODEX_HOME>/sugt-backup/`（带时间戳），已备份不重复覆盖。
3. **快照**：写 `<CODEX_HOME>/sugt-takeover-state.json`，保存接管前后 `config.toml` / `auth.json` 全文，以及 SUGT 是否注入过 `web_search`；`None` 表示文件原本不存在。
4. **原子写**：写 `.tmp` 后 `rename`，避免半截文件。
5. **释放时完整还原**：按快照恢复 `model`、`preferred_auth_method`、`cli_auth_credentials_store`、`auth.json`，并删除 `model_providers.sugt`；用户后续自行修改过的字段不覆盖。
6. **技能目录替换**：`mount_to_dir` 改为「先拷到临时目录 → 成功后原子替换」，不再先删后拷。
7. **显式报错**：`install_for` 未知 target 返回 Err，不再静默装 Claude。

## 8. 接入影响预览契约

新增只读命令 `preview_takeover(profile_id) -> TakeoverPreview`：

```rust
pub struct TakeoverPreview {
    pub profile_id: String,
    pub env_names: Vec<String>,          // 将写入的环境变量名（值脱敏）
    pub env_cleared: Vec<String>,        // 将清除的冲突变量
    pub config_writes: Vec<ConfigWrite>, // { path, field, kind }
    pub skill_dirs: Vec<String>,         // 将注入/挂载的目录
    pub warnings: Vec<String>,           // 冲突、目录不存在、不可回滚等
    pub revertable: bool,
}
```

前端在「接入」按钮前展示该预览；发现弹框的复选框行显示能力摘要。`list_agent_capabilities()` 供 SOUL 与设置页共用。

## 9. SOUL 边界

新增的命令全部是**只读**：`list_agent_capabilities`、`preview_takeover`。SOUL 可以解释「接入 Claude 会写这 2 个变量」「Codex 会按能力矩阵声明写一组字段且可回滚」，并生成结构化计划；**实际写入仍由用户在 UI 点「接入」触发既有的 `apply_takeover_profile`**。ACL 只在白名单里增加这两个只读命令，不扩大 `me_*` 的 shell 权限。

## 10. 落地顺序与验证

| 阶段 | 内容 | 验证 |
| --- | --- | --- |
| P0-1 | `agent_adapters.rs` + 四处目录收敛 | `cargo test --lib`（新增一致性测试） |
| P0-2 | 写入安全（备份/快照/原子写/还原/显式报错） | Codex 往返测试：接管 → 释放 → 文件逐字节还原 |
| P0-3 | 能力矩阵 + `preview_takeover` + 前端预览 | `npm run check` + `npm run build` |
| P1 | SOUL 只读能力命令 + system_prompt 说明 | `cargo test` 全量 |
| 收口 | 文档同步、`npm test` / `npm run build` / `cargo test` 全量 | 测试报告追加验收章节 |

每阶段一个原子提交，中文提交信息；不删除用户文件、不修改用户真实 `~/.codex` 配置（测试使用临时 `CODEX_HOME`）。

## 11. 实际验收（2026-11-05）

| 阶段 | 状态 | 提交 | 实测 |
| --- | --- | --- | --- |
| 设计文档 | 完成 | `d1ed7a4` | — |
| P0-1 目录收敛 | 完成 | `8371e9a` | 新增 8 个一致性测试，`cargo test --lib` 181 passed |
| P0-2 写入安全 | 完成 | `52f8791` | 新增 6 个回归测试，`cargo test --lib` 186 passed |
| P0-3 能力矩阵与预览 | 完成 | `45de787` | `preview_takeover` / `list_agent_capabilities` 两个只读命令；前端行内预览 + 发现弹框能力摘要 |
| P1 SOUL 边界 | 完成（按第 9 节的保守定义） | `45de787` | 能力摘要注入 `system_prompt` 中英两版；**未新增 `me_*` 命令、未扩大 shell 权限** |
| 全链路 | 通过 | — | `npm test`、`npm run build`、`cargo test` 全量：lib 191 passed / 1 ignored，集成 8 + 5 passed |

**P1 的实现选择（与初版设想的差异）**：初版写的是「为 SOUL 增加只读命令」，实际落地改为**把能力矩阵注入提示词**。原因是 `me_agent.rs` 没有工具调用机制，模型只能产出 bash 代码块；给它注册 Tauri 命令它也无法调用，反而会误导。注入提示词后，SOUL 能准确解释各 Agent 的接入影响并明确拒绝自己改客户端配置，这是零权限扩大、真实可用的加强。

**本轮未做（P2）**：skill 版本/来源/增量更新、首次启动主动弹框（目前仍需用户主动点「发现」并输入 ≥2 字符）、LAN 开关与接管写入联动、为 6 个客户端实现各自官方配置文件的完整读写。能力矩阵对这些客户端明确标注为「仅写环境变量、配置文件仅探测不写」，不虚报。

**真实验证边界**：未执行 `tauri build` 完整打包、桌面 UI 手工交互、真实上游 LLM 请求；新增的两个只读命令未做端到端 UI 点击验证（仅类型检查与构建通过）。

## 12. 追加：Codex 工具协议兼容（同日第三轮）

用户实测 Codex 接管后纯文本对话正常、工具层全挂（`shell_command` aborted、`read_mcp_resource` 找不到 server、`get_project_overview` / `compress_output` unsupported call）。根因是「Responses → Chat 回落」只覆盖文本路径，叠加接管配置缺少第三方能力声明。本轮在处理协议转换的同时同步了能力矩阵：

1. **接管配置**：`model_providers.sugt` 增加 `requires_openai_auth = false` 与 `capabilities.{remote_compaction = "unsupported", external_web_access = false}`；用户原本没写 `web_search` 时注入 `disabled`，释放时按快照标志精确清理；`auth.json` 改为合并写入，保留原有 ChatGPT 登录态字段。
2. **MCP 边界**：`read_mcp_resource` / `list_mcp_resources` 是 Codex 客户端内置工具，只在配置了 `[mcp_servers]` 时可用。SUGT **不代管 MCP server**，接管与释放都不碰该表；接入预览与 SOUL 能力摘要已明确说明。
3. **能力矩阵**：`writable_fields` 同步新增上述字段；预览会自动列出新增写入项，并对 Codex 增加一条 MCP 归属告警。

协议层修复与完整验证见 `docs/TEST_REPORT.md` 第 13 节；端到端用例位于 `src-tauri/tests/codex_gateway_e2e.rs`。
