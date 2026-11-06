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
| `codex` | `~/.codex/config.toml` + `auth.json` | `model_provider`、`model`（`sugt-` 别名，可用 `SUGT_CODEX_MODEL_ALIAS=off` 关闭）、`openai_base_url`、`preferred_auth_method`、`cli_auth_credentials_store`、`web_search`（仅原本缺失时注入）、`model_providers.sugt.{name,base_url,wire_api,requires_openai_auth,capabilities.{remote_compaction,external_web_access}}`、`auth.json.{auth_mode,OPENAI_API_KEY}`；**不写** `mcp_servers`、`model_catalog_json` | `~/.agents/skills` / `.agents/skills` | 快照还原 |
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

**真实验证边界**：当时未执行 `tauri build` 完整打包（第四轮已补 `--no-bundle` 编译，见 `docs/TEST_REPORT.md` 14.6）、桌面 UI 手工交互、真实上游 LLM 请求；新增的两个只读命令未做端到端 UI 点击验证（仅类型检查与构建通过）。

## 12. 追加：Codex 工具协议兼容（同日第三轮）

用户实测 Codex 接管后纯文本对话正常、工具层全挂（`shell_command` aborted、`read_mcp_resource` 找不到 server、`get_project_overview` / `compress_output` unsupported call）。根因是「Responses → Chat 回落」只覆盖文本路径，叠加接管配置缺少第三方能力声明。本轮在处理协议转换的同时同步了能力矩阵：

1. **接管配置**：`model_providers.sugt` 增加 `requires_openai_auth = false` 与 `capabilities.{remote_compaction = "unsupported", external_web_access = false}`；用户原本没写 `web_search` 时注入 `disabled`，释放时按快照标志精确清理；`auth.json` 改为合并写入，保留原有 ChatGPT 登录态字段。
2. **MCP 边界**：`read_mcp_resource` / `list_mcp_resources` 是 Codex 客户端内置工具，只在配置了 `[mcp_servers]` 时可用。SUGT **不代管 MCP server**，接管与释放都不碰该表；接入预览与 SOUL 能力摘要已明确说明。
3. **能力矩阵**：`writable_fields` 同步新增上述字段；预览会自动列出新增写入项，并对 Codex 增加一条 MCP 归属告警。

协议层修复与完整验证见 `docs/TEST_REPORT.md` 第 13 节；端到端用例位于 `src-tauri/tests/codex_gateway_e2e.rs`。

## 13. 追加：模型别名与可观测降级（同日第四轮）

第三轮遗留的 `code_mode_only` 风险，公开资料给出的两种做法分别是「导出真实模型目录后改 `use_responses_lite` / `tool_mode`」与「给模型换一个不与官方同名的 slug」。前者需要 `model_catalog_json` 指向完整目录：凭空生成一份只含单个模型的 catalog 会**替换客户端整个内置模型目录**，风险大于收益，因此不采用。

1. **别名 fallback**：`codex_config.rs` 把 `model` 写成 `sugt-<原模型名>`。网关侧本来就用 `upstream_model` 覆盖请求里的 `model`，所以别名只影响客户端看到的 slug，不改变上游路由。别名幂等，且可通过 `SUGT_CODEX_MODEL_ALIAS=off` 关闭——各 Codex 版本对未知 slug 的处理不完全一致，必须留出一行回退口。
2. **可观测降级**：请求中无法透传给 Chat 上游的 Responses 字段（`previous_response_id`、`store`、`reasoning`、`include` 等）写入 `ChatToolMeta.dropped_request_fields` 并打日志，真机抓包时能直接看到「客户端要了什么、网关降级了什么」。`max_output_tokens` 则真正映射成 `max_tokens`，不再被静默丢弃。
3. **预览与 SOUL 同步**：接入预览新增一条别名告警，中英能力摘要同步说明别名用途与关闭方式；`agent_adapters.rs` 的 `note` 标注别名行为。
4. **未做**：不生成 `model_catalog_json`、不改写客户端内置模型目录、不实现服务端会话存储（`previous_response_id` 仍依赖客户端重放完整 `input`）。若真机复测仍见 `get_project_overview` / `compress_output` unsupported call，按公开方案导出真实目录并把 `use_responses_lite` 改为 `false` 是下一步，而不是继续猜测字段。

协议层异常分片兜底与验证见 `docs/TEST_REPORT.md` 第 14 节。

## 14. 追加：1.1.8 能力进化（2026-11-06）

本轮回答三个产品问题：**日志/关于是否还占主 Tab**、**设置是否该拆横向子 Tab**、**SOUL 要不要有技能栏**。前两者照做，第三者按架构红线改为「只读编排层」，不把写操作交给模型。

### 14.1 导航与设置重组（纯前端，后端零改动）

主 Tab 9 → 7：日志、关于移入设置，侧栏底部保留两个小号直达入口（不占主导航）。设置拆为 6 个横向子 Tab：`overview / general / appearance / advanced / logs / about`，**概览放首位**，聚合服务状态、LAN 是否开启与 hosts/base_url/key/model、当前上游、端点、版本构建号与快捷跳转（只读，新增数据全部来自已有 `get_status` / `get_lan_share_info`）。`.settings-tabs` 用 `position: sticky; top: 0` 冻结在 `.tab-body` 首行。

三个必须同步的耦合点已处理：日志高频轮询从 `tab==='logs'` 改为 `tab==='settings' && settingsTab==='logs'`（抽屉仍由 `StatusBar` 自己的定时器刷新，不重复轮询）；`StatusBar` 的「打开日志页」改为跳设置-日志子页；About 的 10 连击版本入口通过 `onVersionCardClick` 下传。子页状态提升到 `App`（`settingsTab` 受控），`SettingsPanel` 未传时退回内部 state 作为 browser-preview fallback。项目原本没有主 Tab 的 localStorage 持久化，因此无迁移兼容问题。

### 14.2 国内外 Agent 目录补全（单一真相源）

`agent_adapters.rs::ADAPTERS` 从 8 扩到 22：新增 CodeBuddy、通义灵码、Trae、iFlow、Kimi CLI、Qoder、Comate、CodeGeeX、Windsurf、Cline、GitHub Copilot CLI、Kiro、Goose、Factory Droid。未确认官方 skill 目录的一律走 `GENERIC_CAPABILITY`（只声明写环境变量 + note 说明），且候选目录必含跨 Agent 事实标准 `~/.agents/skills` 兜底，**不虚报配置写入能力**。

`agent_discover::known_agents()` 直接从 `agent_adapters::all()` 派生完整目录，`fingerprints()` 降级为「旧客户端的探测细节」，不再维护第二份客户端清单。空查询 / 单字符不再返回空列表，而是列出全部已知客户端（本机有痕迹 → high 置信，无痕迹 → low 但 `requires_path=false`，允许用户主动接管）；带查询词时也会把能力矩阵里新增的客户端按名称/厂商/命令补入结果。**这解决了「发现不了就无从接管」的问题**：自动接管现在先有完整目录可挑。

### 14.3 GitHub 地址安装 skill（确定性 + 事务回滚）

新增 `store/github_install.rs` 与 `store_install_git_skills` 命令，复用既有链路、零新依赖：`parse_git_url` → host 白名单（GitHub/Gitee/GitLab/Bitbucket/GitCode + owner/repo 字符校验）→ `git clone --depth 1 --single-branch`（复用 `github_proxy_prefix`，`main` 失败回落 `master`）→ 扫 `SKILL.md`（支持根目录自身就是一个 skill）→ `replace_dir` 原子落地 → 写 `staged.json`。

- 不把仓库登记进 `repos.toml`，不污染用户仓库列表；临时 clone 目录用完即删。
- 单次上限 60 个技能；失败时还原 `staged.json` 并删除本次已落地的目录，**不留半安装状态**。
- 幂等：同地址重复安装覆盖同一批 `git__<owner-repo>__<path>` id，不在列表堆积。
- 装完在「技能 → 本地技能」出现（该列表本来就读 `staged.json`），前端给出引导提示：勾选并挂载到目标 Agent。

### 14.4 触发词与优化建议（规则化，LLM 只做可选解释）

`catalog::parse_skill_meta()` 解析 frontmatter 的 `triggers` / `trigger` / `触发词` / `when_to_use` / `keywords`（逗号、顿号或 YAML `- item` 列表），解析不到就返回空并给可操作建议，**绝不凭空编造触发词**。`LocalSkillView` 新增 `triggers` / `suggestions`：列表显示触发词 chips，点「优化建议」打开 ModalPortal 弹窗看完整建议 + 一键打开技能目录。建议是纯规则（缺 frontmatter、description 过短、无触发词、无 `##` 分节），模型不可用时功能完全可用。新建本地技能的模板也补了 frontmatter / triggers / 分节骨架。

### 14.5 SOUL 只读编排层（红线不变）

新增 `soul_context.rs`：`soul_brief()` 把能力矩阵、已接管客户端、已装技能、仓库目录注入 `system_prompt_with_brief()`（保留原 `system_prompt` 签名兼容单测）。模型对管理类意图只输出一行 `[[SUGT_ROUTE]]<clients|skills|preview|settings>[[/SUGT_ROUTE]]`，前端 `MePanel` 剥离标记并渲染成建议卡片按钮，点击只做页面跳转；接管 / 安装 / 挂载仍由用户点按钮走既有确定性命令 + `TakeoverPreviewModal` 后果预览 + 确认。

**明确没有做**：不给 SOUL 工具调用循环、不扩大 shell 权限、不让模型吐 bash 写配置或下载仓库。理由见第 2 节非目标与 `AGENTS.md`：那样会绕过备份 → 快照 → TOML 校验 → 原子写 → 后果预览 → 确认 → 释放逐字段还原整条保护链，且弱模型/模型不可用时没有可降级路径。LLM 挂掉时所有功能仍可用，只是少了意图路由与解释。

fallback：未知/半截路由标记一律剥离、绝不执行；无 `onRoute`（浏览器预览）时只剥离不渲染按钮；`soul_brief` 读目录失败降级为「暂无」，不阻断对话。

### 14.6 边界与未做

- 本节写成时版本号仍为 1.1.7、打包脚本与 tag 未动；1.1.8 打版时已统一 6 处版本号并改为「固定资产名 + 自动打包」发布流程，详见 `AGENTS.md` 的 Release & packaging 与 `docs/TEST_REPORT.md` 第 18 节。
- GitHub 安装依赖本机 `git` 可用；不支持私有仓库鉴权（显式禁用 credential helper），需要私有仓库仍走「添加仓库」。
- 新增 Agent 的专有目录是**探测候选**而非官方保证；真机若发现某客户端目录不同，只改 `ADAPTERS` 一处即可。
- `window.confirm` 两处、硬编码白色叠加、state 无锁等 P2 未动。
- 真机仍需验收：设置子 Tab sticky 在矮窗口的表现、LAN/Me 弹窗手感、GitHub 安装在国内网络（含代理）下的成功率、SOUL 建议卡片在弱模型下的意图命中率。
