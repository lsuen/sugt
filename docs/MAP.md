# MAP — 工作区模块

> **协作模式**：本项目采用「指挥官模式」，AI 负责拆解 → 分派 → 验收 → 汇总，详见 `.cursor/rules/task-dispatch.mdc` 与 `user_profile.md`。桌面精灵的现行口径、脚本和门禁以 `docs/小精灵维护/` 为准。脚本索引在 `docs/孙测测/MAP.md`。

## 桌面精灵

| 模块 | 职责 | 入口 |
| --- | --- | --- |
| `src/assets/pet/manifest.json` | 动作、画布、节奏槽的唯一真相源 | `autoActions.welcome` 开场挥手，`autoActions.greeting` 靠近点头 |
| `src/pet/petState.ts` | 按优先级选动作；时长从帧数据派生；自动表演租约 | `selectPetAction` / `canBeginAuto` |
| `menu.html` / `src/menu.tsx` | 右键菜单独立小窗，不改精灵透明窗 | `pet_open_menu`（async command）/ `pet_close_menu` |
| `src/pet/hooks/usePetBehaviors.ts` | 开场、靠近、闲置、漂移 | 自动来源等租约；指针可以打断 |
| `src/pet/hooks/usePetGaze.ts` | 视线与命中 | 透明区始终穿透，只认身体、菜单、拖动 |
| `src/pet/config.ts` | 窗口档位与模式切换 | `petSizePatch` 只改宽高；`petModePatch` 在精灵和流量窗 `280×108` 之间切换 |
| `src/PetSettingsSection.tsx` / `src/menu.tsx` | 设置页和右键菜单共用上面两个补丁 | 菜单不写入档位 |
| `src-tauri/src/pet.rs` | 光标、拖动、独立菜单窗、一次性迁移 | `pet_open_menu` 必须是 async command，同步建 WebView2 窗口会死锁 |

## 实验免费通道与上游重试

| 模块 | 职责 | 入口 |
| --- | --- | --- |
| `src-tauri/src/upstream_resilience.rs` | 同一次请求内的重试、换路由、预算用尽后的会话提示、对外模型名改写 | `classify_http` / `notice_response` |
| `src-tauri/src/experimental_free.rs` | 零 Key 路由表、探测、failover 顺序；对外别名固定 `sutai` | `ROUTES`：`openrouter/free` 优先；`apply_route` |
| `src-tauri/src/gateway.rs` | Codex、Chat、Claude 三条线共用上面的策略；流式等待发 SSE 注释，非流式等待发换行 | `proxy_responses_via_chat` / `proxy_chat_upstream` / `proxy_anthropic_request` |
| `src-tauri/src/commands.rs` | 只允许改模型名和开关；保存时若已接管 Codex 会重写其 `model` 为 `sugt-sutai` | `save_provider` / `probe_experimental_free_models` |
| `.cursor/skills/sugt-upstream-resilience/SKILL.md` | 契约和踩坑。发现新的断线或模型名泄漏时，在同一次改动里回写 | 改网关重试前先读 |
| `src/UpstreamSelector.tsx` | 打开控制台只列出路由和主机；手动刷新才探测，不改首选 | `list_experimental_free_candidates` |

客户端看见的模型名是请求里的名字，没有则是 `sutai`。免费通道 429 先换下一条路由，单条上游按 Retry-After 等待，单次等待最多 60 秒，单次请求最多 8 分钟。预算用尽回 HTTP 200 的助手提示。请求放弃后不熔断下一条消息。终态错误仍按路由记 30 秒。声明了工具却只回文字时，免费通道只在这一次请求里按 3、2、1 再要，然后交回原文；Codex、Chat（Trae）、Claude 都在转发前判断。收费模型在添加/编辑页用「未调用工具时再要」自选次数（第一次之外再要几次，上限 5）。旧配置缺字段时视为勾上、次数 2。新添加的默认不勾。下一次请求按该模型自己的次数重算。

## 局域网 WebUI 管理

| 模块 | 职责 | 入口 |
| --- | --- | --- |
| `docs/局域网WebUI管理.md` | 入口、参数、鉴权、API 的唯一说明 | CLI `serve --webui` / GUI 设置开关 |
| `src-tauri/src/webui/mod.rs` | 独立 listener；CLI 显式端口启动、GUI 按配置启动，关闭则零占用 | `start_on_port` / `start_if_enabled` |
| `src-tauri/src/webui/api.rs` | RPC 路由 `/api/invoke` + 登录 + 静态托管 | `build_router` |
| `src-tauri/src/webui/auth.rs` | 网关 Key 换 HMAC 会话，改 Key 即全失效 | `issue_token` / `verify_token` |
| `src-tauri/src/webui/service.rs` | 命令名 → 底层模块调用，覆盖 P0+P1 | `dispatch` |
| `src/transport/index.ts` | 同一份 UI：桌面走 IPC、浏览器走 HTTP | `invoke` / `listen` / window stub |
| `src-tauri/tests/webui_e2e.rs` | 健康、登录、鉴权拒绝、RPC 全链路 | `cargo test --test webui_e2e` |

## 应用图标

| 位置 | 职责 |
| --- | --- |
| `src-tauri/icons/icon.png` | 512 主图。各尺寸由 `npx tauri icon` 等比例缩小 |
| `src-tauri/icons/32x32.png` 等、`icon.icns`、`icon.ico` | 窗口、安装包、Dock |
| `src-tauri/src/tray.rs` | 托盘只用 `32x32.png` |
| `src/assets/brand-logo.png` | 界面里的标志，256，同一张图缩小 |

## 本次变更（2026-11-20）

- 新增局域网 WebUI 管理层：CLI `serve --webui`（本次运行参数）与 GUI 设置开关（持久化配置）两种入口。
- 独立 axum listener 不并入网关 router；单 RPC 入口 `/api/invoke` 复用现有命令名，前端 transport 双通道共用一份 UI。
- 登录复用 `gateway_client_api_key` 换短期 HMAC 令牌；关闭时零监听、零占用。详细入口与参数见 `docs/局域网WebUI管理.md`。

## 本次变更（2026-11-13）

- 已从 `v1.1.9` 打出 `v1.1.10`（`2f09934`）并推到 master。带上的是上游保活和这套新图标。精灵 v1.5 与那两个本地 1.2.0 提交没有进这版。GitHub latest 上 7 个固定资产都在。
- 声明了工具却只回文字时，免费通道仍在同一条请求内按 3、2、1 再要。收费模型改由添加/编辑页「未调用工具时再要」决定：勾上后填写第一次之外再要几次（1–5），不勾则第一次文字直接交回。旧配置缺字段时勾上、次数 2；新添加默认不勾。字段在 `ProviderConfig.tool_text_retry_enabled` / `tool_text_retry_extra`，界面在 `ProviderModal`。`max_tokens` 维持 32768。

## 本次变更（2026-11-12）

- 应用图标换成 `0nogit-dev-source/sugt-icon.png`。桌面各尺寸与界面标志都从这张 512 图等比例缩小；托盘用 `32x32.png`。
- 派任务约定写入 `.cursor/rules/task-dispatch.mdc`：先判断要不要多代理，写入范围重叠时不并行。
- 上游失败在同一次请求内重试和换免费路由。预算用尽回 completed 助手提示，三条协议线一致。对外模型名固定为请求里的名字或 `sutai`。中途断流收成 completed；非流式等待发换行；HTTP 日期的 `Retry-After` 会换算。打开控制台不再自动探测。踩坑写在 `.cursor/skills/sugt-upstream-resilience/SKILL.md`，以后发现新问题就回写这份技能。

## 本次变更（2026-11-11）

- 开场不再把 `greeting` 和 `welcome` 绑在一起。
- 右键改尺寸写入 `sugt.pet.size.v2`，不把精灵拽回角落。
- 右键隐藏与设置页关闭使用同一套流量窗尺寸。
- 首次迁移不再把 `198` / `240` 打回 `148×168`。
- 闲置和漂移共用表演租约，鼠标动作才可以打断。
- 右键菜单曾临时扩到人物窗口外，现已改成独立小窗。
- 透明区始终穿透；去掉粒子、火花、眼神雾和立绘滤镜。
- 命中用的光标改成窗口内坐标。桌面坐标会让身体永远落在命中框外。
- 右键菜单改成独立小窗，不再把精灵透明窗拉大。点选后用 `destroy` 关掉，避免被主窗口的关闭拦截留在屏幕上。
