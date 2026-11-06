# SUGT 测试报告

- **报告跨度**：2026-11-05 ~ 2026-11-06；第 1–17 节记录各轮当时状态，当前状态与发布结论以第 18 节为准
- **分支**：`master`；第 18 节记录 1.1.8 打版与 push，前 17 节的 ahead / 未 push 均为当时快照
- **范围**：P0 固化未提交 WIP → P1 主题校验与协议测试 → P2 巨型模块拆分 → 全链路功能测试 → 文档审计 → 1.1.8 打版评估与升级迁移回归 → 发布链路整改与放行
- **原则**：本报告只记录本机真实执行过的命令与真实输出；未验证、未执行、受限的项目在第 9 节明确列出。

## 1. 环境

| 项 | 值 |
| --- | --- |
| 系统 | macOS Darwin x86_64 |
| Node / npm | v22.22.2 / 10.9.7 |
| Rust / Cargo | 1.97.1 / 1.97.1 |
| 前端 | React 18 + TypeScript 5.7 + Vite 6.4.3 |
| 后端 | Tauri 2.9 + Axum 0.7 |
| 项目版本 | 第 1–17 节为 1.1.7（当时环境）；第 18 节打版为 **1.1.8** |

## 2. 交付概览

| 阶段 | 内容 | 提交 |
| --- | --- | --- |
| P0 | 4 个原子提交固化 1422 行 WIP；残留探测文件纳入 `.gitignore`（不删除） | `d1aacf8` `31cb879` `3ecf025` `daf094b` |
| P1 | 主题校验覆盖全部主题源；协议互转 round-trip 集成测试 | `a64d160` `e4edc53` |
| P2 | 四个巨型模块测试外置 + Responses 上游判定拆分为独立模块 | `8e33d5d` |
| 修复 | 集成测试目标、测试竞态、CI 全覆盖、远程包背景解析 | `81820bb` `1e96418` |
| 整理 | 新文件 rustfmt；文档同步与过期清理 | `08a2907` `b1b8dd6` |

**切分说明**：原计划把前端拆成多个功能提交，实际审阅 diff 后发现 `main.tsx → SettingsPanel → ThemeSettingsSection → theme/* → StatusBar/LogViewer` 是强引用链，拆开会让中间提交编译失败。因此按「每个提交都能独立编译、可 revert」的真实边界切成 4 个：忽略规则 / 后端局域网+failover / Responses 加固 / 前端整体。

## 3. 验证命令与真实结果

| 命令 | 结果 |
| --- | --- |
| `npm test` | ✅ 通过。含主题契约校验、导入冒烟测试、`tsc --noEmit` 双配置类型检查 |
| `npm run build` | ✅ 通过。`tsc && vite build`，1638 modules，59.65s，产出 `dist/index.html` + `dist/overlay.html` 双入口及资源 |
| `cargo check --manifest-path src-tauri/Cargo.toml --lib` | ✅ 通过（最终 4.83s） |
| `cargo test --manifest-path src-tauri/Cargo.toml` | ✅ 全量通过：lib 173 passed / 0 failed / 1 ignored；集成测试 8 + 5 passed；bins 与 doc-tests 0。**合计 186 passed / 0 failed / 1 ignored** |
| `rustfmt --edition 2021 --check`（6 个本轮新增 Rust 文件） | ✅ `FORMAT OK` |
| `cargo fmt -- --check`（全仓） | ⚠️ 报告了仓库既有的格式差异（`bin/cli.rs` 等），未做全仓格式化以免产生无关改动 |
| `git status --short` | ✅ 空，工作区干净 |

## 4. 主题契约校验（`npm run test:themes`）

重写后的 `src/theme/validate-themes.mjs` 校验 **12 个主题源**，全部 `[ok]`，每个 38 个契约令牌：

- 7 个内置 CSS 皮肤：`brickyard / cyber / hiphop / jazz / shanshui / skate / street`
- 2 个内置 JSON 包 + 2 份配套 CSS：`anime-boy / anime-girl`
- 1 个 Agent 模板包：`theme-packs/example`
- `registry.ts` 双向一致性：9 个 id 与源文件、CSS import 语句一一对应

校验项：`:root[data-theme="<id>"]` 选择器（禁止裸 `[data-theme]`）、禁止业务类名覆盖块、38 个令牌齐全、无契约外令牌（`--_xxx` 内部临时变量允许）、声明的背景资源真实存在。

> 修复前先跑了一次未修复状态，脚本按预期报出 2 条失败（`example` 声明了不存在的 `bg.jpg`、以及 `--bg-image` 的相对 URL），证明校验不是空跑。

## 5. 前端功能冒烟测试（`npm run test:theme-pack`）

`tests/theme-pack-smoke.mjs` 用 esbuild 把 `src/theme/loadThemePack.ts` 打包成 ESM 后在 Node 中**真实执行**，8 个用例全过：

| 用例 | 结果 |
| --- | --- |
| 模板包生成 `:root[data-theme]` 选择器 | ✅ |
| 模板包不注入失效背景 URL | ✅ |
| 声明了缺失背景时回退纯色 | ✅ |
| 回退时关闭背景透明度 | ✅ |
| 远程包按 JSON 目录解析背景资源 | ✅（首次失败后修复） |
| 已嵌入 data URL 的背景被保留 | ✅ |
| 契约外 token 被拒绝 | ✅ |
| 缺省令牌由默认值补全为 38 个 | ✅ |

## 6. Rust 测试

| 测试目标 | 数量 | 结果 |
| --- | --- | --- |
| `sugt_lib` 单元测试 | 174 | 173 passed / 1 ignored |
| `tests/dev-platform-module.rs` | 8 | 8 passed（历史上首次编译成功） |
| `tests/protocol_roundtrip.rs` | 5 | 5 passed |
| bins / doc-tests | 0 | — |

新增的 round-trip 集成测试覆盖两条完整链路：

1. `Responses 请求 → Chat 请求 → Chat 响应 → Responses 响应`：工具名、call_id、arguments、usage、stop_reason 往返不丢失；
2. `Anthropic 请求 → OpenAI 请求 → OpenAI 响应 → Anthropic 响应`：system / null 参数过滤 / 历史 tool_call_id / tool_result / tool_use 还原；
3. 边界：上游错误体原样透传、空工具名不泄漏到上游、Codex freeform 工具名无 meta 时还原为 `custom_tool_call`。

## 7. 发现并修复的问题

| # | 问题 | 根因 | 修复 |
| --- | --- | --- | --- |
| 1 | `test:themes` 名不副实，只校验 2 个 JSON，7 个 CSS 皮肤与模板完全没覆盖 | 脚本范围过窄，文件头注释里的运行路径还写错 | 重写校验脚本，覆盖 12 个源 + registry 双向一致性 |
| 2 | `theme-packs/example` 导入后背景失效 | `theme.json` 声明 `bg.jpg` 但目录无此文件 | 模板改为纯配色，文档说明如何添加背景 |
| 3 | 本地导入声明了背景但包内缺文件时注入失效相对 URL | `themePackToCss` 未做资源缺失回退 | 回退 `--bg-image: none` 且透明度置 0 |
| 4 | 远程包背景指向应用页面路径而非 JSON 所在目录 | 值已是 `url("./bg.jpg")` 时跳过基路径重写，与文档承诺不符 | 远程包一律按 `assetBaseUrl` 重写，data URL 除外 |
| 5 | 背景大小提示写死 1.5MB，代码限制是 1.6MB | 文案硬编码 | 由 `MAX_BG_BYTES` 推导，UI / 错误提示 / 中英文档统一 |
| 6 | `dev-platform-module` 集成测试目标从未编译成功 | `mod` 路径指向 `tests/` 根但文件在子目录；且调用 `pub(crate)` 函数 | 补 `#[path]`，公开 3 个纯工具函数，删除废弃 `mod.rs` |
| 7 | CI 只跑 `cargo test --lib`，集成测试永远不被编译 | CI 命令范围过窄 | 改为 `npm test` 与 `cargo test` 全量 |
| 8 | `codex_home_default_under_dot_codex` 随机失败 | 并行测试共享 `CODEX_HOME` 全局状态 | 加互斥锁、保存/恢复环境变量、测试内显式清除 |
| 9 | macOS `launchctl` 单元测试在非 GUI 会话失败 | 单元测试直接调用系统命令 | 标记 `#[ignore]` 并注明手动运行方式 |
| 10 | `SKILL.md` 写了不存在的 `skill list --mounted` | 文档过期 | 改为 `skill list`（输出自带挂载标记） |
| 11 | 验证命令在 AGENTS / README / 主题文档中仍是旧写法 | 文档未随脚本更新 | 中英同步为 `npm test` + `cargo test` |
| 12 | 过期文件 `tests/dev-platform-module/mod.rs` | 自述「不再需要这个文件」 | 已删除并提交 |

## 8. 测试过程中的真实失败与迭代（未做任何结果粉饰）

| 轮次 | 真实输出 | 处理 |
| --- | --- | --- |
| 主题校验（修复前） | `FAILED: 2 issue(s)`，退出码 1 | 确认校验有效，随后修数据与回退逻辑，复跑 12 项全过 |
| round-trip 第 1 次 | `3 passed; 1 failed`，断言找不到 `function_call` | 打印实际响应，发现输出是 `custom_tool_call`——产品对 `shell` 等 Codex freeform 工具名有既有兜底规则。**不是产品 bug，是测试数据选错**；改用中性工具名，并新增一个用例固定该兜底行为 → 5 passed |
| 冒烟测试第 1 次 | `7 passed; 1 failed`，远程包未解析基路径 | 修 `themePackToCss` → 8 passed |
| `cargo test --lib` 第 1 次 | `172 passed; 2 failed`（CODEX_HOME 竞态、launchctl 环境） | 加锁 + ignore → 173 passed / 1 ignored |
| 全量 `cargo test` 第 1 次 | `E0583 file not found for module` | 修模块路径 → 又暴露 `E0603 private function` → 修可见性 → 全量通过 |

## 9. 未完成项与限制（如实列出）

1. **P2 的边界**：四个巨型文件做的是「内联测试外置 + 一个生产模块拆分」，主文件由 9063 行降到 7563 行（约 −16.5%），另建 `responses_upstream.rs`（55 行）与 4 份 `tests.rs`（1452 行）。`me_agent.rs` / `gateway.rs` 等生产逻辑深度耦合私有函数，**按职责全量拆分未做**——那需要行为等价性保障，本轮选择零行为风险的拆分，避免引入回归。
2. **1 个测试被 ignore**：`platform::macos::tests::launch_agent_reloads_when_enabled_twice` 需要真实 macOS GUI `launchctl` 会话；本机报 `Bootstrap failed: 5: Input/output error`。可用 `cargo test -- --ignored` 在有权限的会话手动运行。
3. **Windows CI 未在本机验证**：CI 已改为 `cargo test` 全量，但本机是 macOS，`windows_tests.rs` 由 `cfg(windows)` 跳过；Windows runner 结果需推送后观察。
4. **未执行**：`npx tauri build` 完整桌面打包、Tauri 桌面 UI 手工交互、真实上游 LLM 请求（未配置密钥、未发起外部调用）。
5. **全仓 `cargo fmt --check` 仍有既有差异**：只格式化了本轮新增的 6 个 Rust 文件，没有动既有文件。
6. **未 push（第一轮当时的状态）**：当时 11 个提交在本地 `master`，相对 `origin/master` ahead 13；后续两轮又追加提交，当前状态以第 13 节为准。`origin/HEAD` 指向 `main`，推送前请确认目标分支。
7. **残留文件未删除**：`_me_write_probe.md`、`debug.sh`、`.omo/` 只是被 `.gitignore` 隔离，文件仍在磁盘上。

## 10. 本轮提交清单（新→旧，不含本报告自身的提交）

```
b1b8dd6 doc: 同步验证命令与外观功能，清理过期文档
08a2907 style: 按 rustfmt 格式化本轮新增 Rust 文件
1e96418 test: 增加主题包导入功能冒烟测试并修复远程包背景解析
81820bb fix: 修复从未编译成功的集成测试目标与测试竞态
8e33d5d refactor: 外置巨型模块测试并拆分 Responses 上游判定
e4edc53 test: 补充协议互转跨模块 round-trip 集成测试
a64d160 test: 主题校验覆盖全部主题源并修复失效背景回退
daf094b feat: 主题包导入、自定义背景与状态栏日志
3ecf025 fix: Responses 回落修复空工具名并补回归测试
31cb879 feat: 局域网共享与网关多模型 failover 重构
d1aacf8 chore: 忽略本地探测与调试残留文件
```

## 11. 结论

P0 / P1 / P2 的既定目标全部落地，工作区干净；**所有实际跑过的验证均通过**：`npm test`、`npm run build`、`cargo check`、`cargo test` 全量、`rustfmt --check`（新文件）。共修复 12 个问题，其中 4 个是测试执行过程中新暴露的（远程包背景解析、集成测试目标编译、测试竞态、系统级测试隔离）。

需要注意的只有第 9 节列出的边界：生产代码的深度拆分未做、1 个系统级测试被 ignore、Windows CI 与桌面打包尚未在本机验证、提交尚未推送。

## 12. 追加验收：AgentAdapter 能力矩阵（同日第二轮）

本轮按 `docs/AGENT_ADAPTER_DESIGN.md` 落地 P0 与 P1，工作区干净，所有提交均为本地原子提交、未 push。

本轮 5 个原子提交：`d1ed7a4`（设计文档）、`8371e9a`（目录收敛）、`52f8791`（写入安全）、`45de787`（能力矩阵与预览）、`2a87251`（文档同步与验收）。

| 验证命令 | 真实结果 |
| --- | --- |
| `npm test` | ✅ 主题契约 12 源 × 38 token、导入冒烟 8 例、`tsc --noEmit` 双配置零错误 |
| `npm run build` | ✅ 双入口 `dist/index.html` + `dist/overlay.html`，构建 1m37s |
| `cargo test`（全量） | ✅ lib **191 passed / 0 failed / 1 ignored**；`dev-platform-module` 8 passed；`protocol_roundtrip` 5 passed。合计 **204 passed** |
| `rustfmt --check`（本轮新增 Rust 文件） | ✅ `FORMAT OK` |

### 本轮新增并修复的问题

| # | 问题 | 修复 |
| --- | --- | --- |
| 1 | skill 目录在四个模块各写一份，且自带 skill 会为任意自定义路径凭空建目录 | 新建 `agent_adapters.rs` 作为唯一真相源，四处改为派生；注入只取已存在目录，兜底仅一个 canonical 路径 |
| 2 | `hub.rs` 找不到技能目录时把整个 Agent 静默丢弃 | 返回 canonical 路径并标记 `detected=false` |
| 3 | `codex_config.rs` 的 TOML 解析失败退化成空表并整文件覆盖用户配置 | 解析失败即报错中止，绝不覆盖 |
| 4 | Codex 接管无备份、释放只还原 3 个字段，`model` / `preferred_auth_method` / `auth.json` 丢失 | 接管前备份 + 原文快照，释放逐字节还原；连续接管不覆盖最初快照 |
| 5 | 用户接管期间自己改了 config.toml，释放时会被整文件重写并丢掉注释 | 改用 `toml_edit` 保留注释与格式，只定点清理 SUGT 字段 |
| 6 | `contains("localhost")` 会误删用户自己的本地服务地址 | 只删 SUGT 写入的那个值 |
| 7 | 技能安装 / 挂载「先删后拷」，中途失败会丢数据 | `replace_dir` 原子目录替换 + 失败回滚 |
| 8 | `install_for` 未知 target 静默装成 Claude | 显式报错 |
| 9 | Codex 写入错误被 `let _ =` 吞掉，用户以为接管成功 | 向 UI 传播，配置保存路径记入日志 |
| 10 | 接入前无法知道会改什么；各 Agent 能力无声明 | 新增能力矩阵 + `preview_takeover` 只读预览（含冲突、非法 TOML、目录缺失告警），前端行内展示 |

### 测试过程中的真实失败与迭代

| 轮次 | 真实输出 | 处理 |
| --- | --- | --- |
| 目录收敛脚本第 1 次 | `AssertionError: expected 16 fingerprint path blocks, got 0` | 缩进按 12 空格重写正则，文件未被写入（先确认真实状态再重跑） |
| Codex 安全测试第 1 次 | `4 passed; 2 failed`：`用户改动必须保留` + `PoisonError` | 根因是 `toml::to_string_pretty` 重写丢注释 → 改用 `toml_edit 0.20` 保留格式；测试锁改用 `into_inner()` 容忍中毒，避免连锁误报 |
| `toml_edit::DocumentMut` 编译 | `E0425 cannot find type DocumentMut` | 0.20 的类型名是 `Document`，按编译器提示改用 `Document`，复跑 6 passed |

### 边界（如实列出）

1. **P1 按保守定义交付**：SOUL 获得的是注入 `system_prompt` 的能力矩阵（中英两份），能解释影响并明确「绝不自己改客户端配置」；**没有新增 `me_*` 命令、没有扩大 shell 权限**。原因是 `me_agent.rs` 无工具调用机制，注册命令它也无法调用。
2. 新增的 `preview_takeover` / `list_agent_capabilities` 只做了编译、类型检查与构建验证，**未做桌面 UI 端到端点击验证**。
3. 能力矩阵对 OpenCode / Qwen Code / Aider / Gemini CLI / Cursor / VS Code 明确标注「只写环境变量、配置文件仅探测不写」，**未实现各自官方配置读写**；外部 Agent Skills 资料仅作参考，路径需按本机实际版本核验。
4. P2 未做：skill 版本/增量更新、首次启动主动弹框（`discover_agents` 仍需输入 ≥2 字符）、LAN 与接管写入联动。
5. 未执行：`tauri build` 完整打包、桌面 UI 手工交互、真实上游 LLM 请求；`cargo fmt --check` 全仓仍有既有差异，本轮只格式化新增文件。

## 13. 追加验收：Codex 工具层全挂修复（同日第三轮）

> 第 1–12 节是第一轮 / 第二轮的历史快照：其中的测试数量、提交清单、ahead 数均按当时记录，当前状态以本节为准。写本节前已复核并修正两处过期内容（第 9 节的 ahead 状态、第 12 节缺失的提交 `2a87251`），另修正了 `docs/AGENT_ADAPTER_DESIGN.md` 中错误的快照文件名 `sugt-overrides.json` → `sugt-takeover-state.json`。

### 13.1 触发与症状

用户实测 Codex 接管后：纯文本对话正常，但工具层全挂——所有 `shell_command` 调用返回 `aborted`；`read_mcp_resource` 找不到任何 server（file / agentfs / filesystem 皆报错）；`get_project_overview`、`compress_output` 提示 `unsupported call`。

### 13.2 根因（逐条核对源码行号）

| # | 现象 | 根因 | 位置 |
| --- | --- | --- | --- |
| 1 | `shell_command` aborted | OpenAI Codex prompting guide 明确它是 JSON 参数的 **function** 工具，但 `is_custom_tool_name` 的兜底名单把 `shell_command` 硬判为 freeform，回程输出 `custom_tool_call` + 一次性文本 input，客户端按 function 解析失败 | `openai_responses_adapter.rs:344-353`（旧） |
| 2 | `unsupported call`（MCP / code-mode 工具） | `namespace` 工具只递归 children、丢掉命名空间前缀且回程无还原；未识别 `tools` 静默 `_ => {}`；未识别 `input` item 被序列化成 `role:user` 纯文本，工具闭环断开 | 同文件 `:218-229`、`:251`、`:466`（旧） |
| 3 | 工具调用被上游拒绝 / 中断 | 历史回放里缺 `function_call_output` 时形成孤儿 `tool_calls`；SSE 缺 `response.in_progress` 与 `function_call_arguments.done`，客户端一直挂在 in_progress | 同文件 `:948`、`:1077`、`gateway.rs` 回落路径 |
| 4 | `read_mcp_resource` 无 server | 这是 **Codex 客户端内置工具**，触发条件是客户端自己配置了 `[mcp_servers]`，不是网关能力；SUGT 全仓无 MCP 实现/路由。同时接管 `auth.json` 被整体替换，丢掉原有 ChatGPT 登录态字段 | `codex_config.rs:391-397`（旧）、`agent_adapters.rs:109-120` |
| 5 | hosted 工具 / 长会话 aborted | 接管配置未声明第三方能力：缺 `requires_openai_auth = false`、`capabilities.remote_compaction = "unsupported"`，客户端会按官方账号路径申请 hosted 工具并向网关发 `compaction_trigger` item | `codex_config.rs:107-141`（旧） |

### 13.3 外部佐证（联网调研，非推测）

- **OpenAI 官方 Gateway compatibility requirements**：`A working Chat Completions or Anthropic Messages endpoint doesn't establish Responses compatibility.` / `The full loop must work: Codex receives a call, executes the tool, submits its result, and receives a final answer. A successful text request doesn't verify this loop.`
- **openai/codex #26977**（2026-06，Codex Desktop + `wire_api=responses` 自定义 provider）：所有 MCP 工具 `unsupported call`、内置工具正常；社区兼容代理的做法正是「展平 namespace 工具 + 回程还原名称 + 修 tool history replay + 补 SSE / reasoning / compaction」。
- **openai/codex #22970**（CC Switch proxy 场景）给出可用配方：`experimental_bearer_token` + `requires_openai_auth = false` + `web_search = "disabled"`。
- **CC Switch #8676**（4.0.18）：第三方同名模型继承 `tool_mode: code_mode_only`、共用 `comp_hash`，旧会话持续 `unsupported call: exec`；修法为 `tool_mode: direct` + 独立 `comp_hash` + 完整重启桌面 app-server。
- **同类桥**：LiteLLM proxy 把 namespace 展平成 `multi_agent_v1__spawn_agent`；kimi-codex-bridge 额外补孤儿 `function_call_output`、过滤 `reasoning` / `store` / `previous_response_id`；CLIProxyAPI 做 Responses↔Chat 双向转换；CodexProxy 过滤非标准工具类型；CC Switch 区分「原生 Responses 直连」与「Chat 必须开路由转换」。

### 13.4 修复内容

**协议层**（`openai_responses_adapter.rs`）

1. `ChatToolMeta` 增加 `flattened_names`（展平名 → 客户端原名）与 `has_tool_declarations`。
2. `is_custom_tool_name`：有工具声明时严格按声明判定；兜底名单移除 `shell_command`，仅保留确实是 freeform 的名字。
3. `namespace` 工具展平为 `<namespace>__<name>` 并登记映射，回程在非流式 / 流式两条路径都还原原名。
4. 未识别 `tools` 不再静默丢弃：带 schema 的降级为 `function`，hosted 工具记录告警后跳过。
5. 未识别 `input` item 结构化识别：带 `call_id` + `output` → `role=tool`；带 `name` + `arguments/input` → `assistant.tool_calls`；其它保留文本并告警。
6. 新增 `repair_orphan_tool_calls`，给缺失结果的 `tool_calls` 补配对 `role=tool` 消息。
7. SSE 补 `response.in_progress`、`response.function_call_arguments.done` / `response.custom_tool_call_input.done`；`response.failed` 的 error 补齐 `type` / `code` / `param`。
8. 非流式响应补齐 `created_at`、`output_text`、`parallel_tool_calls`、`tool_choice`、`store`、`incomplete_details`、`error` 等顶层字段。

**接管配置层**（`codex_config.rs`）

9. provider 表增加 `requires_openai_auth = false` 与 `capabilities.{remote_compaction = "unsupported", external_web_access = false}`。
10. 用户原本没写 `web_search` 时注入 `disabled`；快照新增 `web_search_injected` 标志，释放时只在确认由 SUGT 注入且值未变时清理。
11. `auth.json` 改为**合并写入**，只覆盖 `auth_mode` 与 `OPENAI_API_KEY`，保留原有 `tokens` / `account_id` 等登录态字段。
12. 全程不碰 `[mcp_servers]`。

**能力矩阵与文档**

13. `agent_adapters.rs` 的 `writable_fields` 同步新增字段，`note` 明确「不代管 MCP」；`takeover_preview.rs` 增加 MCP 归属告警，SOUL 中英能力摘要同步。
14. `AGENTS.md` 新增「Codex Responses tool compatibility」规范；`README.md` / `README.zh.md`、设计文档第 12 节同步。

### 13.5 验证方法与真实结果

**先写失败用例再改代码**：在 `tests/protocol_roundtrip.rs` 追加 7 个真实 Codex 形态用例（shell_command function、namespace 展平还原、MCP 风格工具结果、孤儿工具调用、未知工具类型降级、非流式顶层字段、SSE 生命周期），首次运行真实输出 `5 passed; 7 failed`，并打印出 `custom_tool_call` / 纯文本工具结果 / 缺失 SSE 事件，与根因逐条对应。修完后该套件 **12 passed / 0 failed**。

| 验证命令 | 真实结果 |
| --- | --- |
| `cargo test --test protocol_roundtrip` | ✅ 12 passed / 0 failed（修复前 5 passed / 7 failed） |
| `cargo test --test codex_gateway_e2e` | ✅ 2 passed / 0 failed：真实 HTTP 网关 + 本地 mock Chat 上游，覆盖两轮工具闭环与流式 SSE 全事件序列 |
| `cargo test`（全量） | ✅ lib **193 passed / 0 failed / 1 ignored**；`codex_gateway_e2e` 2；`dev-platform-module` 8；`protocol_roundtrip` 12。合计 **215 passed / 0 failed** |
| `npm test` | ✅ 主题契约 12 源 × 38 token、导入冒烟 8 例、`tsc --noEmit` 双配置零错误 |
| `npm run build` | ✅ 双入口，1638 modules，1m28s |
| `rustfmt --check`（本轮新增测试文件） | ✅ 两个新增 / 追加的测试文件已格式化 |

端到端用例 `src-tauri/tests/codex_gateway_e2e.rs` 用 `axum::serve` 起真实网关路由与 mock 上游：第一轮 Codex 请求带 `shell_command` + `functions` namespace，上游返回两个 `tool_calls`，断言响应还原成 `function_call` 且名称分别为 `shell_command` / `exec_command`（非 `custom_tool_call`）；第二轮按 `function_call` + `function_call_output` 回灌，断言上游收到的 Chat messages 含两条结构化 `role=tool` 结果。流式用例断言 `response.in_progress` → `function_call_arguments.delta` → `function_call_arguments.done` → `response.completed` → `[DONE]` 完整序列。

### 13.6 测试过程中的真实失败与迭代

| 轮次 | 真实输出 | 处理 |
| --- | --- | --- |
| 新增 7 用例首跑 | `5 passed; 7 failed` | 作为失败基线保留，逐项修协议层与配置层 |
| `cargo test --lib` 第 1 次 | `190 passed; 1 failed`：`lifts_additional_tools_into_chat_tools` 断言 `meta.custom_names.contains("exec")` | 该断言固定的是「namespace 工具不展平」的旧行为，正是 `unsupported call` 的成因。更新断言为展平名 `functions__exec`，并**补上** `flattened_names` 与 `restore_tool_name` 的回程还原断言，测试比原来更强 → 191 passed |
| 全量大文件 rustfmt 检查 | `openai_responses_adapter.rs` 104 行、`codex_config.rs` 163 行、`gateway.rs` 236 行差异 | 用 `rustfmt --emit stdout` 逐条比对，确认差异全部来自既有代码风格（长链拆行等），本轮新增代码合规；只格式化两个新增测试文件，避免制造无关改动 |

### 13.7 边界（如实列出）

1. **未跑真实桌面客户端 / 真实外部 LLM**：本轮「端到端」是真实 HTTP 网关 + 本地 mock Chat 上游，覆盖两轮工具闭环与流式事件；没有实际拉起 Codex / ChatGPT Desktop 点按，也没有向任何真实模型服务商发请求。用户需要在真机重新接管后复测一次。
2. **`code_mode_only` 同名模型风险仍在**：CC Switch #8676 指出客户端会按模型名继承官方工具协议。SUGT 未生成 model catalog（各 Codex 版本字段不稳定，不凭空构造），若上游模型名与官方同名，客户端仍可能套用官方 `tool_mode`。已在能力声明、预览与文档中说明，未做未经验证的 catalog 写入。
3. **MCP 归属**：`read_mcp_resource` / `list_mcp_resources` 能否找到 server 完全取决于客户端自己的 `[mcp_servers]`；SUGT 不代管、不创建、不删除。
4. **未实现的能力**（本次未扩大范围）：MCP server 代理 / 统一面板、原生 Responses 语义级代理（当前仅火山方舟命中时字节透传）、`encrypted_content`、富媒体 part 透传、按 provider 显式切换「直连 vs 转换」模式。
5. 全仓 `cargo fmt --check` 仍有既有差异；第三轮未执行 `tauri build`（第四轮已补 `--no-bundle` 编译，见 14.6）；所有提交均未 push。

### 13.8 本轮提交清单（新→旧）

| 提交 | 内容 |
| --- | --- |
| `doc: 同步 Codex 工具兼容规范与测试报告`（本报告自身） | `AGENTS.md` 新增工具兼容规范、两份 README、设计文档第 12 节、本报告第 13 节 |
| `630e175` | `test: 增加 Codex 工具闭环真实 HTTP 端到端验证` |
| `dad6c40` | `fix: 加固 Codex 接管配置并声明第三方能力` |
| `7efc106` | `fix: 补齐 Responses 工具协议闭环并修复 shell_command 误判` |

四个提交均为本地原子提交、中文提交信息，**未 push**；`master` 相对 `origin/master` 的领先数请在执行推送前用 `git status -sb` 复核（`origin/HEAD` 指向 `main`，推送前需确认目标分支）。

---

## 14. 第四轮：隐藏问题兜底与打包层验证

### 14.1 目标与范围

第三轮修好了「工具闭环」，但 13.7 明确留了空白：没有打包、没有真机、mock 上游过于理想、请求字段白名单只有 7 个、`code_mode_only` 同名模型风险未处理。本轮把**代码里能确定的隐藏问题**逐个补掉，并对不确定字段一律采用可观测、可关闭的 fallback，不凭空构造未经验证的能力。

范围：请求侧静默降级、流式异常分片、上游异常响应伪装成功、`code_mode_only` 规避、打包层编译。

### 14.2 失败基线（先跑红）

新增 `src-tauri/tests/codex_edge_cases.rs`，17 个用例首跑真实输出 `7 passed; 10 failed`。其中 `long_multibyte_payload_does_not_panic` 不是断言失败而是**真实 panic**：

```
thread 'long_multibyte_payload_does_not_panic' panicked at src/openai_responses_adapter.rs:1627:34:
end byte index 500 is not a char boundary; it is inside '中' (bytes 498..501 of string)
```

其余 9 项失败同样逐条打印出现状：`max_output_tokens` 被丢成 `Null`、非法 JSON 被伪装成 `status: completed` 的空回复、未知 item 被序列化成 `{"meta":...,"text":...}` 塞进 user 文本、`output_item.added` 排在两个 `arguments.delta` 之后、匿名工具调用完全没有 `added` 事件、跨 chunk 的「中文测试」变成 U+FFFD。

### 14.3 修复清单（逐条对应失败）

| # | 问题 | 修复 | 位置 |
|---|---|---|---|
| 1 | 长中文 SSE 采样按字节 `&payload[..500]` 切断汉字导致 panic | 按字符边界截断 | `record_upstream_sample` |
| 2 | 逐 chunk `from_utf8_lossy` 把跨 chunk 汉字变成 U+FFFD | `SseLineParser` 改字节缓冲、整行解码 | `SseLineParser::push` / `drain_remainder` |
| 3 | 工具名迟到时 `arguments.delta` 抢在 `output_item.added` 之前 | `ToolCallAccum.pending_deltas` 攒分片，name 到达后先补 added 再按序 flush | `process_tool_call_delta` |
| 4 | 上游始终不给 name → 空名 function_call、缺 added | 收尾 `repair_anonymous_tool_calls` 补占位名（请求里恰好一个工具时用其原名，否则 `unknown_tool`）并补齐事件 | `finalize_events` |
| 5 | 单个非法 JSON 分片被 `unwrap_or({})` 吞掉并影响整条流 | 显式告警后跳过，后续分片继续 | `process_upstream_payload` |
| 6 | 上游空 body / 非法 JSON 伪装成空 `completed` | 返回 Responses 形状 `status: failed` + `error` 对象 | `responses_upstream_failure` |
| 7 | 上游 4xx 被当作成功 SSE 流转发 | 读取错误体，返回 `response.failed` 事件流 | `upstream_failure_sse` + `gateway.rs` 流式 4xx 分支 |
| 8 | `max_output_tokens` 静默丢弃，长任务被上游默认上限截断 | 映射成 `max_tokens` | `responses_to_chat_with_meta` |
| 9 | `input` 里的 `instructions` item 变 user JSON | 转成 `system` 消息 | `normalize_response_item` |
| 10 | 未识别 item 整段 JSON 当用户输入 | `plain_response_item_text` 文本优先、原结构兜底 | `normalize_response_item` |
| 11 | `previous_response_id` / `store` / `reasoning` / `include` 等静默丢弃 | `ChatToolMeta.dropped_request_fields` + 告警，真机抓包可查 | `DEGRADED_RESPONSES_FIELDS` |
| 12 | 上游模型与官方同名 → 客户端套用 `code_mode_only` / `use_responses_lite` | `model` 写 `sugt-` 别名；幂等；`SUGT_CODEX_MODEL_ALIAS=off` 可关 | `codex_model_alias` |

### 14.4 fallback 设计原则

- **降级必须可观测**：不能透传的字段进元数据并打日志，而不是消失。
- **失败必须显式**：空/非法上游、4xx 一律给 `failed`，绝不返回一条「正常地什么都没说」的空回复。
- **不确定就留开关**：别名属于基于公开案例的兼容性推断，各 Codex 版本对未知 slug 行为不同，因此提供一行环境变量回退口。
- **不凭空造能力**：调研确认公开的 `model_catalog_json` 方案是「导出真实目录再改 `use_responses_lite`/`tool_mode`」（linux.do 2026-09-25 排查、Codex #31870、CC Switch #8676）。手写一份只含单模型的 catalog 会**替换客户端整个内置模型目录**，风险大于收益，故不实现。
- **不擅自改模型参数**：`arguments` 拼出非法 JSON 时保留原样并告警，只保证不 panic、不丢工具，不偷偷替换成 `{}`。

### 14.5 外部佐证

- **CC Switch #8676**：第三方同名模型继承 `tool_mode: code_mode_only`、共用 `comp_hash`，旧会话持续 `unsupported call: exec`；修法是 `tool_mode: direct` + 独立 `comp_hash` + 完整退出桌面 app-server。
- **openai/codex #31870**：`use_responses_lite=true` 的模型会让客户端走内部 Responses-Lite 编码，Azure 侧直接报 unsupported-tools；社区做法是换一个不带该标记的模型。
- **linux.do 排查报告（2026-09-25，Codex Desktop 0.155.0-alpha）**：`use_responses_lite=true` 会把 `exec`/`shell`/`apply_patch` 放进 `input[].additional_tools` 而非顶层 `tools`，中转服务只转发顶层 tools 就会「没有工具」；做法是导出真实目录改字段后指向 `model_catalog_json`。
- **CC Switch v4.0.19 升级说明**：按工具协议给模型行分组 `comp_hash`，跨协议切换时先压缩历史，进一步印证「同名 / 共用哈希」这一类问题属于客户端模型目录语义。

### 14.6 验证结果

| 项目 | 结果 |
|---|---|
| `cargo test --test codex_edge_cases` | 16 passed / 0 failed（首跑 7 passed / 10 failed，含 1 个真实 panic） |
| `cargo test` 全量 | 233 passed / 0 failed / 1 ignored：lib 194、edge_cases 16、codex_gateway_e2e 3、dev-platform-module 8、protocol_roundtrip 12 |
| `src-tauri/tests/codex_gateway_e2e.rs` | 新增 `malformed_upstream_body_is_reported_as_responses_failure`：真实 HTTP 网关 + mock 上游返回纯文本，断言返回 `status: failed` 且 `error.message` 含「非法 JSON」 |
| `npm test` | 主题契约（12 源 × 38 token）、导入冒烟、TypeScript 双配置检查全绿 |
| `npm run build` | 双入口 1638 modules 生产构建通过 |
| `npx tauri build --no-bundle` | **成功**，`Built application at: src-tauri/target/release/sugt`（24 MB）。前端 `npm run build` 双入口 1638 modules + Rust `release` profile 完整编译通过；总耗时 6m46s。第三轮「未执行 tauri build」的空白至此补上 |
| `rustfmt` | 两个新测试文件已格式化；改动过的大文件用 `rustfmt --emit stdout --config skip_children=true` 逐条比对，确认本轮新增代码格式合规，既有风格差异未做无关改动 |

### 14.7 边界与真机复测清单

1. **打包层边界**：`--no-bundle` 只验证「前端资源 + Rust release 二进制能一起编译出来」，**未生成 `.app` / `.dmg` 安装包，未做 macOS 签名与公证，未跑 Windows 路径与 Windows CI**。仍未验证真实桌面客户端点按与真实外部 LLM 请求；本轮「端到端」依旧是真实 HTTP 网关 + 本地 mock Chat 上游。
2. **`previous_response_id` 仍无服务端会话存储**：网关依赖客户端重放完整 `input`。字段已降级并打日志，但长会话压缩 / reasoning 续接的真实语义没有实现。真机请跑 20 轮以上或触发一次 compaction 验证。
3. **`code_mode_only` 是缓解不是根治**：别名让客户端不再按官方 slug 匹配模型目录，但若你这版 Codex 对未知 slug 表现异常，用 `SUGT_CODEX_MODEL_ALIAS=off` 回退；若仍见 `get_project_overview` / `compress_output` unsupported call，下一步应按 14.5 的公开方案导出真实模型目录改字段，而不是继续猜。
4. **MCP 归属不变**：`read_mcp_resource` 能否找到 server 只取决于客户端自己的 `[mcp_servers]`，SUGT 不代管。
5. **app-server 缓存**：公开案例一致指出改完模型目录/配置必须**完整退出**桌面应用与其后台 app-server，仅刷新窗口无效。
6. 全仓 `cargo fmt --check` 仍有既有差异；所有提交均未 push。

**建议真机复测顺序**：完整退出 Codex/ChatGPT 桌面端 → 重新接管 → 新建会话 → ① 执行一条 shell 指令 ② 一次触发两个工具 ③ 一段 20 轮以上长会话 ④ 观察日志里 `dropped_request_fields` 与 `response.failed`。若 ① 或 ② 失败，把网关日志与抓到的 `/v1/responses` 请求体带回来定位。

### 14.8 本轮提交清单（新→旧）

| 提交 | 内容 |
|---|---|
| `doc: 同步降级与别名规范并追加第四轮测试报告`（本报告自身） | `AGENTS.md`、中英 README、设计文档第 13 节、本报告第 14 节；同时清理 13.7 与设计文档中「未执行 tauri build」的过期描述 |
| `3e1ffaf` | `test: 增加异常上游响应的真实 HTTP 端到端验证` |
| `12435d0` | `fix: Codex 接管增加模型别名 fallback 规避 code_mode_only` |
| `19f8fa4` | `fix: 补齐 Responses 降级可观测性与流式异常分片兜底` |

四个提交均为本地原子提交、中文提交信息，**未 push**。上一轮的四个提交（`7efc106` / `dad6c40` / `630e175` / 文档提交）同样未 push；`origin/HEAD` 指向 `main`，推送前需用 `git status -sb` 复核目标分支。

## 15. 第五轮：1.1.8 打版评估与升级迁移回归（2026-11-06）

### 15.1 评估结论

**需要打 1.1.8，但不能按当前 `master` 直接打 tag。**

- 对照 `git show origin/master:src-tauri/src/codex_config.rs`：1.1.7 用 `std::fs::write` 直接覆盖 `config.toml`、用 `write_codex_auth` 覆盖 `auth.json`，`clear` 只删 `model_provider` / `model_providers.sugt` / `openai_base_url`，属数据丢失级缺陷，叠加工具层全挂。
- `1.1.7..HEAD` 共 28 提交 / 76 文件 / 9830+ 2185-，含应用内更新、主题导入、LAN failover 等新功能，不是补丁级改动。
- 1.1.7 完全没有应用内更新模块（`src-tauri/src/update/*`、`src/UpdateSection.tsx` 均为 1.1.7 之后新增），老用户不会自动收到新版，必须公告并引导手动下载。
- 阻断项：1.1.7 升级用户没有快照，取消接管仍残留 SUGT 写入的固定值（DEF-004），即最需要修复的存量用户没被修到。

### 15.2 缺陷报告（按标准字段）

| ID | 问题是什么 / 导致的现象 | 复现步骤 | 预期结果 | 实际结果 | 优先级 / 严重级别 | 修复后结果 |
| --- | --- | --- | --- | --- | --- | --- |
| DEF-004 | 1.1.7 升级用户无快照，取消接管后残留 `model` / 鉴权固定值与假 `auth.json`，形成半接管、ChatGPT 登录态不可用 | 1.1.7 启用接管 → 升级 1.1.8 → 保存一次设置（触发自动 apply）→ 取消接管 | `config.toml` / `auth.json` 不残留 SUGT 写入值 | legacy prune 只清 `sugt-` 前缀 model，三项与 auth.json 全保留；自动 apply 还会把污染态写成 `config_before` | **P0 / 阻断** | 识别 1.1.7 形态后按已知写入值清理；auth.json 按确定性归属做字段级 prune，tokens / account_id 等一律保留 |
| DEF-005 | 流式错误判定未同步非流式防护，`error:null` 分片把整轮对话判成 failed 且事件序列错乱 | `stream:true`，上游任一 SSE 分片带 `error:null` 或 `{}` | 与非流式一致，正常收尾 `response.completed` | 先发 `response.failed` 并置 `completed=true`，后续增量乱序且永远收不到终态 | P1 / 高 | 流式与非流式共用 `is_real_error_value`，两条 SSE 用例转绿 |
| DEF-006 | `/v1/responses` 上游失败统一落到通用 502，客户端报「网关响应格式错误」而非真实上游故障 | 上游 4xx（非流式）/ 5xx（流式），单一 provider | Responses 形状 `status: failed` / `response.failed` SSE | `{"error":{"type":"sugt_gateway_error","code":502}}` | P1 / 中高 | 在 `proxy_request` 的 `v1/responses` 分支就地包装，attempt 耗尽 / 5xx / 熔断全部闭合 |
| DEF-007 | Windows `atomic_write` 先删原文件再 rename，失败即丢用户 `config.toml` / `auth.json` | Windows 上目标被占用或权限导致 rename 失败 | 写入失败时原文件保持不变 | 原文件已删，只剩 `*.sugt-tmp` | P1 / 中（平台限定） | 统一 `replace_with_backup`：备份 → 替换 → 失败回滚 → 清理 tmp |

### 15.3 先红后绿（真实输出，未做结果粉饰）

- 先写 7 条回归用例跑红基线：`cargo test --lib` 为 `198 passed / 3 failed`（残留 `model = "demo-model"`、假 key、tmp 文件）；`codex_edge_cases` 为 `21 passed / 2 failed`（打印出发出的 `response.failed`，message 为 `"null"` / `"{}"`）；新增 `release_1_1_8_e2e` 为 `0 passed / 2 failed`（收到 `sugt_gateway_error` 502）。
- 红基线中**既有测试全部通过**，证明用例打在缺陷上而不是空跑。
- 修完后全量 `252 passed / 0 failed / 1 ignored`：lib 203、edge_cases 23、codex_gateway_e2e 4、dev-platform 8、protocol_roundtrip 12、release_1_1_8_e2e 2。

### 15.4 修复落点

| 文件 | 改动 |
| --- | --- |
| `src-tauri/src/codex_config.rs` | 新增 `legacy_1_1_7_gateway_base`、`prune_sugt_auth_fields`、`replace_with_backup`；`clear_sugt_codex_overrides` 在有快照但 auth 被改过时也做字段级清理；`prune_sugt_fields` 增加 legacy 分支；`atomic_write` 改为备份回滚 |
| `src-tauri/src/openai_responses_adapter.rs` | 抽出 `is_real_error_value`，流式 `upstream_error_message` 与非流式共用；`responses_upstream_failure` 改为 `pub(crate)` |
| `src-tauri/src/gateway.rs` | 新增 `responses_failure_response`；`v1/responses` 分支包装 `proxy_responses_via_chat` 的 `Err` |
| `src-tauri/tests/codex_edge_cases.rs` | 新增 2 条 SSE `error:null` / `{}` 用例 |
| `src-tauri/tests/release_1_1_8_e2e.rs` | 新增文件：真实 HTTP 网关 + mock 上游 4xx / 5xx 两条用例 |
| `AGENTS.md` | 固化四条防回归规范（升级迁移、错误判定对称、失败出口闭合、禁止先删后搬） |

### 15.5 验证结果

| 命令 | 结果 |
| --- | --- |
| `cargo test`（全量） | **252 passed / 0 failed / 1 ignored** |
| `npm test` | 主题契约 12 源 × 38 token、主题包冒烟 8、`tsc --noEmit` 双配置检查全绿 |
| `npm run build`（含在 tauri beforeBuildCommand 中） | 双入口 1638 modules 生产构建通过 |
| `npx tauri build --no-bundle` | 成功，`src-tauri/target/release/sugt`（release 编译 2m55s） |
| `cargo build --release` | 最终格式微调后用同一份代码增量复验，19:43 重新生成 `sugt` / `sugt-cli` |
| `rustfmt --check` | 两个测试文件通过；三个大文件只修本轮新增段格式，既有风格差异未做无关改动 |
| `git diff --check` | 无空白错误 |

### 15.6 边界与发版前待办

1. **版本号四处仍是 1.1.7**（`package.json:3`、`src-tauri/Cargo.toml:3`、`src-tauri/Cargo.lock:3531`、`src-tauri/tauri.conf.json:4`）；按约定等确认目标平台后再改。
2. **打包链路缺失**：本地 `scripts/` 目录不存在，`package.json` 全部 `package:release*` 指向失效 ps1；`release-feature.yml` 仅 Windows portable zip 且 `workflow_dispatch` 需预置 tag；macOS 的 1.1.7 两个 zip 是 10-26 历史产物，当前流程无法复现；签名公证未做。
3. **1.1.7 无应用内更新**：必须公告引导手动下载，并写清升级后如何干净退出接管。
4. **真机验收**：本机 `sugt-takeover-state.json` 是新版接管态，常规复测测不出升级路径，需专门构造 1.1.7 遗留形态；真实外部 LLM 请求、20 轮以上长会话、48 工具是否存在同名仍未验证。
5. **本轮未修的 P2**：state 文件无锁且损坏静默降级、config/auth/state 三文件无事务、匿名工具兜底名不在声明内、custom 工具流式无 delta、`dropped_request_fields` 白名单外的字段仍静默丢弃。
6. `--no-bundle` 只验证前端资源 + Rust release 二进制可编译，**未生成 `.app` / `.dmg`**。

### 15.7 工作区状态

本轮改动全在工作区，**未 commit、未 push**：`AGENTS.md`、`docs/TEST_REPORT.md`、`src-tauri/src/codex_config.rs`、`src-tauri/src/gateway.rs`、`src-tauri/src/openai_responses_adapter.rs`、`src-tauri/tests/codex_edge_cases.rs`，新增 `src-tauri/tests/release_1_1_8_e2e.rs`。`master` 相对 `origin/master` ahead 28，远端 tag `v1.1.7` 指向 `9d14dee`。

## 16. 第六轮：导航重组 + 能力进化回归（2026-11-06）

### 16.1 报告头

| 项 | 内容 |
| --- | --- |
| 报告编号 | SUGT-TR-16 / 第六轮能力进化回归 |
| 测试范围 | 设置导航重组（主 Tab 9→7、6 个横向子 Tab、概览页、sticky）、Agent 目录补全与发现、GitHub 地址安装 skill、触发词与规则化建议、SOUL 只读编排层；并对 1.1.8 前五轮成果做全量回归 |
| 被测版本 | 工作区 HEAD `2254b4b` + 本轮未提交改动；四处版本号仍为 `1.1.7`（未打版） |
| 环境 | macOS（aarch64）、Node / Vite 6.4.3、Rust edition 2021、Tauri 2.9；`cargo test` 全量、`npm test`、`npm run build`、`npx tauri build --no-bundle` |
| 测试类型 | 功能、业务、UI 结构、架构边界、安全（host 白名单 / 路径穿越 / 事务回滚）、回归 |
| 结论 | **通过（可进入真机验收）**；无阻断项，遗留风险见 16.7 |

### 16.2 用例与结果统计

| 套件 | 用例数 | 结果 |
| --- | --- | --- |
| `cargo test --lib` | 219 passed / 1 ignored | 全绿（较第五轮 203 → 219，新增 16 条） |
| `tests/codex_edge_cases.rs` | 23 | 全绿 |
| `tests/codex_gateway_e2e.rs` | 4 | 全绿 |
| `tests/dev-platform-module.rs` | 8 | 全绿 |
| `tests/protocol_roundtrip.rs` | 12 | 全绿 |
| `tests/release_1_1_8_e2e.rs` | 2 | 全绿 |
| **Rust 合计** | **268 passed / 0 failed / 1 ignored** | 1 ignored 为 macOS GUI `launchctl` 用例 |
| `npm test` | 主题契约 12 源 × 38 token、主题包冒烟 8、`tsc --noEmit` 双配置 | 全绿 |
| `npm run build` | 1642 modules 双入口生产构建 | 通过 |
| `npx tauri build --no-bundle` | 前端 + Rust release 编译（2m42s），产物 `src-tauri/target/release/sugt` | 通过 |
| `git diff --check` | 无空白错误 | 通过 |

基线可追溯：先跑出 `217 passed / 2 failed / 1 ignored` 的红基线（`soul_context` 断言过严、`github_install` 的 `..` 清洗缺口），修复后转绿；另有 1 条高负载下偶发假红，见 DEF-008。既有 217 条全通过，说明新增用例打在新增缺陷上，不是空跑。

### 16.3 本轮新增用例（16 条，按能力分组）

| 能力 | 用例 | 判定点 |
| --- | --- | --- |
| Agent 目录 | `catalog_covers_domestic_and_global_clients`、`unconfirmed_skill_dirs_always_have_a_shared_fallback` | 14 个国内外客户端进目录；未确认目录必含 `~/.agents/skills` 兜底且不虚报写配置 |
| 发现链路 | `empty_query_lists_every_known_agent`、`newly_added_domestic_and_global_clients_are_searchable`、`catalog_item_uses_a_safe_env_template_for_new_clients` | 空查询返回完整目录；新增客户端可搜；env 模板安全 |
| GitHub 安装 | `only_known_hosts_are_allowed`、`path_traversal_in_repo_parts_is_rejected`、`import_ids_are_stable_and_sanitized`、`scan_finds_skill_md_and_skips_dot_dirs`、`repo_root_can_be_a_single_skill` | host 白名单、路径穿越、id 稳定且无 `..`、扫描规则、仓库根即单技能 |
| 触发词 / 建议 | `parses_trigger_words_from_separated_values`、`parses_yaml_list_triggers`、`missing_frontmatter_produces_actionable_suggestions` | 逗号/顿号、YAML list、缺 frontmatter 给可操作建议 |
| SOUL 编排 | `brief_is_bilingual_and_never_requests_a_write`、`brief_exposes_runtime_facts_without_panicking_on_missing_files`、`brief_contains_all_four_route_tokens` | 双语、禁写、读目录失败降级、四类 route 齐全 |

前端无单测框架，UI 由 `tsc --noEmit` 双配置 + 生产构建 + 静态核对覆盖：`en` 字典为 `Record<keyof typeof zh, string>`，**缺键会直接编译失败**，因此本轮新增的 zh/en 键成对由 `npm test` 保证。

### 16.4 缺陷报告

| ID | DEF-008 | DEF-009 |
| --- | --- | --- |
| 标题 | 高负载下 `shell_runs_in_the_given_directory` 用 5s 超时导致全量套件偶发假红 | GitHub 安装生成的技能 id 残留 `..`，目录名带路径穿越语义 |
| 复现 / 触发条件 | 全量 `cargo test` 与前端构建并发、两个 `run_loop` 用例各跑 60s+ 时；单独运行 `cargo test --lib me_agent::tests::shell_runs_in_the_given_directory -- --exact` | 调 `make_import_id(parsed, "../evil")`；或 owner/repo 含连续点号时生成 id |
| 预期结果 | 用例稳定通过，超时行为由专门用例验证 | id 只含一个路径组件，不含 `..`、`/`、`` |
| 实际结果（证据） | `test result: FAILED. 218 passed; 1 failed; 1 ignored`；`assertion left == right failed: /private/var/.../sugt-me-echo-...`，`left: None, right: Some(0)`，即 `run_shell` 在 5s 被 kill 后 `exit_code=None` | 红基线 `store::github_install::tests::import_ids_are_stable_and_sanitized` 失败；`git__owner-repo__..--evil` |
| 导致的现象 | 全量套件假红，阻断放行判定；不影响任何用户功能 | 技能落地目录名污染；真实 clone 的相对路径不会产生 `..`，但函数对外暴露且 id 直接用作 `skills_dir` 子目录名 |
| 根因 | `src-tauri/src/me_agent/tests.rs:344` 只验证工作目录与输出，却与超时行为共用 5s 硬超时 | `src-tauri/src/store/github_install.rs` 原 `sanitize()` 保留点号，且未过滤纯点号路径段 |
| 修复后的结果 / 验收 | 超时放宽到 15s 并注释说明；单独复跑 3 次均 0.29s 通过，全量 `219 passed / 0 failed / 1 ignored` | 新增 `sanitize_relative()` 丢弃 `.`/`..` 段并把点号折叠为 `-`；`import_ids_are_stable_and_sanitized` 补 5 组穿越形态断言，全量转绿 |
| 优先级 / 严重级别 | P2 / 一般 Major（测试稳定性） | P2 / 一般 Major（防御性，非用户可见） |
| 状态 | 已修复 | 已修复 |

**排除项（已核实不成立）**：① Def-008 不是本轮功能回归——单独复跑 3 次 0.29s 全绿、cwd 断言未报错；② 不是 shell 不存在或 spawn 失败——`pwd` 已输出正确路径，另一条 `shell_times_out_and_stops` 正常；③ 不是磁盘满或构建产物损坏——同环境 `tauri build --no-bundle` release 编译成功；④ Def-009 不影响已安装技能存量——`staged.json` 中已有 id 未变（合法相对路径无点号段），仅新安装路径受影响。

### 16.5 修复落点

| 层 | 文件 |
| --- | --- |
| 导航 / UI | `src/main.tsx`、`src/SettingsPanel.tsx`、`src/SettingsOverview.tsx`、`src/LogsSection.tsx`、`src/AboutSection.tsx`、`src/TakeoverPreviewModal.tsx`、`src/styles.css`、`src/i18n.ts` |
| Agent 单一真相源 | `src-tauri/src/agent_adapters.rs`、`src-tauri/src/agent_discover.rs`、`src/AgentDiscoverModal.tsx` |
| 技能安装 / 元数据 | `src-tauri/src/store/github_install.rs`（新增）、`store/mod.rs`、`store/commands.rs`、`app.rs`、`permissions/app-commands.toml`、`store/catalog.rs`、`store/hub.rs`、`agent_advanced.rs`、`src/SkillsPanel.tsx`、`gen/schemas/acl-manifests.json`（命令注册生成物） |
| SOUL 只读编排 | `src-tauri/src/soul_context.rs`（新增）、`lib.rs`、`me_agent.rs`、`src/MePanel.tsx` |
| 测试加固 | `src-tauri/src/me_agent/tests.rs` |
| 文档 | `AGENTS.md`、`docs/AGENT_ADAPTER_DESIGN.md` 第 14 节、`README.md`、`README.zh.md`、本报告 |

### 16.6 架构边界复核（本轮红线）

- SOUL **只注入事实、只输出 `[[SUGT_ROUTE]]` 标记**，前端剥离标记渲染建议按钮，点击仅跳转页面；没有任何工具循环、bash 写配置、下载仓库或安装技能路径。接管/安装/挂载仍走确定性命令 + `TakeoverPreviewModal` 后果预览 + 用户确认。
- GitHub 安装不写客户端配置、不登记 `repos.toml`，host 白名单 + 浅克隆 + `SKILL.md` 扫描 + 原子落地 + 失败回滚 + 幂等 id；依赖本机 `git`，不支持私有仓库鉴权。
- 触发词与优化建议是**规则化**的，不编造、不依赖 LLM；模型不可用时全部功能仍可用，只少了意图路由与解释。
- 新增 14 个客户端只声明 `UserEnv` 能力，未确认官方 skill 目录的一律含 `~/.agents/skills` 兜底。

### 16.7 遗留风险与真机验收（未做，不影响本轮放行结论）

1. 版本号四处仍为 `1.1.7`；打包脚本、tag、push 按约定未动，需先确认目标平台。
2. 未生成 `.app` / `.dmg`（`--no-bundle` 只验证前端资源 + release 二进制）。
3. 真机待验：设置子 Tab sticky 在矮窗口表现、日志轮询 gate 的 CPU 收益、GitHub 安装在国内网络（含代理）成功率、SOUL 建议卡片在弱模型下的意图命中率、1.1.7 legacy 升级路径与 Codex 客户端工具层。
4. 既有 P2 未动：`window.confirm` 两处（`TakeoverProfilesSection.tsx:123`、`SkillsPanel.tsx:471`）、硬编码白色叠加、state 文件无锁且损坏静默降级、config/auth/state 三文件无事务、`dropped_request_fields` 白名单外字段仍静默丢弃。
5. 全仓 `cargo fmt --check` 仍有既有风格差异；本轮只保证新增段格式，`git diff --check` 无空白错误。

### 16.8 放行结论与工作区状态

**通过率 268/268（1 ignored），0 失败，前端类型检查 / 构建 / 打包全部通过；无 P0 / Blocker，可进入真机验收，暂不建议直接打 tag。** 阻断依据仍沿用 15.6：1.1.7 无应用内更新、打包链路缺失、版本号未改、真机升级路径未验。

本轮 38 个改动/新增文件全部在工作区，**未 commit、未 push**；未改版本号、未建 tag。

## 17. 第七轮：窗口净高补偿 + Codex 接管行为核查（2026-11-06 晚）

### 17.1 报告头

| 项 | 内容 |
| --- | --- |
| 报告编号 | SUGT-TR-17 |
| 测试范围 | ① 状态栏导致的初始窗口净高不足与尺寸持久化（按用户确认的方案 2 实施）② Codex 接管后「请求通过但浏览器无动作」的日志与代码核查（只读，未改产品代码） |
| 被测版本 | 工作区 HEAD `2254b4b` + 本轮未提交改动；版本号仍 `1.1.7` |
| 环境 | macOS aarch64；本机真实日志 `~/Library/Application Support/sugt/sugt.log`（22:25–22:27 会话）、本机 `~/.codex/config.toml`（22:24 重新接管） |
| 结论 | 窗口修复通过；Codex 现象**不是网关失败，也不是新增功能 bug**，是接管能力边界 + 弱上游共同造成的能力降级，但「请求通过」的表述具备误导性，建议产品侧明示 |

### 17.2 DEF-010：状态栏加入后初始窗口净高被吃掉 34px

| 字段 | 内容 |
| --- | --- |
| 问题 / 现象 | `tauri.conf.json` 的 `height:680` 从最早版本 `4acd85c` 未变；状态栏提交 `daf094b` 把 `.app-shell` 改成「内容 1fr + 状态栏 auto」两行（22 个改动文件**不含** `tauri.conf.json`），状态栏 `min-height:34px` + 全局 `border-box` → 内容区 `680→646`。用户打开后控制台底部首屏看不到，手动拉大后重启又回 680 |
| 复现 | 默认状态栏开启 → 启动 → 停在 1040×680 控制台 → 观察底部卡片 |
| 预期 / 实际 | 净高应与状态栏加入前一致且尺寸可保留 / 净高少 34px，`maxHeight:820` 还封顶，全仓无窗口尺寸持久化 |
| 根因 | 布局改成两行后静态窗口配置未同步；`minHeight:640` 时净高仅约 606；`.dashboard-layout{min-height:100%}` 与设置 sticky 子 Tab 行进一步压缩首屏 |
| 修复 | `tauri.conf.json`：`height 680→714`、`minHeight 640→674`、`maxHeight 820→960`；新增 `src-tauri/src/window_state.rs`（启动恢复 + Resized 防抖落盘 + min/max 与当前显示器可用区 clamp + 忽略 restore 回声避免小屏覆盖大屏记录 + 全路径失败降级）；`app.rs` 在 `show()` 前恢复、在 `Resized` 里换算逻辑像素后保存；`styles.css` 提 `--status-bar-h:34px`；顺带修 `i18n.ts` zh/en 两处 `set.statusBarHint` 过期文案（日志已在设置-日志） |
| 修复后结果 / 验收 | 初始净高恢复 680，用户手动调整的尺寸写入 `config_dir/window-state.json` 并在下次启动恢复。单测覆盖 clamp / NaN / 越界、往返读写、损坏文件降级、恢复回声识别、状态栏补偿常量；真机仍需验 1040×714 首屏截图与「拉大→重启→保持」 |
| 优先级 / 严重级别 / 状态 | P2 / 一般 Major / **已修复** |

### 17.3 DEF-011：Codex「请求通过」但浏览器未被操作（只读核查，未改代码）

| 字段 | 内容 |
| --- | --- |
| 问题 | 接管后让 Codex 打开浏览器/网址，客户端回「请求通过」，肉眼看不到浏览器动作 |
| 复现 | 22:24 重新接管 Codex → 上游为「实验·免费通道」`kilo-auto/free` → 22:25–22:27 让 Codex 操作浏览器 |
| 预期 | 要么真正驱动浏览器，要么明确告知该能力不可用 |
| 实际（已确认证据） | 日志全为 `inbound request ok`，`response.failed` 0 次，latency 2–11s；`proxying codex responses via chat completions` 显示 `model=kilo-auto/free`、`mode=chat_completions_adapter`、`custom_tools:0`；每轮 `codex responses stream completed` 的 `tool_calls:1`、`content_len` 多为 3，`input_tokens` 由 18455 递增到 28664，伴 `rewrote history tool_calls names to flattened upstream form`；本机 `~/.codex/config.toml` 有 `web_search="disabled"`、`[model_providers.sugt.capabilities] external_web_access=false / remote_compaction="unsupported"`、`[mcp_servers.node_repl]` 仍在 |
| 根因（分层） | ① 网关层正常：请求确实转发成功并返回，「请求通过」对应的是网关请求成功，不是浏览器动作；② 浏览器操作由 ChatGPT 桌面端本地插件（Computer Use / Browser Use，经 `node_repl` MCP）执行，SUGT 不代管也不执行；③ SUGT 接管按设计注入 `web_search="disabled"` 与 `external_web_access=false`（`codex_config.rs:183-213`），且 Responses→Chat 回落跳过无 schema 的 hosted 工具（`openai_responses_adapter.rs:412-416`，日志 08:01Z / 12:57Z 可见 `web_search` 被跳过，22:25 会话已不再声明）；④ 当前上游是免费弱模型 + Chat 回落，工具调用稳定性不足 |
| 导致的现象 | 用户把「网关请求成功」误判为「动作已执行」；实际无浏览器操作，且会话反复重试（token 持续上涨） |
| 修复后结果 / 建议 | ① 换支持稳定 tool calling 的正规上游做 A/B 验证；② 接管影响预览补一条「接管后 hosted 联网 / 浏览器自动化能力不可用」；③ 把「跳过 hosted 工具」的 WARN 汇总到日志页，避免误判；④ 若必须保留官方浏览器能力，只能取消接管并承担 hosted 工具在自定义网关上报 unsupported 的风险。**本轮未改任何产品代码** |
| 优先级 / 严重级别 / 状态 | P2 / 一般 Major（认知误导、体验）/ **待确认** |

**排除项（已核实不成立）**：① 不是网关失败或 502——0 条 `response.failed`、0 条 ERROR，`inbound request ok`；② 不是 MCP 配置丢失——`[mcp_servers.node_repl]` 仍在，SUGT 明确不代管 MCP；③ 不是 Codex 配置被写坏——三项能力字段是 SUGT 按设计注入且有快照可还原；④ 不是 `search_content_types` 报错——日志中无该错误，`web_search=disabled` 已生效。

**待验证推断（外部资料 / 未实测）**：① 公开资料称 Codex 0.162 的 `capabilities.external_web_access=false` 控制「该 provider 的 hosted web search 能否访问公网」，第三方网关文档也把 `web_search="disabled"` 列为自定义 provider 必需项（否则报 `search_content_types` 不支持）；公开资料**未覆盖**桌面端 Computer Use / Browser Use 插件是否连带受影响，需真机对照；② 「请求通过」四字的确切出处（模型文本 / 客户端审批提示 / 工具结果）需用户提供原句或截图；③ 免费模型能否稳定产出浏览器工具调用未验证；④ 反复重试是否与工具名展平/还原映射有关未定位。

### 17.4 验证结果

| 命令 | 结果 |
| --- | --- |
| `cargo test`（全量，最终代码复跑） | lib **223 passed / 1 ignored**；edge 23、codex_e2e 4、dev-platform 8、protocol 12、release_1_1_8 2 → **272 passed / 0 failed / 1 ignored** |
| `cargo check` / `git diff --check` | dev profile 6.54s 通过；无空白错误 |
| `npm test` | 主题契约、主题包冒烟 8、`tsc --noEmit` 双配置全绿（i18n zh/en 键由 `Record<keyof typeof zh, string>` 编译期保证） |
| `npm run build` | 1642 modules 双入口生产构建通过 |
| `npx tauri build --no-bundle` | release 编译 2m53s，产物 `src-tauri/target/release/sugt`，退出码 0 |

### 17.5 边界与待办

- 窗口尺寸恢复/保存是 GUI 行为，单测只覆盖纯逻辑；真机待验：1040×714 首屏控制台完整性、拉大窗口后重启是否保持、换小显示器是否被 clamp、关闭状态栏后是否无空白。
- Codex 侧建议按 17.3 的四条建议择一验证后再决定是否改代码；本轮保持只读。
- 版本号四处仍 `1.1.7`，打包脚本、tag、push 未动；`--no-bundle` 未生成 `.app` / `.dmg`。

### 17.6 工作区状态

本轮新增 `src-tauri/src/window_state.rs`；修改 `src-tauri/tauri.conf.json`、`src-tauri/src/lib.rs`、`src-tauri/src/app.rs`、`src/styles.css`、`src/i18n.ts`、`AGENTS.md`、本报告。全部**未 commit、未 push**，未改版本号、未建 tag。

---

## 18. 第八轮：1.1.8 打版与发布链路验收（2026-11-06 深夜）

- **报告编号**：SUGT-TR-18
- **测试范围**：版本号一致性、GitHub Actions 自动打包（Windows / macOS 双架构）、Homebrew cask 滚动发布、应用内更新资产匹配、`lsuenweb` 下载映射、双方文档与规范同步
- **被测版本**：`master` 工作区 HEAD `2254b4b` + 第 15–17 轮未提交改动 → 打版 **1.1.8**
- **环境**：macOS（Homebrew 7.0.11、Node 20、Rust stable）、GitHub Actions `windows-latest` / `macos-15` / `macos-15-intel` / `ubuntu-latest`、`lsuen/sugt` 与 `lsuen/homebrew-sugt` 远端仓库
- **执行方式**：静态核对 + 编译/测试 + 远端 API 写入 + 配置解析校验；检查阶段只报告，用户授权「打版 1.1.8 然后 commit + push」后执行

### 18.1 缺陷清单（发布链路）

| ID / 标题 | 复现 / 触发条件 | 预期 | 实际（证据） | 现象与影响 | 根因 | 修复后结果与验收 | 优先级 / 级别 | 状态 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| DEF-011 版本号 6 处仍为 1.1.7，且旧文档只记「四处」 | 打版前全仓检索 | 6 个文件、7 个键统一 1.1.8 | `package.json`、`package-lock.json`（顶层 + `packages[""]`）、`src-tauri/Cargo.toml`、`Cargo.lock`、`tauri.conf.json`、`src/release-notes.ts` 均为 1.1.7；第 15 节写「四处」 | 应用内更新会把自身判成旧版；发布包与 tag 不一致 | 前轮统计漏 `package-lock.json` 与 `release-notes.ts` | 6 文件统一 1.1.8，`check_version_consistency.py` 报「7 个版本键一致」退出码 0；脚本同步扩展到 7 键 | P0 / Critical | 已修复 |
| DEF-012 release 资产名携带版本号 | 查看 `v1.1.7` 远端资产 | 稳定资产名，发版只替换同名文件 | `SUGT-1.1.7-windows-x64.zip`、`SUTAI-1.1.7-macos-arm64.zip`、`SUTAI-1.1.7-macos-x64.zip`、`SUTAI_1.1.7_x64-setup.exe` | 每次发版必须同步改下载站、cask、文档，任一遗漏即下载 404 | 打包脚本按 tag 拼接文件名 | 固定为 `SUTAI-windows-x64-setup.exe` / `SUTAI-windows-x64.zip` / `SUTAI-macos-arm64.zip` / `SUTAI-macos-x64.zip` / `SUTAI-checksums.txt`；`/releases/latest/download/<name>` 永久有效 | P0 / Critical | 已修复 |
| DEF-013 macOS 与 brew 无自动打包流程 | 读 `.github/workflows/release-feature.yml` | push `v*` tag 自动产出 win + 双架构 mac | 旧 workflow 仅 `workflow_dispatch` + `windows-latest`，只传 zip 和 NSIS；mac 两个 zip 是 10-26 手工产物 | mac 用户拿不到与新 tag 对应的包；无法复现 | CI 从未覆盖 macOS | 新增 `release.yml`：tag push 触发，windows / macos(matrix 双架构) / publish 三腿，产出 5 个固定名资产 | P0 / Critical | 已修复 |
| DEF-014 Homebrew cask 需每版手改且 trust 命令写错 | 读远端 `Casks/sugt.rb` 与两份 README | cask 一次配置长期有效 | `url` 含 `v#{version}` + 双 `sha256`，每版必改；文档写 `brew trust lsuen/sugt`（既非 tap 名也缺 `--cask`） | `brew install` 装到旧版或报错；用户照文档执行失败 | cask 绑定语义版本；trust 参数写错 | cask 改 `version :latest` + `sha256 :no_check` + `releases/latest/download/SUTAI-macos-#{arch}.zip`；本机 `brew trust --help`（7.0.11）确认 `--cask <全名>` 语法；文档、tap README、`lsuenweb` 三处命令修正 | P1 / Major | 已修复 |
| DEF-015 下载站 Intel Mac 拿到 arm64 包，且无法列多包 | 读 `src/main.rs::pick_asset` 与下载模板 | 按架构分别给链接 | 旧逻辑每种 kind 只取一个资产，macOS 双架构 zip 只会命中第一个；`apk` 偏好规则写死 | Intel Mac 下载错误架构；Windows 便携包/安装包无法共存 | 单资产选择 + 硬编码关键词 | 新增 `Project.assets` 配置化精确名 + `asset_specs` / `resolve_assets` 多资产去重 + 关键词回退；模板说明按系统选择 | P1 / Major | 已修复 |
| DEF-016 `lsuenweb` 中与 SUGT 相关文案过期 | 读 `assets/projects.toml` | 与 1.1.8 一致 | `sutai-cli`（实际 `sugt-cli`）、「Windows 公开版 180 天试用」（已无该限制）、旧 Tab 名称、旧接管/技能描述 | 用户按错误命令操作失败，误解授权 | 文案未随产品迭代 | 全部改为 `sugt-cli`、移除试用限制、同步 22 客户端 / GitHub 安装 / 触发词 / SOUL 只读 / 设置→关于路径 | P2 / Minor | 已修复 |
| DEF-017 新增下载映射代码编译失败 | `cargo check --all-targets` | 0 error | `E0106` 缺生命周期 + 4×`E0716` 临时值被借用，共 5 处（`src/main.rs:150/500/514/540/550`） | 下载站无法构建，push 后自动部署会失败 | `asset_specs` 返回借用未标 `'a`；测试把 `&gh_assets(...)` 临时量直接传入 | 显式 `fn asset_specs<'a>(project: &'a Project, ...)`；4 处测试改 `let assets = gh_assets(...)` 绑定；`cargo check --all-targets` 退出码 0 | P0 / Blocker | 已修复 |

### 18.2 关键设计判定（含排除项，均为「已确认」）

| 判定 | 依据 |
| --- | --- |
| **改固定资产名不会断应用内更新** | 逐条核对 `src-tauri/src/update/mod.rs:140-170` 四个 cfg 分支：Windows `setup`+`.exe`、Windows `windows`+`.zip`、mac arm64 `macos`+`arm64`+`.zip`、mac x64 `macos`+(`x64`\|`x86_64`)+`.zip`，5 个新资产名全部命中；`asset_sha256` 取 GitHub asset `digest`，publish job 仍会上传资产 |
| **tag 必须保留语义化 `v1.1.8`** | `check_for_update` 用 `cmp_version(release.tag_name, CARGO_PKG_VERSION)`；固定 `latest` tag 会解析成 `0.0.0`，导致更新检查永久静默失效。Release **标题**带版本不违反「打包文件不挂版本号」的要求 |
| **Homebrew 不需要额外打「brew 包」** | cask 本质是指向 release 资产的 Ruby 定义；固定资产名后，CI 产出 mac zip 即等价于 brew 可安装。`:latest` cask 被普通 `brew upgrade` 跳过，故文档统一给 `--greedy` |
| **排除：改名不会让下载站 404** | `lsuenweb` 精确名配在 `assets/projects.toml`，同时保留关键词回退；旧版资产名仍能被回退规则命中 |
| **排除：首次 `cargo build` 失败是磁盘权限** | 首轮 `L6deEu` 报 target 产物不可写（沙箱拦截旧产物）；改用 `CARGO_TARGET_DIR=/tmp/lsuenweb-verify-target` 后暴露出的是 DEF-017 的真实代码错误，已单独修复并复跑通过 |
| **排除：`macos-15-intel` runner 不存在** | 已联网核对 GitHub runner label 可用；`aarch64-apple-darwin` 与 `x86_64-apple-darwin` 分别用各自原生 runner，避免交叉编译 |
| **排除：CI 依赖作者本地脚本** | 旧 `/scripts`（gitignore）不在 workflow 引用链中；`package.json` 的 `package:release*` 仅本地便利命令 |

### 18.3 验证结果

| 检查项 | 命令 / 手段 | 结果 |
| --- | --- | --- |
| 版本号一致性 | `python3 docs/孙测测/scripts/check_version_consistency.py --root . --expect 1.1.8` | **通过**，7 键均为 1.1.8，退出码 0；留痕 `traces/版本一致性检查-20261106-233857.md` |
| workflow 结构 | Ruby `YAML.safe_load` | 通过，`jobs = windows, macos, publish`；触发 `push.tags=["v*"]` + `workflow_dispatch` |
| workflow 产物名一致性 | 静态比对 `env.ASSET_*`、`publish.files`、cask `url`、`lsuenweb [projects.assets]`、`update::pick_asset` | 5 个资产名五处一致 |
| 项目配置 | Python `tomllib` 解析 `assets/projects.toml` | 通过，`sugt.assets` 4 键齐全，项目数不变 |
| sugt Rust | `cargo test --manifest-path src-tauri/Cargo.toml` | lib **223 passed / 1 ignored**；edge 23、codex_e2e 4、dev-platform 8、protocol 12、release_1_1_8 2 → **272 passed / 0 failed / 1 ignored** |
| sugt 前端 | `npm test` / `npm run build` | 主题契约 + 主题包冒烟 + `tsc --noEmit` 双配置全绿；1642 modules 双入口构建通过 |
| sugt 打包 | `npx tauri build --no-bundle`（第 17 节最终代码） | 通过，release 产物 `src-tauri/target/release/sugt` |
| lsuenweb | `cargo check --all-targets` / `cargo test` / `cargo build --release`（独立 target 目录） | 0 error；**5 passed / 0 failed**；release 构建通过 |
| 静态卫生 | `git diff --check` | 无空白错误 |
| 远端写入 | `gh api` 更新 `Casks/sugt.rb`、tap `README.md` | 写入后回读确认，cask `ruby -c` 语法通过 |

**通过率：272/272（1 ignored）+ lsuenweb 5/5，0 失败；缺陷 7 项全部已修复。**

### 18.4 遗留风险与放行结论

- **放行结论：可打版 1.1.8 并 push。** 无 P0 / Blocker 遗留，核心链路（版本号、资产命名、CI 触发条件、应用内更新匹配、下载站映射）均已验证。
- **待验证推断（不阻断放行，push 后观察）**：GitHub Actions 首次实跑 `windows` / `macos` 三腿与 `publish` 合并上传是否成功（含 NSIS bootstrapper、macOS 双架构 `.app` 产物路径、`sha256sum` 清单）；Runner 实际排队时长。
- **已知留白**：macOS 应用未签名/未公证（沿用历史策略，README 已给 `xattr -cr`）；`:latest` cask 需 `--greedy`；1.1.7 无应用内更新模块，存量用户需公告手动下载；设置 sticky、GitHub 安装国内网络、SOUL 弱模型命中率、1.1.7 legacy 升级路径与 Codex 客户端工具层仍待真机验收。
- **CI 之外的验证边界**：本地 `--no-bundle` 不生成 `.app` / `.dmg` / NSIS，最终安装形态以 CI 产物为准。

### 18.5 本轮改动清单

- **sugt 新增**：`.github/workflows/release.yml`；**删除**：`.github/workflows/release-feature.yml`
- **版本号**：`package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`src-tauri/tauri.conf.json`、`src/release-notes.ts`
- **文档**：`AGENTS.md`（Release & packaging 规范）、`README.md` / `README.zh.md`（删个人情况段落、修 brew 命令、补固定资产名表）、`docs/AGENT_ADAPTER_DESIGN.md`、`docs/孙测测/scripts/check_version_consistency.py`、`docs/孙测测/MAP.md`、本报告、`.gitignore`（孙测测目录入库）
- **远端仓库**：`lsuen/homebrew-sugt` 的 `Casks/sugt.rb` 与 `README.md`
- **lsuenweb**：`src/projects.rs`、`src/main.rs`、`assets/projects.toml`、`assets/templates/download.html`、`AGENTS.md`、`docs/MAP.md`

### 18.6 首跑暴露的两项缺陷（push 后真实 CI 证据，非本地推断）

推送 `v1.1.8` tag 后，GitHub Actions 首跑给出了本节证据；按上线闭环要求，逐条复验后重建 tag 再跑。

| ID / 标题 | 复现 / 触发条件 | 预期 | 实际（已确认证据） | 现象与影响 | 根因 | 修复后结果与验收 | 优先级 / 级别 | 状态 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| DEF-018 `npm ci` 在 macOS 报 EBADPLATFORM，release 两条 mac 腿全挂 | 在 `macos-15` / `macos-15-intel` 上执行 `npm ci` | 三腿构建成功、publish 上传 5 个资产 | run `42246105220`：job `127988623937` 日志 `npm error notsup Unsupported platform for @tauri-apps/cli-win32-x64-msvc@2.9.0: wanted {"os":"win32","cpu":"x64"} (current: {"os":"darwin","cpu":"arm64"})`；两个 mac job `completed failure`，`publish: completed skipped`，`gh release view v1.1.8` 返回 `release not found`。Windows job `completed success`，上传 22,455,666 字节 artifact | 用户拿不到任何 macOS 包，Release 根本不会创建；Windows 产物孤立 | `package.json` 把 Windows 专用二进制 `@tauri-apps/cli-win32-x64-msvc` 写成**根 devDependency**；lockfile 顶层对应条目无 `optional: true`，npm 视为必需依赖做 os/cpu 校验。`@tauri-apps/cli` 自身已通过 optionalDependencies 按平台安装二进制，重复声明有害 | 删除该 devDependency，`npm install --package-lock-only` 重生成 lock（diff 仅 2 处：根声明 + 顶层非 optional 条目，嵌套 2.11.2 optional 条目保留）；本机 macOS `npm ci` 退出码 0，`npm test` / `npm run build` 通过；重建 tag 后 run `42250535758` 三腿 + publish 全绿，5 个固定资产上传成功 | P0 / Blocker | **已修复并 CI 复验通过** |
| DEF-019 `ci` 工作流在 Windows 长期假红（3 个用例） | `windows-latest` 上 `cargo test` | 0 failed | run `42246044892`：`212 passed; 3 failed`。① `agent_adapters::tests::expand_home_handles_all_path_forms` 断言 `/tmp/x`，Windows 得 `C:/tmp/x`；② `me_agent::tests::prompt_names_both_directories_and_a_fresh_shell` 断言 `/tmp/sugt-home/projects`；③ `me_agent::tests::shell_runs_in_the_given_directory` 执行 `pwd && cat marker.txt`，PowerShell 的 `pwd` 输出表格且 `&&` 不可用，报 `Os { code: 3, ... The system cannot find the path specified }`。历史 run `40217193771`、`40199740654` 同样失败 | 每次 push 的 CI 都是红的，掩盖真实回归；发布质量门禁失效 | 单元测试硬编码 Unix 路径字面量与 POSIX shell 语法，未考虑 CI 跑在 Windows | 绝对路径改用 `std::env::temp_dir()` 构造；提示词断言改为 `projects_root(&config).display().to_string()`；shell 用例按 `cfg(windows)` 选 `cat marker.txt`（bash 与 PowerShell 均支持），路径回显断言只保留在 Unix。修复后 Windows CI run `42250341481` **success** | P1 / Major | **已修复并 CI 复验通过** |

**首跑其它结论（已确认）**：Windows 腿 `npx tauri build --bundles nsis` + `cargo build --bin sugt-cli` + 稳定名打包全部成功，说明 Windows 侧路径、NSIS 产物定位、`Compress-Archive` 逻辑正确；`macos-15` 与 `macos-15-intel` runner label 可用，set up job / checkout / setup-node / rust-toolchain 四步都通过，失败点唯一且已定位到 `npm ci`。

**排除项**：不是 macOS runner 缺 Rust target（日志显示 `rustc 1.99.1 host: aarch64-apple-darwin` 正常）；不是 `setup-node` 缓存污染（失败发生在 `npm ci` 首次解析阶段）；不是 Windows 的 `npm ci` 也失败（Windows job 全程 success，证明 optionalDependencies 在 win32 上能正确装到 `cli-win32-x64-msvc@2.11.2`）；不是代码回归（本地 272 用例全绿、前端构建通过）。

**处理结果**：因 `publish` 被跳过、远端 `v1.1.8` Release 不存在，删除并重建同名轻量 tag 到修复后的 HEAD，重新走一次完整的 tag push 自动发布链路，保证 tag 指向的构建号与最终产物一致（避免「构建号不可追溯」）。

### 18.7 封版复验（2026-11-07）

| 检查项 | 证据 | 结果 |
| --- | --- | --- |
| 版本号一致性 | `check_version_consistency.py --root .`；留痕 `traces/版本一致性检查-20261107-002813.md` | 7 键均为 1.1.8 |
| release 工作流复跑 | run [`42250535758`](https://github.com/lsuen/sugt/actions/runs/42250535758)：windows / macos-arm64 / macos-x64 / publish 全 `success` | 通过 |
| GitHub Release | [v1.1.8](https://github.com/lsuen/sugt/releases/tag/v1.1.8) 资产：`SUTAI-windows-x64-setup.exe` / `SUTAI-windows-x64.zip` / `SUTAI-macos-arm64.zip` / `SUTAI-macos-x64.zip` / `SUTAI-checksums.txt`（**均无版本号**） | 通过 |
| `/releases/latest/download/*` | HEAD 全部 HTTP 200；体积约 9.0–13.6 MB；checksums 可下载 | 通过 |
| Homebrew cask | 远端 `lsuen/homebrew-sugt@e2474bf`：`version :latest` + `sha256 :no_check` + `releases/latest/download/SUTAI-macos-#{arch}.zip`；本机 `brew info` 显示 `latest` | 通过（本轮无需再改 tap） |
| lsuenweb | `assets/projects.toml` 四个资产精确名已固定，无版本号 | 已就绪 |
| Windows CI（DEF-019） | run `42250341481` `success` | 通过 |
| winget | 见下方备注；**不阻断** 1.1.8 封版 | 待 moderator |

**封版结论：v1.1.8 已发布，Release / brew / 下载站资产链路闭环完成。**
