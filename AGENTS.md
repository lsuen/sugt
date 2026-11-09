# AGENTS.md — guidance for coding agents working on SUGT

## Product snapshot

SUGT is a local AI gateway + skills console (Tauri 2 + React + Rust/Axum). Prefer small, reversible changes. Do not invent private packaging scripts or closed-source paths.

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
- Status bar default **on** (`sugt_status_bar`); solid log drawer from the bar — not a glass overlay. Full logs live in Settings → Logs (the status-bar "open logs" action jumps straight to that sub-tab).
- **Window size compensates for the status bar.** The status bar is a second grid row (`--status-bar-h: 34px`), so `tauri.conf.json` starts at `1040×714` (680 + 34) with `minHeight: 674` and `maxHeight: 960`. `window_state.rs` persists the user's size in `config_dir/window-state.json`: restore before `show()`, debounce-save on `Resized`, clamp to the config min/max and to the current monitor's usable area, and ignore the restore "echo" so a smaller monitor never overwrites a larger saved size. Any failure degrades to the config default. Keep the four constants in `window_state.rs` in sync with `tauri.conf.json`.
- **Desktop companion (小su) poses are single-layer.** Each `.pet-loop[data-loop]` / `.pet-poster[data-frame]` rule in `src/pet/pet.css` must reference exactly one matching WebP. Multiple `background-image` values **stack, they are not a fallback** — an alpha animation on top shows a second figure through its transparent pixels. Adding a pose is a four-place edit: `src/pet/pet.css` → `src/assets/pet/` (+ `ASSETS.md`) → `docs/孙测测/scripts/build_pet_preview.py` (`FRAME_NAMES`/`LOOPS`/`STATES`/`BUBBLES`/`SIZES`) → `check_pet_overlay.py` (`PET_POSES`/`D04`/`D06`), then regenerate the preview and run `npm run build`. Companion settings live on the `pet` sub-tab (`PetSettingsSection.tsx`); the gaze loop must cancel in-flight polls on effect cleanup and skip IPC when `document.hidden`. Frontend pet code lives in `src/pet/` (`config.ts` types, `PetStage.tsx`, `PetContextMenu.tsx`, `pet.css`, `hooks/`); backend pet commands live in `src-tauri/src/pet.rs`. The pet window is **hard-zero background**: `overlay.html` + `.pet-stage` must keep `background: transparent !important` (no halo plate, no ground shadow, no stage fill). Preview pages may use a checkerboard **outside** the dashed window to simulate wallpaper; the dashed `.win` / `.pet-stage` itself must stay transparent. Element-level click-through: `overlay::apply` starts pet mode click-through and does not re-assert ignore on config updates; `usePetGaze` flips `pet_set_clickable(true)` only when the cursor is inside the **figure or menu hitbox** (or during a drag)—never because `menuOpen` alone, or the empty side gutter becomes a transparent click wall. Right-clicking the figure opens a quick-action menu (settings / mute / roam / size / dock / hide); Esc or leaving the hitbox closes it.

## Backend / gateway

- Prefer existing Axum routes and config.toml hot-reload patterns.
- Codex takeover uses `model_providers.sugt` + `wire_api=responses` (see `codex_config.rs`).

### Codex Responses tool compatibility

A working Chat Completions bridge does **not** prove Responses compatibility. Keep the full tool loop working:

- `shell_command` is an official JSON-argument **function** tool. Never classify it as a freeform `custom` tool; classify by the request's tool declarations (`ChatToolMeta`), not by a hardcoded name list.
- Flatten `namespace` tools to `<namespace>__<name>` for Chat upstreams and restore the client name on the way back via `ChatToolMeta.flattened_names`.
- Never silently drop unknown `tools` (degrade schema-bearing ones to `function`, log the rest) and never stringify structured tool items (`mcp_call`, `*_output`, etc.) into user text. Pair orphan `tool_calls` with a `role=tool` result before sending upstream.
- Emit the complete SSE lifecycle: `response.in_progress`, `response.function_call_arguments.done` / `response.custom_tool_call_input.done`, `response.completed`, and a Responses-shaped `error` object on failure.
- **Degrade, never fake success.** An empty/invalid upstream body or a 4xx reply must produce a Responses-shaped `failed` response, not an empty `completed`. Fields the stateless gateway cannot forward (`previous_response_id`, `store`, `reasoning`, `include`, …) go into `ChatToolMeta.dropped_request_fields` with a warning instead of vanishing silently. Map `max_output_tokens` → `max_tokens`.
- **Streaming is hostile input.** Accumulate SSE bytes (not decoded strings) across chunks, never slice `&str` at a fixed byte offset, buffer `arguments` fragments until the tool name arrives so `output_item.added` always precedes `arguments.delta`, and give anonymous tool calls a resolvable fallback name at finalize time.
- `instructions` items in `input` become `system` messages; unknown items degrade to their plain text, never to a serialized JSON blob.
- Validate with `src-tauri/tests/codex_gateway_e2e.rs` (real HTTP + mock upstream, streaming and non-streaming) and `src-tauri/tests/codex_edge_cases.rs` (malformed/edge payloads). A passing text-only request is not sufficient.
- **Streaming and non-streaming error detection must share one predicate.** Use `is_real_error_value` for both `is_openai_compat_error_body` and `upstream_error_message`: a chunk carrying `error: null`, `""` or `{}` is a normal response, never a `response.failed`. Fixing only the non-streaming path silently breaks SSE event ordering.
- **Every `/v1/responses` failure exit must be Responses-shaped.** Wrap `proxy_responses_via_chat` errors at the `v1/responses` branch of `proxy_request` (non-streaming → `status: failed` JSON, streaming → `response.failed` SSE). Never let an exhausted attempt, 5xx or circuit breaker fall through to the generic `json_error(502)` handler.
- **Streaming proxy must not set a total request timeout** on the upstream HTTP client (a 600s wall clock previously mid-cut Codex tool loops). Use connect timeout + TCP keepalive only; short chat/probe helpers may use `build_upstream_client_with_timeout`.
- **Never strip `tools` proactively** (including experimental free channels). Match cc-switch / official gateway practice: convert Responses ↔ Chat while preserving tools; only enter command-mode after the upstream explicitly rejects function calling (`tools_unsupported_hint`), then retry once on the same attempt.
- **Store Codex/Claude one-click takeover** must call `install_clients_env` / `uninstall_clients_env` (writes env + `~/.codex/config.toml`). Template-page `apply_takeover_profile` is a separate path — do not substitute it in the Store panel.

## Agent adapters (single source of truth)

- All agent fingerprints, config capabilities and skill directory candidates live in `src-tauri/src/agent_adapters.rs`. Never hardcode `~/.claude/skills` etc. in a second module; derive from `by_id()` / `skill_dir_candidates()`.
- `ConfigCapability.writable_fields` describes what SUGT **actually writes**, not what a client officially supports. Do not claim config-file writes for clients that only use env vars.
- `preview_takeover` / `list_agent_capabilities` are read-only. Before applying a takeover, show the impact preview. SOUL gets the capability brief through `system_prompt` only — it must never write client config files itself.
- Codex config writes are backed up and snapshotted; `apply_codex_config` fails on invalid TOML instead of overwriting. Keep `clear_sugt_codex_overrides` byte-for-byte revertible.
- On takeover release, restore the three scalars SUGT unconditionally overwrites (`model`, `preferred_auth_method`, `cli_auth_credentials_store`) **from the snapshot**, never by matching their values: only touch a field whose current value equals what SUGT wrote, restore the pre-takeover value when present, delete only when it was absent. Users often had these fields before takeover; deleting them by value silently wipes their config. With no snapshot, leave the auth fields alone — residue is safer than data loss.
- `auth.json` is merged, not replaced: preserve existing ChatGPT login fields. Declare third-party capabilities (`requires_openai_auth`, `capabilities.remote_compaction`, `web_search`) so the client does not request hosted tools the gateway cannot serve.
- Codex `model` uses a `sugt-` alias so a same-named upstream model does not inherit the official `code_mode_only` / `use_responses_lite` tool protocol. Keep it switchable via `SUGT_CODEX_MODEL_ALIAS=off` and idempotent. Do **not** generate `model_catalog_json` from scratch — the published workaround exports the real catalog first, and a minimal hand-written catalog would replace the client's entire built-in model list.
- SUGT never writes or removes `mcp_servers`. `read_mcp_resource` / `list_mcp_resources` are client-side tools and only work when the client has its own MCP servers configured; say so in the preview and SOUL brief.
- **No snapshot does not mean "never clean".** v1.1.7 shipped without any snapshot mechanism and unconditionally overwrote `model`, `preferred_auth_method`, `cli_auth_credentials_store` and the whole `auth.json`. Detect that exact legacy shape (`model_provider=sugt` + unaliased `model` + provider without `requires_openai_auth`/`capabilities` + no injected `web_search` + no state file) and prune only the values SUGT is known to have written. For any other snapshot-less shape, keep the conservative "residue is safer than data loss" rule. `prune_sugt_auth_fields` must key off a deterministically-owned value (`auth_mode=apikey` plus a gateway key equal to the current one or `sugt-` prefixed) and preserve all other login fields.
- **`atomic_write` must never delete before replacing.** All platforms go through `replace_with_backup`: move the original to a pid-scoped backup, rename the temp file in, restore the backup on failure and clean the temp file. Windows `rename` cannot overwrite an existing target; the old "remove then rename" lost user config when rename failed.
- Regression suite for the release path lives in `src-tauri/tests/release_1_1_8_e2e.rs` (real HTTP gateway + mock upstream returning 4xx/5xx) alongside the existing Codex suites.
- **The agent catalog has one source, not two.** `agent_discover::known_agents()` derives the full client list from `agent_adapters::all()`; `fingerprints()` only holds legacy probe details (env var templates). An empty/short discovery query must return the whole known-client catalog, not `Vec::new()`. New domestic/global clients go into `ADAPTERS` only; clients with unconfirmed skill dirs use `GENERIC_CAPABILITY` and always include `~/.agents/skills` as fallback. Never claim a config-file write for them.
- **SOUL is a read-only orchestrator.** `soul_context::soul_brief()` injects runtime facts (capability matrix, taken-over clients, local skills, repos) into `system_prompt_with_brief()`. The model may only emit one `[[SUGT_ROUTE]]<intent>[[/SUGT_ROUTE]]` line (`clients` / `skills` / `preview` / `settings`), which the frontend renders as suggestion buttons. Never give SOUL a tool loop, never let it emit bash for management intents, never let it write client configs, clone repos or install skills. All writes go through the existing deterministic commands plus `TakeoverPreviewModal` impact preview and explicit user confirmation. Unknown/partial route tokens must be stripped, never executed or shown.
- **GitHub URL skill install is deterministic and transactional.** `store::github_install` reuses `parse_git_url` → host allowlist → `git clone --depth 1` (proxy-aware, `main`→`master` fallback) → `SKILL.md` scan → `replace_dir` → `staged.json`. It never registers a repo in `repos.toml`, caps the skill count, cleans the temp clone, and on failure restores `staged.json` and removes already-written dirs. It is idempotent (same URL overwrites the same ids). The frontend must guide users to Skills → Local Skills for mounting afterwards.
- **Trigger words and skill advice are rule-based, with LLM as optional explanation only.** `catalog::parse_skill_meta()` reads frontmatter (`triggers` / `触发词` / `when_to_use`, comma-separated or YAML list) and emits deterministic suggestions. Never invent trigger words from thin air; missing metadata produces an empty list plus an actionable suggestion. These still work when the model is unavailable.
- **Navigation:** Logs and About live under Settings as horizontal sub-tabs (`overview` / `general` / `appearance` / `advanced` / `pet` / `logs` / `about`), with `overview` first and `.settings-tabs` sticky. Desktop companion (小su) settings live on the `pet` tab between Advanced and Logs. The sub-tab state is owned by `App` so the status-bar drawer can jump straight to `logs`. High-frequency log polling is gated on the logs sub-tab being visible; the drawer keeps its own timer — do not double-poll.

## Release & packaging

- **Release asset names are stable and version-free.** Only the git tag carries a version. Builds always upload `SUTAI-windows-x64-setup.exe`, `SUTAI-windows-x64.zip`, `SUTAI-macos-arm64.zip`, `SUTAI-macos-x64.zip`, `SUTAI-linux-x64.tar.gz`, `SUTAI-linux-arm64.tar.gz` and `SUTAI-checksums.txt`. Never put a version in an asset filename again: the download site, the Homebrew cask and the in-app updater all resolve `/releases/latest` by filename, so a versioned filename forces a coordinated edit on every release.
- **Tags stay semantic (`v1.1.9`).** `update::check_for_update` compares `release.tag_name` with the bundled `CARGO_PKG_VERSION`; a fixed `latest` tag parses to `0.0.0` and silently disables update checks. Use `v*` tags, never a rolling tag.
- **One workflow, four build legs.** `.github/workflows/release.yml` runs on every `v*` tag push (and manual dispatch with an explicit tag): `windows-latest` builds NSIS + portable zip, `macos-15` / `macos-15-intel` build both macOS arches, `ubuntu-latest` / `ubuntu-24.04-arm` build **headless `sugt-cli`** tarballs (no Linux desktop GUI yet); a final `publish` job merges artifacts, writes checksums and uploads everything. CI never depends on the author-only `/scripts` PowerShell files (gitignored), and `package.json` `package:release*` entries remain local conveniences.
- **LAN / server mode is opt-in, auth-hardened.** Default bind stays `127.0.0.1`. `sugt-cli serve --lan` (or host `0.0.0.0` / `allow_lan_access`) binds non-loopback and **requires** a matching `gateway_client_api_key` on every request; missing headers are only tolerated on loopback for local Codex. Ship `deploy/sugt-gateway.service` as the systemd sample — do not make LAN the Linux platform default.
- **Homebrew is a rolling release.** `lsuen/homebrew-sugt` uses `version :latest` + `sha256 :no_check` + `releases/latest/download/<fixed-name>`; publishing a new SUGT version needs no tap change. Integrity is covered by `SUTAI-checksums.txt` and the GitHub asset `digest` used by the in-app updater. Users must run `brew upgrade --cask --greedy sugt` (or use in-app updates) because `:latest` casks are skipped by plain `brew upgrade`.
- **Download site reads `/releases/latest` too.** `lsuenweb` maps kinds to explicit asset names in `assets/projects.toml` and falls back to substring rules. Once names are stable, a new release only replaces same-named assets — neither repository needs a code or link change.
- **Version lives in six places**, not four: `package.json`, `package-lock.json` (two keys), `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, `src/release-notes.ts`. Run `python3 docs/孙测测/scripts/check_version_consistency.py --root .` before tagging.
- **Changing an asset name is a multi-place edit**: `release.yml` env, this section, both READMEs, the Homebrew cask (mac only), and `lsuenweb/assets/projects.toml`. Do it together or not at all.
- **Never declare platform-specific CLI binaries in devDependencies.** `@tauri-apps/cli` already installs the matching binary via optionalDependencies; pinning `@tauri-apps/cli-win32-x64-msvc` makes `npm ci` fail with `EBADPLATFORM` on macOS/Linux and kills the macOS release legs. Regenerate the lockfile with `npm install --package-lock-only` after any dependency edit.
- **Unit tests must be OS-neutral.** CI runs `cargo test` on `windows-latest`: build absolute paths from `std::env::temp_dir()` instead of `/tmp/...` literals, and cfg-gate POSIX-only shell commands (PowerShell's `pwd` prints a table, not a path).

## Verification

```bash
npm test                      # theme contract + import smoke + TypeScript check
npm run build                 # frontend production build (dual entry)
cd src-tauri && cargo test    # Rust lib + integration tests
```

`npm test` runs `test:themes`, `test:theme-pack` and `check` in order. CI runs `npm test` and `cargo test` (full suite, not just `--lib`). `cargo test` includes `tests/codex_gateway_e2e.rs`, which spins up a real gateway router and a mock Chat upstream to verify the two-turn Codex tool loop over HTTP. One macOS `launchctl` test is `#[ignore]`d; run it on a real GUI session with `cargo test -- --ignored`.
