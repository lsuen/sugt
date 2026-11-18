# SUGT v1.1.11 PRD — 网关可观测性与编排能力

> 版本：v1.1.11
> 状态：草案（待作者评审后定稿）
> 范围：本 PRD 同时覆盖产品功能设计与开发设计（架构 / 抽象 / 解耦），并约定 commit 与 task 的关联方式，用于后续追回进度与问题。

---

## 0. 阅读说明（协作方式）

- 本文档**不写强制规则**。每一条是"期望的结果 + 理由"，实现有偏差时评估影响、跑门禁、记录偏差即可。
- 当实现与本文档冲突、或需求本身含糊时，**先向作者确认再动工**，不猜。
- 每个需求拆出编号化 task（T1.1、T2.1 …），commit message 用 `type(scope): [task-id] 摘要` 格式关联回本文档，形成 commit → task → PRD 的可追查链。
- 完成情况回填到文末「Task 清单」表，保持单一事实源。

---

## 1. 背景与目标

SUGT 是本地 AI 网关 + 技能控制台。当前痛点集中在"上游不可观测"与"无法编排"：

1. **连通性测试失真**：现在的"测试连接"只发一个 `ping`，HTTP 200 就算过，既不验证 LLM 真的能回话，也不测延迟/速度。用户无法区分"网络通"和"模型真的能用"。
2. **模型列表无法批量治理**：LLM 提供商返回的模型列表一大片，哪些能用只能逐个试，且无法禁用坏模型、无法批量测试。
3. **多厂商无法编排**：长任务场景下，单厂商限速/变慢时没有自动切换手段。
4. **对外接入信息不可见**：SUGT 已提供 `/v1/models` 接口，但用户不知道本机 IP、baseurl、modelid 怎么拿。
5. **API Key 无法池化**：多 key 轮换、用量监控缺失。
6. **请求改写规则硬编码**：Codex 的 max_tokens / tool 重试等规则写死在代码里，不可配置、可能误伤其他客户端。
7. **缓存与日志会膨胀**：日志只写不清理，缓存无清理入口。

本版本目标：**让网关"可观测、可治理、可编排、可配置"**，为长任务稳定运行打基础。

---

## 2. 需求总览与已对齐决策

| 编号 | 需求 | 已对齐的关键决策 |
| --- | --- | --- |
| R1 | 连通性测试改为测 LLM 连通性（返回内容 + 延迟 + t/s） | 新增后端 probe 抽象层；前端 toast 提示 |
| R2 | 「模型列表」新主 tab，批量测试 / 禁用 / 启用 | 独立主 tab |
| R3 | 策略盒子（多厂商聚合编排） | **轻量策略配置**（优先级/权重/轮换/速度阈值切换），不做可视化节点连线 |
| R4 | 获取 modelid（models 接口 + 本机 IP + 复制按钮） | 并入「模型列表」tab |
| R5 | API Key 池（多 key 轮换 + 用量监控） | **一期只监控 + 告警，不硬拦截**；独立主 tab |
| R6 | 请求改写规则可配置（作用域 + key-value） | 放「规则」主 tab；作用域默认/codex/其他 agent/全局 |
| R7 | 设置页清理缓存 + 日志自动清理周期 | 有默认值 + 最大值 |

---

## 3. 现状基线（勘查结论）

以下为开发前必须知道的现状，后续设计都建立在此基础上：

### 3.1 连通性测试现状
- 后端入口：[gateway.rs `test_provider_connection`](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/gateway.rs#L2803) → `test_openai_connection` / `test_anthropic_connection`。
- 当前实现：发 `messages=[{"role":"user","content":"ping"}]`，`stream:false`，`max_tokens:8`，只要 HTTP 2xx 就返回成功，**不读响应体、不测延迟/速度**。
- 前端入口：[ProviderModal.tsx](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src/ProviderModal.tsx#L115-L140) 调用 Tauri 命令 `test_provider_draft`，成功即 toast "连接成功"。

### 3.2 模型配置与模型列表现状
- 数据结构：[model.rs `ProviderConfig`](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/model.rs#L37-L62)：`id/name/provider/base_url/api_key/model_name/protocol/enabled/status/last_checked_at/last_error/model_alias/auto_adapt_base_url/tool_text_retry_enabled/tool_text_retry_extra`。**这是 provider 级配置，没有 model 级状态**。
- 已有命令 `list_provider_models`（[commands.rs](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/commands.rs#L585-L616)）返回 `Vec<String>`，走 `gateway.list_provider_models` → `list_openai_models`。
- 前端 [UpstreamSelector.tsx](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src/UpstreamSelector.tsx#L102-L135) 已封装模型列表加载与选择。

### 3.3 网关路由与 Codex 规则现状
- 三条协议线在 [gateway.rs](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/gateway.rs)：`proxy_responses_via_chat`（Codex）/ `proxy_chat_upstream`（Chat）/ `proxy_anthropic_request`（Claude）。
- **硬编码规则**：[openai_responses_adapter.rs `DEFAULT_CODEX_MAX_TOKENS=32768`](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/openai_responses_adapter.rs#L14-L24) + `responses_to_chat_with_meta` 自动填 `max_tokens`；[gateway.rs `rewrite_model`](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/gateway.rs#L2367-L2393) 也自动填 `max_tokens`。`tool_text_retry_enabled/extra` 在 ProviderConfig 上。
- **客户端识别已具备**：`x-sugt-client` header（`me` 客户端已带）+ [anthropic_client_detect.rs](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/anthropic_client_detect.rs#L38-L43) 的 User-Agent 精确/模糊识别（claude-cli / codex / cline / windsurf 等）+ [gateway_stats.rs `client_label`](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/gateway_stats.rs#L366-L367)。这是 R6 作用域配置的基础。

### 3.4 设置页 / 导航 / 日志现状
- 主 tab 在 [main.tsx](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src/main.tsx#L601-L611)：`dashboard/me/models/clients/skills/rules/settings`。
- 设置页子 tab 在 [SettingsPanel.tsx](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src/SettingsPanel.tsx#L43-L54)：`overview/general/appearance/advanced/pet/logs/about`。
- 日志：[logging.rs](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/logging.rs#L5-L24) 用 `tracing_appender::rolling::never` 写 `sugt.log`，**无滚动/清理**。配置目录 `config_dir` 在 [config.rs](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/config.rs#L19-L31)。

### 3.5 前端调用方式
- 前端**无统一 request utils**，各组件直接 `import { invoke } from '@tauri-apps/api/core'` 调命令。抽象层应落在**后端**（Rust 模块），前端只做薄封装。

---

## 4. 需求详情

### R1 — 连通性测试改为测 LLM 连通性

**目标**：测试按钮验证的是"模型真的能回话"，并展示延迟与速度。

**产品设计**
- 用户点击"测试连接"后，向 LLM 发送 `直接回复 我在`，拿到真实回复即视为通过。
- toast 文案：`网络测试通过！模型返回"我在"`（"我在"不做内容校验）。
- 附加展示：延迟（ms）、速度（t/s）。失败时展示错误原因。

**技术设计**
- 新增 Rust 模块 `src-tauri/src/probe.rs`（snake_case 文件名，符合规范）。
- 核心函数与返回结构：

```rust
// probe.rs
pub struct ProbeReport {
    pub ok: bool,
    pub reply: Option<String>,      // 模型返回内容
    pub latency_ms: u64,            // 首包/完整响应延迟（见下）
    pub tokens_per_sec: f64,        // 速度
    pub completion_tokens: u32,     // usage.completion_tokens，缺失则按字节估算
    pub error: Option<String>,
}

pub async fn probe_llm_connectivity(client: &Client, provider: &ProviderConfig) -> ProbeReport;
```

- 请求体：`messages=[{"role":"user","content":"直接回复 我在"}]`，`stream:false`，`max_tokens` 给足够（如 64，避免 "ping" 那种 8 导致回复被截）。
- 延迟口径：一期取**完整响应耗时**（发起 → 收到完整 body）；流式首包延迟（TTFT）记为后续优化项。
- 速度口径：`completion_tokens / 耗时秒`。上游返回 `usage.completion_tokens` 就用它，否则用 `回复字节数 / 4` 近似，并在返回里标注是估算值。
- 替换现有 `test_provider_connection` 内部实现（或让旧函数改为对 `probe_llm_connectivity` 的薄包装），`test_provider_draft` 命令改为返回 `ProbeReport` 而非裸 `String`。
- 前端：ProviderModal 测试按钮解析 `ProbeReport`，构造 toast（成功/失败两种文案 + 延迟/t/s）。

**验收标准**
- 用一个能正常回话的 provider 测试：toast 显示成功 + 延迟 + t/s 数值合理（>0）。
- 用一个网络通但模型失效的 provider（如错误 model name）：toast 显示失败原因，而非"连接成功"。
- 现有 `gateway.rs` 里对 `test_provider_connection` 的调用（provider 状态探测）不受破坏。

---

### R2 — 「模型列表」主 tab（批量测试 / 禁用 / 启用）

**目标**：把 provider 返回的一大片模型变成可观测、可治理的清单。

**产品设计**
- 新增主 tab「模型列表」（sidebar，位置在「模型」之后、「客户端」之前）。
- 顶部工具栏：
  - 服务商下拉框（关联已配置的 provider，含实验免费通道）。
  - 按钮：`获取/刷新`、`测试全部`、`测试选中`、`启用选中`、`禁用选中`。
- 下方容器框内表格，列：
  - 复选框（最左，支持全选）
  - 名字（model id）
  - 连通性（可用/不可用/未知，色点）
  - 延迟(ms)
  - 速度(t/s)
  - 测试时间
  - 启用状态（开关）

**已对齐决策（2026-11-18 作者确认）**
- **「测试全部」= 全量测试当前服务商的所有模型**，不受当前勾选影响；点击后先弹二次确认（说明将测多少条、可能耗时与消耗额度），确认后才开始。
- **禁用即隐藏**：模型被禁用后，控制台（`UpstreamSelector` 的模型选择）**直接不出现**该模型；「模型列表」tab 仍完整展示并带禁用态，用户在那里重新启用即可恢复。

**技术设计**
- 新增 model 级状态数据，与 provider 级 `ProviderConfig` 解耦：

```rust
// model_registry.rs（新）
pub struct ModelEntry {
    pub id: String,              // 唯一 id（provider_id + model_name 的稳定键）
    pub provider_id: String,     // 关联 provider 或 "experimental-free"
    pub model_name: String,
    pub enabled: bool,           // 默认 true
    pub last_probe: Option<ModelProbe>,
}

pub struct ModelProbe {
    pub ok: bool,
    pub latency_ms: u64,
    pub tokens_per_sec: f64,
    pub probed_at: DateTime<Utc>,
    pub error: Option<String>,
}
```

- 存储：独立 JSON 文件 `config_dir/model-registry.json`（高频读写 + 与 config.toml 热重载解耦，避免污染主配置）。加载/持久化走现有原子写路径（pid 备份 + rename，参考 `codex_config.rs::atomic_write`）。
- 命令：
  - **保留** `list_provider_models` 原签名（`ProviderModal` 与 `UpstreamSelector` 两处仍读 `Vec<String>`，不改它，避免一处改动牵连三处）。
  - 新增 `list_model_registry(provider_id)`：上游拉取模型列表 + 合并 registry 的启用/禁用与最近探测结果，返回 `Vec<ModelEntry>`。未知新模型自动补 registry（enabled=true，未探测）。
  - 新增 `set_model_enabled(provider_id, model_name, enabled)`（用 provider_id + model_name 定位，不用拼字符串 id 让前端猜）。
  - 新增 `probe_models(provider_id, model_names: Vec<String>)`：并发（限并发数）对每个 model 跑 R1 的 probe，回写 registry 并返回最新 `Vec<ModelEntry>`。
- 前端：新组件 `src/ModelListPanel.tsx`，复用 R1 的 probe 结果展示。

**验收标准**
- 刷新后表格展示该 provider 全部模型，数量与上游一致。
- 禁用某个模型后，控制台模型选择不再出现它；重新启用后恢复。
- 批量测试后，各列连通性/延迟/速度/测试时间正确填充。
- 测试结果持久化：重启应用后仍能看到上次测试时间（连通性可标记为"未知/需重测"）。

---

### R3 — 策略盒子（多厂商聚合编排，轻量策略配置）

**目标**：把不同厂商的模型聚合为一个"策略组"，按策略选路由，服务长任务限速/变慢自动切换。

**产品设计（轻量策略配置，非可视化工作流）**
- 新增"策略盒子"概念：一个策略组 = 多个成员（provider + model）+ 一套路由策略。
- 策略模式（一期）：
  - `priority`：按优先级，高优先级可用则用，失败/变慢则降级。
  - `weighted`：按权重随机/轮换分配。
  - `round_robin`：顺序轮换。
  - `speed_threshold`：记录每个成员最近延迟，超过阈值自动切换；配合冷却时间避免抖动。
- 每个成员可配：所属 provider、model、权重、优先级。
- 控制台可选一个"策略盒子"作为上游（对外暴露一个统一 model id）。

**技术设计（一期落地范围）**

数据模型：新增 `src-tauri/src/strategy_box.rs`，字段沿用上方 `StrategyBox` / `StrategyMember`，另加：

```rust
pub struct StrategyRuntime {          // 进程内，不落盘
    pub rotation: HashMap<String, u64>,        // box_id -> round_robin 游标
    pub health: HashMap<String, MemberHealth>, // "{box_id}::{provider_id}::{model_name}"
}

pub struct MemberHealth {
    pub last_latency_ms: Option<u64>,
    pub consecutive_failures: u32,
    pub cooldown_until: Option<Instant>,
    pub last_error: Option<String>,
}
```

**1) 触发键（先定这个，其余都靠它）**

网关当前根本不看请求里的 model：`ordered_providers()`（gateway.rs L2320）只按 `active_provider_id` + failover 排队，`rewrite_model`（L2379）再把出站 model 改成 `provider.model_name`。所以策略盒子必须自己抢在前头。命中顺序：

1. 客户端请求体里的 `model` 命中某个 `enabled` 盒子的 `public_model_id` → 用这个盒子；
2. 否则看当前激活 provider 的 `model_name` 是否命中盒子 → 用这个盒子（这是控制台「选一个策略盒子」的落地方式：控制台只是把 `provider.model_name` 写成盒子 id）；
3. 都不命中 → 完全走现有 `ordered_providers()`，R3 对既有行为零影响。

**2) 存储与加载**

- 落盘 `config_dir/strategy-boxes.json`（`{version:1, boxes:[...]}`），复用 R2 `model_registry.rs` 的原子写做法。
- `GatewayState` 增加 `strategy: Arc<RwLock<StrategyRuntime>>` 与 `boxes: Arc<RwLock<Vec<StrategyBox>>>`；启动时从盘加载，`save_strategy_box` / `delete_strategy_box` 写盘后同步刷新内存副本（不做 mtime 轮询，避免热重载竞态）。
- 保存校验：`public_model_id` 非空、全局唯一、且不得与任何 provider 的 `public_model_id()` 冲突；成员至少一个；`speed_threshold` 模式必须有阈值；`weighted` 模式总权重 > 0。

**3) 选路：把成员展开成 attempt 链**

新函数 `providers_for_request(config, boxes, client_model) -> Vec<ProviderConfig>`，返回一条**有序 attempt 链**，直接喂给现有 `upstream_attempts` 的调用点（gateway.rs L835 / L1373 / L1579 三处替换 `ordered_providers`）：

- 成员 → `ProviderConfig` 克隆：按 `provider_id` 查 provider，跳过停用 provider；`model_name` 覆盖成成员自己的；成员模型若在 R2 注册表里被禁用则跳过（禁用即不参与编排，与「禁用即隐藏」同源）。
- 排序按模式：
  - `priority`：`priority` 升序（1 最高），整条链都保留作为降级后备。
  - `round_robin`：`rotation[box_id] += 1` 后对健康成员轮转起点，再环形展开整条链。
  - `weighted`：按权重做无放回加权抽样，得到一条覆盖全部成员的顺序；`weight=0` 视为不参与。
  - `speed_threshold`：先看健康度——`now < cooldown_until` 或 `last_latency_ms > threshold` 的成员排到链尾；其余按延迟升序。没有健康成员时退化为 `priority` 顺序，不整条拒绝。
- 链长度上限 8（盒子成员再多也只取前 8），避免一次请求打穿所有上游。
- 实验免费通道成员：复用 `prepare_upstream_provider`，free 源的内部 failover 仍由 `upstream_attempts` 负责。

**4) 健康度与冷却**

- 只记内存，不落盘：每次 attempt 结束按「HTTP 2xx 且未命中错误体」判成功。成功 → `last_latency_ms = 本次耗时`、`consecutive_failures = 0`、清冷却；失败 → `consecutive_failures += 1`，若有 `cooldown_ms` 则 `cooldown_until = now + cooldown_ms`。
- 与现有熔断器（`GatewayState::is_circuit_open` / `trip_circuit`，按 provider_id）**并存不替代**：熔断是 provider 级粗粒度，盒子健康度是成员级细粒度，两者任一命中都跳过该成员。
- `speed_threshold` 的判据是**最近一次成功请求的耗时**（不是均值），避免历史值拖住恢复。

**5) 与既有链路的关系**

- `rewrite_model` / `rewrite_model_stream` 不动：客户端看到的仍然是它请求的盒子 id，上游拿到的仍是成员的 `model_name`，`client_model_of` 已经是这个口径。
- `/v1/models`（L665）追加所有启用盒子的 `public_model_id`（`owned_by: "strategy"`），客户端才能发现这些 id。
- 重试预算、`tool_text_retry_*`、会话提示、`response.failed`/completed notice 全部沿用现有逻辑；成员全挂时的退出方式不改。

**6) 命令**

- `list_strategy_boxes` / `save_strategy_box` / `delete_strategy_box`（读写 JSON + 刷新内存）。
- `get_strategy_box_status`：返回每个成员的 `last_latency_ms / consecutive_failures / cooldown_until / last_error`，供 UI 展示健康度；只读，不再发探测请求（探测走 R2 的 `probe_models`）。

**7) UI（已定：新主 tab「策略盒子」）**

- sidebar 在「模型列表」之后新增主 tab「策略盒子」：列表 + 编辑弹窗（名称 / public id / 模式 / 阈值 / 冷却 / 成员增删），删除走 `ConfirmModal`。
- 控制台 `UpstreamSelector` 在「模型」下拉里追加启用盒子的 public id，选中即写 `provider.model_name`（按触发键第 2 条生效）。

**已对齐决策（2026-11-18 作者确认）**
- 配置入口：**新主 tab「策略盒子」**。
- 选中方式：**双入口**——客户端请求体 `model` 命中优先，其次激活 provider 的 `model_name` 命中（控制台写入）也生效。
- `priority` 语义：**数字小优先**（1 最高），升序排序。

**验收标准**
- 能创建一个策略盒子，添加多个不同厂商的成员，保存后控制台可选它。
- `round_robin` / `weighted` / `priority` 三种模式路由行为符合预期（可用统计验证分发）。
- `speed_threshold`：人为把某成员延迟调高（或指向慢上游），观察请求自动切到其他成员，且冷却期内不来回抖动。
- 成员全部失败时，走现有"预算用尽 → completed 助手提示"退出，不裸 502。

> 注：这是本版本最重的一块。以上数据流经作者确认后再拆 T3.1→T3.3 编码。

---

### R4 — 获取 modelid（models 接口 + 本机 IP + 复制）

**目标**：让用户知道 SUGT 自己的 `/v1/models` 怎么访问，并一键复制 baseurl / modelid。

**产品设计（并入「模型列表」tab 顶部或独立区块）**
- 展示一行接入信息：`已请求 http://<本机IP>:<端口>/v1/models`（本机 IP 前置打印）。
- 下方列表：baseurl 与 modelid 的复制按钮，包含：
  - `sutai`（对外统一 model id）
  - 指定厂商各自的 model id
- 每个条目：`baseurl` 复制、`modelid` 复制两个动作。

**技术设计**
- 后端新增命令 `get_local_access_info`：返回本机局域网 IP（枚举网卡取非 loopback 的 IPv4，取不到回退 `127.0.0.1`）+ 网关端口 + 监听 host。
- 复用 `list_provider_models` 得到各 provider 模型，或直接读 registry，拼出 modelid 列表。
- 前端复制用 `navigator.clipboard.writeText`，成功给 toast。

**验收标准**
- 顶部正确显示本机 IP + 端口（局域网内另一台机器可据此访问）。
- 复制按钮能复制出正确的 baseurl 和 modelid。
- `sutai` 与各厂商 modelid 都在列表里。

---

### R5 — API Key 池（多 key 轮换 + 用量监控，一期不硬拦截）

**目标**：一个 provider 可配多个 key，轮换使用，并看到每个 key 的用量。

**产品设计**
- 新增主 tab「Key 池」（sidebar）。
- 池结构：一个池 = 绑定某 provider + 多个 key。
- 每个 key 展示：脱敏 key（如 `sk-***abcd`）、启用开关、已用次数、已用 token、最近错误、配额（只读展示，一期不拦）。
- 轮换策略（一期）：`round_robin` / `least_used`（按次数最少的先用）。
- 一期**只监控 + 告警，不硬拦截**：配额达到时 UI 标红告警，但不拒绝请求（避免上游不返回 usage 时计数不准误伤）。

**技术设计**
- 新增 `src-tauri/src/api_key_pool.rs`：

```rust
pub struct ApiKeyPool {
    pub id: String,
    pub name: String,
    pub provider_id: String,        // 绑定 provider
    pub selection: KeySelection,    // round_robin / least_used
    pub keys: Vec<ApiKeyEntry>,
}

pub struct ApiKeyEntry {
    pub id: String,
    pub key: String,                // 明文存储与现有 ProviderConfig 一致；仅展示脱敏
    pub enabled: bool,
    pub usage: KeyUsage,
    pub quota: Option<KeyQuota>,    // 一期只读展示
}

pub struct KeyUsage {
    pub request_count: u64,
    pub total_tokens: u64,          // 从 usage.total_tokens 累加；缺失则估
    pub last_error: Option<String>,
}
```

- 存储：`config_dir/api-key-pools.json`；用量为运行时内存态 + 定期/退出时持久化（高频更新，避免每次请求写盘）。
- 网关埋点：转发时若命中 key 池，按策略选 key 替换请求的 `Authorization`，响应回来后累加 usage 到内存计数。
- 命令：`save_api_key_pool` / `list_api_key_pools` / `delete_api_key_pool` / `reset_key_usage`。

**验收标准**
- 能添加多个 key，UI 脱敏展示，轮换策略生效（`least_used` 下请求会流向次数最少的 key）。
- 发起若干请求后，用量统计（次数/token）增长正确。
- 达到告警阈值时 UI 有标红提示，但请求仍正常发出。

> 安全提示：key 明文存储与现有 `ProviderConfig.api_key` 一致，PRD 暂不引入加密；如作者要求加密需单独评估迁移成本。

---

### R6 — 请求改写规则（作用域可配置，取代硬编码）

**目标**：把 Codex 的 max_tokens / tool 重试等硬编码规则，变成可 UI 编排、按作用域生效的配置。

**产品设计（放「规则」主 tab）**
- 新增"请求改写规则"子区块，规则列表 + 增删改。
- 每条规则：
  - 名称
  - 作用域（scope）：`默认` / `codex` / `其他 agent 客户端`（可单独指定具体客户端）/ `全局`
  - 开关（enabled）
  - 动作列表（actions）
- 动作类型（一期）：
  - `设置默认 max_tokens`（对应现有 Codex 自动填 32768）
  - `设置请求头`（key-value）
  - `设置请求体字段`（key-value，如 `max_completion_tokens`）
  - `未调用工具时再要`（tool_text_retry，extra 次数）
- 作用域默认策略（可全关）：
  - `默认`：对所有请求生效（宽松兜底）。
  - `codex`：默认开（保持现行为），可关。
  - `其他 agent 客户端`：默认关，可开。
  - `全局`：默认关，可开（覆盖所有）。

**技术设计**
- 新增 `src-tauri/src/request_rules.rs`：

```rust
pub struct RequestRule {
    pub id: String,
    pub name: String,
    pub scope: RuleScope,       // Default / Codex / Agent(String) / Global
    pub enabled: bool,
    pub actions: Vec<RuleAction>,
}

pub enum RuleScope {
    Default,
    Codex,
    Agent(String),     // 具体客户端标签
    Global,
}

pub enum RuleAction {
    SetDefaultMaxTokens(u32),
    SetHeader { key: String, value: String },
    SetBodyField { key: String, value: String },
    ToolTextRetry { extra: u32 },
}
```

- 存储：`config_dir/request-rules.json`。
- 网关集成：在请求进入协议适配（`responses_to_chat_with_meta` / `rewrite_model`）之前，根据客户端标签（`x-sugt-client` + User-Agent 识别）匹配作用域，按优先级（Global > Agent > Codex > Default）合并生效的规则，应用动作。
- **迁移**：现有 `DEFAULT_CODEX_MAX_TOKENS` 硬编码与 `tool_text_retry` 逻辑，改由"内置默认规则"驱动，行为等价，但可关、可改。
- 命令：`save_request_rule` / `list_request_rules` / `delete_request_rule`。

**验收标准**
- 关闭 codex 作用域的 max_tokens 规则后，Codex 请求不再自动填 max_tokens（用 e2e 测试验证请求体）。
- 给某 agent 开一条 `SetHeader`，该 agent 的请求带上该 header，其他客户端不受影响。
- 旧配置（无规则）升级后行为与 v1.1.10 一致（向后兼容）。
- 现有 `codex_gateway_e2e.rs` / `release_1_1_8_e2e.rs` 全部通过。

---

### R7 — 设置页：清理缓存 + 日志自动清理周期

**目标**：提供缓存清理入口，日志按周期自动清理，避免膨胀。

**产品设计（设置页 → general 子 tab）**
- 新增"清理缓存"按钮：清理模型探测结果缓存、临时文件等（不删用户配置）。
- 新增"日志自动清理周期"配置：下拉/数字输入，有默认值（如 7 天）与最大值（如 90 天）。设为 0 表示不自动清理。

**技术设计**
- 日志滚动：把 `tracing_appender::rolling::never` 改为按天滚动（`rolling::daily`），并新增一个清理任务：启动时 + 每日检查，删除超过保留期的日志文件。
- 配置：`config.toml` 新增 `[maintenance]` 段（或并入现有 settings）：`log_retention_days`（默认 7，最大 90）。
- 命令：`clear_cache`（枚举已知缓存目录并删除内容，返回清理了多少项/字节）、`get_maintenance_settings` / `set_maintenance_settings`。
- 缓存范围需开发时盘点（模型探测结果、可能的临时下载等），PRD 里先留"可枚举的缓存清单"接口。

**验收标准**
- 清理缓存后，缓存目录被清空，用户配置（provider / theme / pet 等）不受影响。
- 日志保留期生效：超过 N 天的日志文件被删除，N 天内的保留。
- 日志文件会按天滚动，不再无限增长单文件。

---

## 5. 架构与解耦设计（总览）

### 5.1 新增 Rust 模块（单一职责，snake_case）
| 模块 | 职责 | 关键类型/函数 |
| --- | --- | --- |
| `probe.rs` | LLM 连通性探测 | `ProbeReport` / `probe_llm_connectivity` |
| `model_registry.rs` | model 级启用/禁用 + 探测结果持久化 | `ModelEntry` / `ModelProbe` |
| `strategy_box.rs` | 多厂商聚合与路由策略 | `StrategyBox` / `StrategyMember` |
| `api_key_pool.rs` | 多 key 轮换 + 用量监控 | `ApiKeyPool` / `ApiKeyEntry` / `KeyUsage` |
| `request_rules.rs` | 请求改写规则 + 作用域匹配 | `RequestRule` / `RuleAction` / `RuleScope` |
| `maintenance.rs`（或并入 `logging.rs`） | 缓存清理 + 日志滚动清理 | `clear_cache` / 清理任务 |

### 5.2 关键解耦原则
- **配置与运行时分离**：静态配置（规则/策略组/key池）走 JSON 文件 + `atomic_write`；高频用量（key 用量）走内存态 + 定期持久化，避免每请求写盘。
- **网关只加一个"前置规则/选路"层**：R3/R6 都在现有协议适配之前插入，不重写 `gateway.rs` 主链路，最大化复用现有重试/换路由/错误分类。
- **provider 级与 model 级状态分离**：`ProviderConfig` 保持 provider 级，`model_registry` 管 model 级，二者通过 `provider_id` 关联，避免把 model 状态硬塞进 provider。
- **所有用户可见文案走 `src/i18n.ts`**（zh 为源，en 同步），新增主 tab / 子 tab 文案需注册 key。

### 5.3 导航与 UI 结构变更
- 主 tab（[main.tsx](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src/main.tsx#L601-L611)）新增：
  - `模型列表`（R2/R4）
  - `Key 池`（R5）
- 「规则」主 tab 内新增"请求改写规则"区块（R6）。
- 设置页 general 子 tab 新增"清理缓存 + 日志清理周期"（R7）。

---

## 6. 配置持久化设计

| 数据 | 存储位置 | 说明 |
| --- | --- | --- |
| 现有 provider / 网关设置 | `config.toml` | 不动 |
| model 启用/禁用 + 探测结果 | `config_dir/model-registry.json` | 新 |
| 策略盒子 | `config_dir/strategy-boxes.json` | 新 |
| API Key 池 | `config_dir/api-key-pools.json` | 新 |
| 请求改写规则 | `config_dir/request-rules.json` | 新 |
| 日志保留期 | `config.toml`（`[maintenance]`） | 新 |

- 所有新 JSON 文件写入走 [atomic_write / replace_with_backup](file:///Users/sunwenlong/CodeFiles/RustProjects/sugt/src-tauri/src/agent_adapters.rs) 的既有安全路径（先备份再替换，失败回滚，Windows rename 兼容）。
- 新增文件都带版本字段或默认值兜底，保证旧配置/无文件时能反序列化。

---

## 7. 测试与验收门禁

- 前端：`npm test`（theme 契约 + import smoke + TS 检查）+ `npm run build`。
- 后端：`cd src-tauri && cargo test`（含 `codex_gateway_e2e.rs`、`release_1_1_8_e2e.rs`）。
- R6 需补：作用域规则匹配的单元测试（Default/Codex/Agent/Global 优先级与合并）。
- R5 需补：key 轮换（round_robin/least_used）与用量累加的单测。
- R3 需补：策略模式选路单测（priority/weighted/round_robin/speed_threshold）。
- 新增命令都要有对应前端调用验证（本地 `npm run tauri dev` 冒烟）。

---

## 8. commit 管理约定（commit → task → PRD 可追查）

- 本文档每个需求下的 task 有唯一 id（如 T1.1）。
- commit message 格式：`type(scope): [T1.1] 摘要`，type 用 `feat/fix/refactor/test/docs`，scope 用模块名（`probe`/`model-registry`/`strategy-box`/`key-pool`/`request-rules`/`maintenance`）。
- 一个 commit 尽量对应一个 task，便于回滚与定位。
- 完成一个 task 后，回填文末「Task 清单」表（状态 + commit hash + 备注）。

---

## 9. 风险与待定项（冲突时先问作者）

1. **R3 网关数据流**：策略盒子如何精确插入现有选路链路（`rewrite_model` / `upstream_attempts` 之间），需要先出数据流设计再编码——这是最大的一块，建议单独里程碑。
2. **R5 用量 token 计数准确性**：上游不返回 `usage` 时只能估算，一期已对齐"不硬拦截"规避误伤；若未来要硬拦截需先解决计数精度。
3. **R5 key 加密存储**：现与 provider 一致明文存，若作者要求加密，需单独评估迁移成本与现有 `masked_key` 的兼容。
4. **R6 作用域匹配覆盖度**：现有客户端识别依赖 `x-sugt-client` + User-Agent，未识别到的客户端落到 `Default` 作用域，需确认这是否符合预期。
5. **R7 缓存清单**：具体哪些算"缓存"需开发时盘点，PRD 先留可枚举接口。
6. **R2 与现有 `list_provider_models` 的关系（已定）**：保留原签名不动（`ProviderModal` / `UpstreamSelector` 依赖 `Vec<String>`），R2 另开 `list_model_registry` / `set_model_enabled` / `probe_models` 三个新命令。

---

## 10. Task 清单（单一事实源，开发时回填）

| Task | 需求 | 内容 | 状态 | commit | 备注 |
| --- | --- | --- | --- | --- | --- |
| T1.1 | R1 | probe.rs + ProbeReport + probe_llm_connectivity | 已完成 | 75afd4e | 单测 3 条：除零/空回复/usage 解析 |
| T1.2 | R1 | test_provider_draft 返回 ProbeReport + 前端 toast | 已完成 | 75afd4e | 弹窗提示含延迟与 t/s，估算有标记 |
| T1.3 | R1 | 控制台单测/全测回传指标 + 测试全部限并发 | 已完成 | 7c943f0 | 3 个一批；CLI 输出格式不变 |
| T2.1 | R2 | model_registry.rs + model-registry.json 持久化 | 已完成 | 5acf3e0 | 原子写；5 条单测（默认启用/状态保留/持久化/损坏回退/往返） |
| T2.2 | R2 | ModelListPanel.tsx + 主 tab + 工具栏/表格 | 已完成 | f699be8 | 「测试全部」带二次确认弹窗 |
| T2.3 | R2 | 禁用联动控制台模型选择隐藏 | 已完成 | f699be8 | 禁用即隐藏，列表 tab 仍展示可恢复 |
| T2.4 | R2 | probe_models 批量测试 + 并发控制 | 已完成 | 5acf3e0 | 全量测当前服务商，3 条一批 |
| T3.1 | R3 | strategy_box.rs 数据模型 + 持久化 | 已完成 | 4402541 / a1dc724 | 原子写；校验四类；12 条单测 |
| T3.2 | R3 | 策略模式选路逻辑（4 模式） | 已完成 | 4402541 | priority 升序 / 轮转 / 加权无放回 / 阈值排尾，链长 8 |
| T3.3 | R3 | 网关选路集成 + 控制台可选策略盒子 | 已完成 | 4402541 / a1dc724 | 三入口接入 + 健康度回写 + 7 条网关测试 |
| T4.1 | R4 | get_local_access_info + 接入信息/复制 UI | 已完成 | f5782d0 / b0f6737 | 4 条单测：回退/首条 sutai/去重/停用不收 |
| T5.1 | R5 | api_key_pool.rs 数据模型 + 持久化 | 待开发 | | |
| T5.2 | R5 | key 轮换 + 网关埋点 + 用量监控 | 待开发 | | |
| T5.3 | R5 | Key 池主 tab UI + 用量展示/告警 | 待开发 | | |
| T6.1 | R6 | request_rules.rs 数据模型 + 持久化 | 待开发 | | |
| T6.2 | R6 | 作用域匹配 + 动作应用 + 硬编码迁移 | 待开发 | | |
| T6.3 | R6 | 规则 UI（规则 tab 区块） | 待开发 | | |
| T7.1 | R7 | 日志滚动 + 清理任务 + 保留期配置 | 已完成 | a5d2018 / cd546e1 | daily 滚动；0-90 天；启动 + 每 24h |
| T7.2 | R7 | clear_cache + 设置页 UI | 已完成 | a5d2018 / cd546e1 | 只清技能仓缓存与探测结果；ConfirmModal 二次确认 |
