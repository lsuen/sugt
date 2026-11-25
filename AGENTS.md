# AGENTS.md — guidance for coding agents working on SUGT

## 开发规则总索引

`public_code_rules.md`（项目根目录，**仅本地保留、不同步公开仓库**）是维护者与 agent 的内部开发契约：协作与写入、代码与数据、主题与 UI、日志、版本管理与发布、测试验收、命名与文档。本文与其余文档的规则条目以该文件为准；发现冲突时更新该文件并同步本文。公开仓库的协作者按 AGENTS.md 与本文各节执行。**Git 已获用户授权直接操作**，不再需要事前确认。

## 协作模式：指挥官模式 (Commander Mode)

本项目采用「指挥官模式」——AI 作为项目指挥官负责拆解、分派、验收、汇总，而非直接执行。子 Agent 接到明确任务指令（输入 / 预期输出 / 验收标准 / 时限）后独立执行，父代理汇总结果并验证后再采信。同一策略连续失败 3 次须切换方案。受阻时汇报「卡点 + 已试 + 需要什么决策」。收尾总结四项：做了什么 / 产出物 / 遗留问题 / 下一步。

## How to read this document (2026-11-11)

Every item below is an **expectation grounded in measured experience**, not a prohibition. Each one states the outcome we want and the reason behind it. When a change needs to deviate, evaluate the impact, run the relevant gates, and note the deviation — wording alone is not a blocker. Please phrase new entries as "Expect X because Y (verify with Z)" rather than "never / must / forbidden". Entries about security, data loss, licensing and release-asset naming still spell out their consequences because those are objective facts.

## Notice model (1.2.0)

**Expect** every UI notice — main-window toasts and companion bubbles — to share one vocabulary in `src/notify/messageModel.ts`: category (`state|result|idle|interaction`), severity, dedupe key, TTL and priority. `src/notify/useNoticeQueue.ts` owns the React queue (merge, prune, pause-on-hover, visible slice). Expect new producers to call `pushNotice` / `pushToast` rather than maintaining a second list, because two independent structures previously drifted apart; verify with `npm run test:notice-model` and `npm test`.

**Expect** state messages to come only from real in-flight gateway statistics (`canEmitStateNotice`): zero in-flight, short requests and cooldown windows stay silent, because a fake "working" status is worse than no status. Expect idle lines to follow the current pose (`IDLE_MESSAGE_KEYS`) and interaction lines to follow the intent (`INTERACTION_MESSAGE_KEYS`).

**Expect** `check_pet_overlay.py` D03 to scan the overlay entry **plus** the shared chunks it statically imports, because the notice model is bundled into a shared `messageModel-*.js` chunk. Keep both copies of that script (`docs/孙测测/scripts/` and `docs/小精灵维护/scripts/`) byte-identical; `petctl.py doctor` checks their sha256.

**Expect** destructive deletes to use `src/ConfirmModal.tsx` (ModalPortal, Esc, overlay click, focus return, consequence text) instead of `window.confirm`, because native dialogs do not follow the theme and cannot explain consequences. The read-only companion status dialog lives in `src/pet/PetInfoModal.tsx` and is opened by double click or the context-menu item.

## 派任务

**Expect** 接到任务后先判断要不要拆给多个代理，因为同一条链路由几个代理同时改会互相覆盖。只读勘查可以并行；会写文件时，一个文件只有一个写入者。父代理汇总，子代理不提交、不改写对方的结果。细则在 `.cursor/rules/task-dispatch.mdc`。

## Product snapshot

SUGT is a local AI gateway + skills console (Tauri 2 + React + Rust/Axum). Prefer small, reversible changes. Do not invent private packaging scripts or closed-source paths.

## Experimental free channel (settings ownership)

**Expect** the experimental free channel (`sugt-free`) to be configured from **网关 → 模型配置 → 该行「设置」** (`FreeChannelSettingsModal`), not from Settings → Advanced. That modal owns disclaimer acceptance, fetch-and-test for free routes, idle probe, and auto-rotate by t/s. Settings → Advanced may still restore a deleted channel and keeps **custom** fetch-and-test only.

**Expect** strategy boxes to remain an optional outer layer: they may include `sugt-free` as a member, but must not be reimplemented inside the free-channel modal. Enabling the channel (or restoring it) without `experimental_free_disclaimer_accepted` is rejected; grandfathering marks already-enabled installs as accepted on load.

## Data authenticity (2026-11-22)

Production code must get every displayed number, list and status from a real source — a command result, runtime statistic, config file or user draft. No mock/fake values, no hardcoded `0` used as a stand-in for real usage, no estimated token count presented as measured, no brand or fallback key duplicated as a literal across files. Reason: the default primary key `sugt-local-key` was excluded from usage accounting, so **接入 → 接入密钥** showed a real-looking but permanently fake "0次 0". Verify with `python3 docs/孙测测/scripts/check_no_fake_data.py --root .` (exit 0) plus the business E2E in `src-tauri/tests/client_keys_e2e.rs`. Shared literals live in one place (`src/constants.ts` / `gateway_auth.rs`), estimated values carry an `*_estimated` marker and are shown separately from measured values.

**Expect** one `request_id` to tie every log line of a gateway request, with the masked inbound key, selected upstream key, mode/model, outcome and elapsed time. Skill install, free-channel probe and overlay IPC failures must leave a log line. Verify the id appears in 设置 → 日志 via the frontend whitelist.

## Keys & auth (do not conflate)

Three distinct systems share the word “key”. Mixing them produces wrong UI and wrong docs:

| Concept | Persist | GUI | Role |
| --- | --- | --- | --- |
| **Access / inbound keys** | `config_dir/client-keys.json` | **接入 → 接入密钥** (`ClientKeyPanel`) | Keys clients present to this gateway (`Authorization` / `x-api-key`). Multi-key, enable/disable, per-key usage + client/IP detail. |
| **Primary inbound key** | also mirrored to `AppConfig.gateway_client_api_key` | same panel — row with `primary: true` | Cannot delete/disable. Editing it writes `config.toml`, refreshes access-info card, and is what LAN WebUI login + takeover scripts use. `ensure_primary` seeds from config (fallback `sugt-local-key`). |
| **Upstream key rotation** | key-pool file | **网关 → 上游 Key 轮换** | Provider API keys for outbound calls. Quotas/rotation only. |

**Expect** agents to edit inbound keys only via the Access Keys UI / `client_keys` commands, never by inventing a Settings “gateway key” editor. `update_gateway_settings.gateway_client_api_key` exists for programmatic sync (e.g. primary-key save) — it is not a second product surface. WebUI login hint text may still say `gateway_client_api_key`; that value is the primary access key.

LAN / non-loopback binds require a matching inbound key on every request; loopback may omit the header for local Codex. Verify with `src-tauri/tests/client_keys_e2e.rs`.

## AppConfig ↔ GUI surface

Most day-to-day knobs are in GUI. Port model (2026-11-20):

| Field | GUI | Notes |
| --- | --- | --- |
| `port` (preferred listen) | **设置 → 常规 → 网关** | User intent. Changing it rebinds a running gateway. |
| Runtime listen port | Console / access banner when ≠ preferred | From embedded bind or candidate scan; **must not** be persisted over `config.port`. `sync_client_endpoints_to_listen_port` only rewrites takeover scripts. |
| `port_fallback_enabled` | Settings → General | keep |
| `port_fallback_ports` | **设置 → 高级** (comma list; empty → defaults `9878/18887/28887`) | |
| `gateway_client_api_key` | **接入 → 接入密钥** (primary) | do not duplicate under Settings |
| `upstream_retry_budget_ms` / `upstream_retry_single_ms` | **设置 → 高级 → 上游韧性** | blank = defaults (8 min / 60 s) |
| `allow_lan_access` / WebUI enable+port / watchdog / failover / fetch-and-test | Settings | keep |
| `host` | derived from LAN toggle | do not expose separately |
| Takeover flags / providers / rules | their own tabs | keep |
| Config presets (`config-presets.json`, `presets.rs`) | **设置 → 配置预设** | three built-ins + user snapshots; applying overwrites only the fields declared in `PresetSettings` and never exports keys |

**Expect** new config fields to land on the existing tab that owns that behaviour (Access for inbound auth, Gateway for upstream, Settings for process/bind), not a catch-all “raw TOML” page. Docs: Chinese main README is `README.md`; English is `README.en.md` (`README.zh.md` is a stub redirect).

## Theme system (skins)

**Goal:** lightweight, file-per-theme, importable packs — never a God-token CSS file.

### Rules

1. **Contract is fixed** in `src/theme/schema.ts` (`THEME_TOKEN_KEYS`, ~40 keys). Do not add variant tokens (`*-2`, `*-3`, `*-pink`).
2. **One theme = one CSS file** under `src/theme/themes/<id>.css`, or a pack folder `src/theme/packs/<id>/` (`theme.json` + assets + optional `theme.css`). Theme selectors must be `:root[data-theme="<id>"]` (not bare `[data-theme]`), or default `:root` tokens win.
3. **Register** builtins only in `src/theme/registry.ts`. Add zh/en labels in `src/i18n.ts`.
4. **Components** consume `var(--*)` via `src/theme/shell-theme.css` / `src/styles.css`. Never hardcode brand colors in new UI. Never write `[data-theme="x"] .BusinessClass` override blocks.
5. **Background / glass** use `--bg-image`, `--bg-image-opacity`, translucent `--surface*`, and `--glass-blur` (set `0px` to disable).
6. **External packs** are **JSON** (`theme.json`, `version: 1`). Not YAML/TOML. Loader: `src/theme/loadThemePack.ts`.
7. **Docs:** `docs/THEME_AUTHORING.md` + `docs/THEME_AUTHORING.zh.md`. Keep them in sync when the contract changes.
8. **Validate:** `npm test` (theme contract + import smoke + TypeScript check), then `npm run build`. The theme validator covers all builtin CSS skins, builtin JSON packs and the starter template: `:root[data-theme]` selectors, all contract tokens, declared background assets, and two-way `registry.ts` consistency. The import smoke test (`tests/theme-pack-smoke.mjs`) executes the real loader via esbuild.

### Adding a builtin theme (checklist)

- [ ] New `src/theme/themes/<id>.css` (or pack) filling every token
- [ ] `registry.ts` entry + id in `BUILTIN_THEME_IDS`
- [ ] i18n `theme.*` zh + en
- [ ] If art backdrop: place under pack folder; set `--bg-image` + opacity
- [ ] `npm test && npm run build`

### Forbidden

- Merging all themes into one giant `themes.css`
- Growing `tokens.css` with per-component shade ladders
- Theme-specific selectors against business class names
- Committing secrets, API keys, or local probe files (`.omo/`, `_me_write_probe.md`, ad-hoc `debug.sh` unless asked)

## Frontend notes

- UI copy: `src/i18n.ts` only (`t('key')`). zh is source; en must cover the same keys.
- Settings appearance UI: `src/theme/ThemeSettingsSection.tsx`
- Persist theme id: `localStorage` key `sugt_theme`
- Modals / fixed toasts: wrap with `ModalPortal` (`src/ModalPortal.tsx`) so they mount on `document.body`. Do not put `z-index` on `.app-shell > *` (traps `position:fixed`). Prefer theme tokens over hardcoded dark colors in overlays.
- User theme packs: zip/folder import + optional custom bg (`src/theme/loadThemePack.ts`, `customBg.ts`). Template for agents: `theme-packs/example/`.
- Status bar default **on** (`sugt_status_bar`); solid log drawer from the bar — not a glass overlay. Full logs live in Settings → Logs (the status-bar "open logs" action jumps straight to that sub-tab). The drawer poll effect depends only on `drawerOpen`; hold `onRefreshLogs` in a ref. A fresh inline callback in the dependency list retriggers the effect on every `setLogs` and pegs the WebView2 renderer (measured 2026-11-25). Skip `setLogs` when the joined text is unchanged, and scroll the log view with `behavior: 'auto'` — `smooth` restarts an animation on every tick.
- **Window size compensates for the status bar.** The status bar is a second grid row (`--status-bar-h: 34px`), so `tauri.conf.json` starts at `1040×730` (status-bar compensation + visual trim; historical step was 680+34=714) with `minHeight: 674` and `maxHeight: 960`. `window_state.rs` persists the user's size in `config_dir/window-state.json`: restore before `show()`, debounce-save on `Resized`, clamp to the config min/max and to the current monitor's usable area, and ignore the restore "echo" so a smaller monitor never overwrites a larger saved size. Any failure degrades to the config default. Keep the min/max (+ `DEFAULT_WIDTH`/`DEFAULT_HEIGHT`) in `window_state.rs` in sync with `tauri.conf.json`.

### Desktop companion (小su)

**Sources of truth.** `src/assets/pet/manifest.json` (v2) is the action registry and the only place pose data lives: name, poster/loop files, `frameOrder`, `frameMs[]`, playback, motion, `hitBox`, `semantic` / `trigger` / `group` / `priority` / `states`, `autoActions`. Normalization parameters and measured heights live in `docs/小精灵维护/config/normalize-baseline.json`. Current counts, byte sizes and gate tallies are recorded in `docs/小精灵维护/README.md`; expect this file not to repeat them, because copied numbers went stale five times across v1.1–v1.5.

**Adding or changing an action is a scripted transaction.** Append one manifest entry and drop its two WebP files into `src/assets/pet/`, then run `python3 docs/孙测测/scripts/gen_pet_from_manifest.py` (regenerates the `@generated:pet-assets` block in `src/pet/pet.css`, `src/pet/petManifest.generated.ts` and backfills alpha hitBoxes) → `build_pet_preview.py` → `npm test && npm run build` → `check_pet_overlay.py` → `check_pet_final_frames.py` → `petctl.py doctor`. Never hand-edit generated blocks. `PET_ACTIONS` is generated sorted by priority, `PET_ACTION_TABLE` maps over it and `PetStage.tsx` resolves via `selectPetAction`, so frontend code does not change. Scripts read the manifest through `load_pet_manifest`; never re-hardcode pose lists. The full flow is `docs/小精灵维护/README.md` 流程 B; regeneration lessons (ComfyUI prompts, chroma key, licensing) are in `docs/小精灵维护/素材生产经验.md`.

- **One WebP per layer.** Each `.pet-loop[data-loop]` / `.pet-poster[data-frame]` rule references exactly one WebP. Multiple `background-image` values stack, they are not a fallback — an alpha animation on top shows a second figure through its transparent pixels (gate C17).
- **Liveliness is transform layers, not more frames**: `.pet-figure` (facing + head gaze) → `.pet-weight` (sway + body gaze) → `.pet-motion` (breathing) → `.pet-secondary` (hair) → `.pet-loops`. Cycles come from `autoActions.liveliness.cycles` and are injected as CSS custom properties by `PetStage.tsx`; `pet.css` only carries fallbacks. Gaze uses three decreasing lerps (`eye > head > body`) writing `--gaze-*` / `--head-*` / `--body-*`; `usePetGaze` judges convergence on the slowest layer.
- **Priority strata are interaction > gateway state > fallback** (`lifted` / `landing` on top, `idle` at 0). If a state action outranks an interaction action, celebrate / disappointed / petted silently disappear. `trigger: always` marks a state action; every other trigger equals its action name and is driven by a boolean flag. Idle variants only fire when `state === 'idle'`.
- **Drag is three phases** owned by `usePetDrag` (`dragPhase`): pointer-down → `lifted`, release with movement → `landing` for one full clip, then idle. A click without movement does not play landing. **Roam = `lifted` → `landing`, never `walk`**, and `petActionFlags()` only maps roaming to lifted when idle so it never covers real typing / error feedback. On Windows, `overlay_start_drag` (`start_dragging` → `WM_NCLBUTTONDOWN`) must be invoked in the same left-button mousedown turn. Waiting for a 4px move, or sending it from a later mousemove, never starts the modal move loop. The 4px threshold only distinguishes click from drag for the landing clip. macOS does not have this constraint.
- **Triggers do not borrow each other** (`welcome` and `greeting` have separate flags and `autoActions` slots). Auto performances share `PerformanceLease` (`canBeginAuto`) so one never cuts another; click, drag, right-click and approach may interrupt.
- **Durations derive from frame data.** See the next section; do not hardcode ms.
- **Hard-zero background.** `overlay.html` and `.pet-stage` keep `background: transparent !important`; no halo plate, ground shadow, particles, sparks or `brightness` / `grayscale` filters. The pet window keeps `overflow: hidden`. Bubbles and menus are flat theme colours without `box-shadow` / `backdrop-filter`.
- **Transparent areas always pass clicks through.** `pet_set_clickable(true)` only while the cursor is inside the figure alpha hitBox (`PET_HITBOXES` via `figureHitRect`, which must track the CSS `center bottom` alignment) or during a drag. `pet_click_through` survives only for parsing old configs.
- **Right-click opens a separate `pet-menu` window** (`pet_open_menu`, `menu.html`) above the figure. `pet_open_menu` is an **async** command: a sync command that builds a WebView2 window deadlocks, because `CreateCoreWebView2Controller`'s completion callback needs the main thread the command is holding. The symptom is "right-click does nothing". Debouncing `pointerdown` + `contextmenu` does not fix that deadlock. An already-open menu is focused, not toggled closed. Pick, Esc or clicking elsewhere destroys it; do not route through `close`, because the main window's `CloseRequested` handler intercepts it. Hiding from the menu or from Settings both go through `petEnabledPatch`.
- **Window presets stay height-dominant.** `PET_SIZES` in `src/pet/config.ts` is wider than tall in the same ratio as the canvas, so `background-size: contain` is driven by height. Widening the canvas without widening the presets shrinks the figure ~20%. Size changes write `sugt.pet.size.v2`; docking resets coordinates to `-1`. Presets must fit the backend width/height validation (gate C10).
- **Asset normalization never crops the character.** Pure scale + translate from the union box; geometry changes rerun the offline `--phase frames` step instead of regenerating in ComfyUI.
- Pet frontend lives in `src/pet/` (settings tab `src/PetSettingsSection.tsx`); backend commands in `src-tauri/src/pet.rs`. The gaze loop cancels in-flight polls on cleanup and skips IPC when `document.hidden`.

### Pet action duration — derive from frame data, never hardcode ms（2026-11-11）

**Expect** every visual dwell time to be derived from the clip itself: `poseDurationMs(pose)` sums `PET_FRAME_MS[pose]` from `petManifest.generated.ts`, and `poseHoldMs(pose, { cycles, tailMs })` turns "play it N times, then linger" into milliseconds. `resultHoldMs('done'|'error')` extends the same rule to result-state windows. `autoActions` now declares `holdCycles` / `tailMs` instead of `holdMs`, so swapping a WebP re-derives the timing automatically.

The reason is measured: idle variants are 3924–5018 ms long, but `idleVariant.holdMs` was `2200` — every idle animation was cut at 44–56% and snapped back to the base pose, which read as "the animation ends halfway". The same defect sat in `wake` (1600/4390), `celebrate` (3500/4795), the triple-tap celebrate (1400/4795), `disappointed` (3000/4795), `welcome` (6000/4390 — over-hold), the `done` state window (3500 < 4795) and `landing` (420/4390 — only the crouch played).

Two deliberate exceptions stay short because the clips are seamless loop "state poses", not narratives: `petted` (720 ms) and `poked` (1600 ms) — first and last frames match, amplitude is small, and cutting them is a rhythm choice rather than a truncation. Write the reason at the call site when you add another exception.

Verify with `npm run test:pet-state` (three assertions cover the derivation, the `error`/`done` windows, and the "no idle variant shorter than its clip" regression) plus the C37b / C46b / C38 gates, which now assert `poseHoldMs('landing'` and `autoHold('celebrate'`.

## Windows runtime（2026-11-25）

macOS 上能复现的时序问题（连发右键、`setx` 堵 async worker）不能直接当成 Windows「没反应 / 假死」的根因。1.2.3 在真机上量到的是下面三条；`setx` 进 `spawn_blocking`、值未变就跳过，仍然要留，但不是首启卡死的主因。

- **关着的回环口探测。** 安全软件下，没人听的 `127.0.0.1` 要大约 2 秒才拒绝连接。串行扫候选口再叠状态轮询，启动会被钉住约 10 秒。探测客户端连接超时 250ms、候选口并行；没有独立网关 PID 就不扫备选口；先看嵌入网关的内存状态。`wait_reachable` 保持短次数，不要回到 20 次。
- **退出。** `app.exit(0)` 只往事件循环投 `RequestExit`。主线程或 WebView2 卡住时这条消息不会被处理。一点退出就置 `commands::QUITTING`，watchdog 停止把网关拉起来；独立线程 6 秒后进程还在就 `process::exit(0)`。只给清理加超时不够，卡点在 `exit` 之后。
- **状态栏日志抽屉自激**见上面 Frontend notes。排障先看谁占着主线程、谁在空转渲染，再改调用方式。

## Backend / gateway

- Prefer existing Axum routes and config.toml hot-reload patterns.
- Codex takeover uses `model_providers.sugt` + `wire_api=responses` (see `codex_config.rs`).

### Codex Responses tool compatibility

A working Chat Completions bridge does **not** prove Responses compatibility. Keep the full tool loop working:

- `shell_command` is an official JSON-argument **function** tool. Never classify it as a freeform `custom` tool; classify by the request's tool declarations (`ChatToolMeta`), not by a hardcoded name list.
- Flatten `namespace` tools to `<namespace>__<name>` for Chat upstreams and restore the client name on the way back via `ChatToolMeta.flattened_names`.
- Never silently drop unknown `tools` (degrade schema-bearing ones to `function`, log the rest) and never stringify structured tool items (`mcp_call`, `*_output`, etc.) into user text. Pair orphan `tool_calls` with a `role=tool` result before sending upstream.
- Emit the complete SSE lifecycle: `response.in_progress`, `response.function_call_arguments.done` / `response.custom_tool_call_input.done`, `response.completed`, and a Responses-shaped `error` object on failure.
- **Degrade, never fake an empty success.** An empty or invalid HTTP 200 body, and an empty SSE stream, still become a Responses-shaped `failed` response, not an empty `completed`. A stream that already emitted text closes as `completed` with a notice, because `response.failed` after visible output drops the agent session. Quota, 5xx and free-route exhaustion are a different exit: after the in-request retry budget they become a completed assistant notice (see `.cursor/skills/sugt-upstream-resilience/SKILL.md`). Fields the stateless gateway cannot forward (`previous_response_id`, `store`, `reasoning`, `include`, …) go into `ChatToolMeta.dropped_request_fields` with a warning instead of vanishing silently. Map `max_output_tokens` → `max_tokens`.
- **Streaming is hostile input.** Accumulate SSE bytes (not decoded strings) across chunks, never slice `&str` at a fixed byte offset, buffer `arguments` fragments until the tool name arrives so `output_item.added` always precedes `arguments.delta`, and give anonymous tool calls a resolvable fallback name at finalize time.
- `instructions` items in `input` become `system` messages; unknown items degrade to their plain text, never to a serialized JSON blob.
- Validate with `src-tauri/tests/codex_gateway_e2e.rs` (real HTTP + mock upstream, streaming and non-streaming) and `src-tauri/tests/codex_edge_cases.rs` (malformed/edge payloads). A passing text-only request is not sufficient.
- **Streaming and non-streaming error detection must share one predicate.** Use `is_real_error_value` for both `is_openai_compat_error_body` and `upstream_error_message`: a chunk carrying `error: null`, `""` or `{}` is a normal response, never a `response.failed`. Fixing only the non-streaming path silently breaks SSE event ordering.
- **Every `/v1/responses` exit stays Responses-shaped.** A body that never enters the retry loop must not fall through to generic `json_error(502)`. After the retry budget, or on a terminal upstream status, the exit is HTTP 200 with a completed assistant notice (`status: completed`, `output_text`, `error: null`, the client's model echoed). That notice is not `response.failed`. `error: null` on a real upstream 200 is still success. Expect the same policy on plain Chat and Claude `/v1/messages`, because the three wires used to disagree and agents dropped on the ones that returned 429 immediately (verify with `src-tauri/tests/release_1_1_8_e2e.rs`; set `upstream_retry_budget_ms: Some(0)` on that test's config so a 5xx does not sleep for 8 minutes). When a new disconnect, retry or model-leak pitfall shows up, update `.cursor/skills/sugt-upstream-resilience/SKILL.md` in the same change.
- **Streaming proxy must not set a total request timeout** on the upstream HTTP client (a 600s wall clock previously mid-cut Codex tool loops). Use connect timeout + TCP keepalive only; short chat/probe helpers may use `build_upstream_client_with_timeout`.
- **Never strip `tools` proactively** (including experimental free channels). Match cc-switch / official gateway practice: convert Responses ↔ Chat while preserving tools; only enter command-mode after the upstream explicitly rejects function calling (`tools_unsupported_hint`), then retry once on the same attempt.
- **A text-only 200 is a finished turn.** Codex, Trae (Chat) and Claude all stop the turn when the model returns text and no tool call. Expect the gateway, before forwarding any byte, to retry only inside that one request. The experimental free channel still uses 3, then 2, then 1 across its first three routes, then forwards the last text. A paid provider does not hardcode that ladder: `tool_text_retry_enabled` / `tool_text_retry_extra` on the add/edit model form are the extra retries after the first call (2 means 3 calls total, cap 5). Unchecked, the first text is forwarded. Configs saved before the field existed deserialize as on with extra 2; a newly added provider starts off. A tool call returns immediately and the count is not stored. The Codex bridge default output cap stays 32768 when rules do not suppress it; a short completed reply is not a max-token cut. **Non-Codex Chat passthrough does not invent `max_tokens`** — only an explicit request-rule value is written; clients that already speak Chat Completions get a thin proxy.
- **Codex Chat-bridge history must keep valid tool arguments.** Canonicalize `tool_calls[].function.arguments` to a JSON object string (strip trailing garbage, wrap non-objects), merge consecutive assistant tool-only messages, and treat free-route 400 as Switch — never Terminal that short-circuits the rest of the free ladder. Circuit trips only on auth failures; BadRequest must not lock the route for 30s.
- **Client takeover goes through takeover profiles.** The Access tab drives `apply_takeover_profile` / `release_takeover_profile` after a `preview_takeover` impact preview. The env/config writer underneath is `clients::install_for` / `uninstall_for` (tray and auto-takeover call it directly); the `install_clients_env` / `uninstall_clients_env` commands have had no GUI caller since the old Store client panel was removed.

## Agent adapters (single source of truth)

- All agent fingerprints, config capabilities and skill directory candidates live in `src-tauri/src/agent_adapters.rs`. Never hardcode `~/.claude/skills` etc. in a second module; derive from `by_id()` / `skill_dir_candidates()`.
- `ConfigCapability.writable_fields` describes what SUGT **actually writes**, not what a client officially supports. Do not claim config-file writes for clients that only use env vars.
- `preview_takeover` / `list_agent_capabilities` are read-only. Before applying a takeover, show the impact preview. SOUL gets the capability brief through `system_prompt` only — it must never write client config files itself.
- Codex config writes are backed up and snapshotted; `apply_codex_config` fails on invalid TOML instead of overwriting. Keep `clear_sugt_codex_overrides` byte-for-byte revertible.
- On takeover release, restore the three scalars SUGT unconditionally overwrites (`model`, `preferred_auth_method`, `cli_auth_credentials_store`) **from the snapshot**, never by matching their values: only touch a field whose current value equals what SUGT wrote, restore the pre-takeover value when present, delete only when it was absent. Users often had these fields before takeover; deleting them by value silently wipes their config. With no snapshot, leave the auth fields alone — residue is safer than data loss.
- `auth.json` is merged, not replaced: preserve existing ChatGPT login fields. Declare third-party capabilities (`requires_openai_auth`, `capabilities.remote_compaction`, `web_search`) so the client does not request hosted tools the gateway cannot serve.
- Codex `model` uses a `sugt-` alias so a same-named upstream model does not inherit the official `code_mode_only` / `use_responses_lite` tool protocol. Keep it switchable via `SUGT_CODEX_MODEL_ALIAS=off` and idempotent. Do **not** generate `model_catalog_json` from scratch — the published workaround exports the real catalog first, and a minimal hand-written catalog would replace the client's entire built-in model list.
- SUGT never writes or removes `mcp_servers`. `read_mcp_resource` / `list_mcp_resources` are client-side tools and only work when the client has its own MCP servers configured; say so in the preview and SOUL brief.
- **No snapshot does not mean "never clean".** v1.1.7 shipped without any snapshot mechanism and unconditionally overwrote `model`, `preferred_auth_method`, `cli_auth_credentials_store` and the whole `auth.json`. Detect that exact legacy shape (`model_provider=sugt` + unaliased `model` + provider without `requires_openai_auth`/`capabilities` + no injected `web_search` + no state file) and prune only the values SUGT is known to have written. That release wrote the raw `model_name` (aliases were empty); after public ids fell back to `sutai`, legacy cleanup still has to recognize `model_name` or the residue stays. For any other snapshot-less shape, keep the conservative "residue is safer than data loss" rule. `prune_sugt_auth_fields` must key off a deterministically-owned value (`auth_mode=apikey` plus a gateway key equal to the current one or `sugt-` prefixed) and preserve all other login fields.
- **`atomic_write` must never delete before replacing.** All platforms go through `replace_with_backup`: move the original to a pid-scoped backup, rename the temp file in, restore the backup on failure and clean the temp file. Windows `rename` cannot overwrite an existing target; the old "remove then rename" lost user config when rename failed.
- Regression suite for the release path lives in `src-tauri/tests/release_1_1_8_e2e.rs` (real HTTP gateway + mock upstream returning 4xx/5xx) alongside the existing Codex suites.
- **The agent catalog has one source, not two.** `agent_discover::known_agents()` derives the full client list from `agent_adapters::all()`; `fingerprints()` only holds legacy probe details (env var templates). An empty/short discovery query must return the whole known-client catalog, not `Vec::new()`. New domestic/global clients go into `ADAPTERS` only; clients with unconfirmed skill dirs use `GENERIC_CAPABILITY` and always include `~/.agents/skills` as fallback. Never claim a config-file write for them.
- **SOUL is a read-only orchestrator.** `soul_context::soul_brief()` injects runtime facts (capability matrix, taken-over clients, local skills, repos) into `system_prompt_with_brief()`. The model may only emit one `[[SUGT_ROUTE]]<intent>[[/SUGT_ROUTE]]` line (`clients` / `skills` / `preview` / `settings`), which the frontend renders as suggestion buttons. Never give SOUL a tool loop, never let it emit bash for management intents, never let it write client configs, clone repos or install skills. All writes go through the existing deterministic commands plus `TakeoverPreviewModal` impact preview and explicit user confirmation. Unknown/partial route tokens must be stripped, never executed or shown.
- **GitHub URL skill install is deterministic and transactional.** `store::github_install` reuses `parse_git_url` → host allowlist → `git clone --depth 1` (proxy-aware, `main`→`master` fallback) → `SKILL.md` scan → `replace_dir` → `staged.json`. It never registers a repo in `repos.toml`, caps the skill count, cleans the temp clone, and on failure restores `staged.json` and removes already-written dirs. It is idempotent (same URL overwrites the same ids). The frontend must guide users to Skills → Local Skills for mounting afterwards.
- **Trigger words and skill advice are rule-based, with LLM as optional explanation only.** `catalog::parse_skill_meta()` reads frontmatter (`triggers` / `触发词` / `when_to_use`, comma-separated or YAML list) and emits deterministic suggestions. Never invent trigger words from thin air; missing metadata produces an empty list plus an actionable suggestion. These still work when the model is unavailable.
- **Navigation:** Sidebar facade layer is `控制台` then `SOUL`, then a divider; tool layer is `网关` / `规则` / `接入` / `技能` / `设置` (`FACADE_GROUP_IDS` in `src/main.tsx`). Copy uses `控制台` (not `总览`). SOUL and Skills are first-class main groups (not leaves under overview/access); migrating old `access/skills` localStorage lands on the skills group. Gateway leaves stay separate: `models` / `modelList` / `strategyBox` / `keyPool` — never merge 模型配置 with 模型列表. Access leaves are `clients` + `clientKeys` only. Logs and About live under Settings as horizontal sub-tabs (`overview` / `general` / `presets` / `appearance` / `advanced` / `pet` / `logs` / `about`), with `overview` first and `.settings-tabs` sticky. Desktop companion (小su) settings live on the `pet` tab between Advanced and Logs. The sub-tab state is owned by `App` so the status-bar drawer can jump straight to `logs`. High-frequency log polling is gated on the logs sub-tab being visible; the drawer keeps its own timer — do not double-poll.

## Release & packaging

- **Release asset names are stable and version-free.** Only the git tag carries a version. Builds always upload `SUTAI-windows-x64-setup.exe`, `SUTAI-windows-x64.zip`, `SUTAI-macos-arm64.zip`, `SUTAI-macos-x64.zip`, `SUTAI-linux-x64.tar.gz`, `SUTAI-linux-arm64.tar.gz` and `SUTAI-checksums.txt`. Never put a version in an asset filename again: the download site, the Homebrew cask and the in-app updater all resolve `/releases/latest` by filename, so a versioned filename forces a coordinated edit on every release.
- **Tags stay semantic (`v1.1.9`).** `update::check_for_update` compares `release.tag_name` with the bundled `CARGO_PKG_VERSION`; a fixed `latest` tag parses to `0.0.0` and silently disables update checks. Use `v*` tags, never a rolling tag.
- **One release workflow.** `.github/workflows/release.yml` runs on every `v*` tag push (and manual dispatch with an explicit tag): `windows-latest` builds NSIS + portable zip (plus `sugt-cli`), `macos-15` / `macos-15-intel` build both macOS arches, `ubuntu-latest` / `ubuntu-24.04-arm` build **headless `sugt-cli`** tarballs (no Linux desktop GUI yet); a final `publish` job merges artifacts, writes checksums and uploads everything. CI never depends on the author-only `/scripts` PowerShell files (gitignored), and `package.json` `package:release*` entries remain local conveniences.
- **LAN / server mode is opt-in, auth-hardened.** Default bind stays `127.0.0.1`. `sugt-cli serve --lan` (or host `0.0.0.0` / `allow_lan_access`) binds non-loopback and **requires** a matching inbound key on every request (primary `gateway_client_api_key` or any enabled row in `client-keys.json`); missing headers are only tolerated on loopback for local Codex. Ship `deploy/sugt-gateway.service` as the systemd sample — do not make LAN the Linux platform default.
- **Homebrew is a rolling release.** `lsuen/homebrew-sugt` uses `version :latest` + `sha256 :no_check` + `releases/latest/download/<fixed-name>`; publishing a new SUGT version needs no tap change. Integrity is covered by `SUTAI-checksums.txt` and the GitHub asset `digest` used by the in-app updater. Users must run `brew upgrade --cask --greedy sugt` (or use in-app updates) because `:latest` casks are skipped by plain `brew upgrade`.
- **Download site reads `/releases/latest` too.** `lsuenweb` maps kinds to explicit asset names in `assets/projects.toml` and falls back to substring rules. Once names are stable, a new release only replaces same-named assets — neither repository needs a code or link change.
- **Version lives in six places**, not four: `package.json`, `package-lock.json` (two keys), `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, `src/release-notes.ts`. Run `python3 docs/孙测测/scripts/check_version_consistency.py --root .` before tagging.
- **Changing an asset name is a multi-place edit**: `release.yml` env, this section, both READMEs, the Homebrew cask (mac only), and `lsuenweb/assets/projects.toml`. Do it together or not at all.
- **Never declare platform-specific CLI binaries in devDependencies.** `@tauri-apps/cli` already installs the matching binary via optionalDependencies; pinning `@tauri-apps/cli-win32-x64-msvc` makes `npm ci` fail with `EBADPLATFORM` on macOS/Linux and kills the macOS release legs. Regenerate the lockfile with `npm install --package-lock-only` after any dependency edit.
- **Unit tests must be OS-neutral.** CI runs `cargo test` on `windows-latest`: build absolute paths from `std::env::temp_dir()` instead of `/tmp/...` literals, and cfg-gate POSIX-only shell commands (PowerShell's `pwd` prints a table, not a path).

## Verification

```bash
npm test                      # theme / pet / notice / css / provider guards + TypeScript check
npm run build                 # frontend production build (multi-entry)
cd src-tauri && cargo test    # Rust lib + integration tests
```

`npm test` chains every `test:*` script in `package.json` plus `check` (TypeScript). CI runs `npm test`, `npm run build`, the Python gates (`check_no_fake_data.py`, both `check_pet_overlay.py` copies, `check_version_consistency.py`) and `cargo test` (full suite, not just `--lib`). `cargo test` includes `tests/codex_gateway_e2e.rs`, which spins up a real gateway router and a mock Chat upstream to verify the two-turn Codex tool loop over HTTP. Two tests are `#[ignore]`d — the macOS `launchctl` test (needs a real GUI session) and the experimental free-channel live probe (needs network); run them with `cargo test -- --ignored`.
