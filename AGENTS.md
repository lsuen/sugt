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
- Status bar default **on** (`sugt_status_bar`); solid log drawer from the bar — not a glass overlay. Full logs stay on the Logs tab.

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

## Agent adapters (single source of truth)

- All agent fingerprints, config capabilities and skill directory candidates live in `src-tauri/src/agent_adapters.rs`. Never hardcode `~/.claude/skills` etc. in a second module; derive from `by_id()` / `skill_dir_candidates()`.
- `ConfigCapability.writable_fields` describes what SUGT **actually writes**, not what a client officially supports. Do not claim config-file writes for clients that only use env vars.
- `preview_takeover` / `list_agent_capabilities` are read-only. Before applying a takeover, show the impact preview. SOUL gets the capability brief through `system_prompt` only — it must never write client config files itself.
- Codex config writes are backed up and snapshotted; `apply_codex_config` fails on invalid TOML instead of overwriting. Keep `clear_sugt_codex_overrides` byte-for-byte revertible.
- `auth.json` is merged, not replaced: preserve existing ChatGPT login fields. Declare third-party capabilities (`requires_openai_auth`, `capabilities.remote_compaction`, `web_search`) so the client does not request hosted tools the gateway cannot serve.
- Codex `model` uses a `sugt-` alias so a same-named upstream model does not inherit the official `code_mode_only` / `use_responses_lite` tool protocol. Keep it switchable via `SUGT_CODEX_MODEL_ALIAS=off` and idempotent. Do **not** generate `model_catalog_json` from scratch — the published workaround exports the real catalog first, and a minimal hand-written catalog would replace the client's entire built-in model list.
- SUGT never writes or removes `mcp_servers`. `read_mcp_resource` / `list_mcp_resources` are client-side tools and only work when the client has its own MCP servers configured; say so in the preview and SOUL brief.

## Verification

```bash
npm test                      # theme contract + import smoke + TypeScript check
npm run build                 # frontend production build (dual entry)
cd src-tauri && cargo test    # Rust lib + integration tests
```

`npm test` runs `test:themes`, `test:theme-pack` and `check` in order. CI runs `npm test` and `cargo test` (full suite, not just `--lib`). `cargo test` includes `tests/codex_gateway_e2e.rs`, which spins up a real gateway router and a mock Chat upstream to verify the two-turn Codex tool loop over HTTP. One macOS `launchctl` test is `#[ignore]`d; run it on a real GUI session with `cargo test -- --ignored`.
