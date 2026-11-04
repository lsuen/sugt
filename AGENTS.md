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

## Verification

```bash
npm test                      # theme contract + import smoke + TypeScript check
npm run build                 # frontend production build (dual entry)
cd src-tauri && cargo test    # Rust lib + integration tests
```

`npm test` runs `test:themes`, `test:theme-pack` and `check` in order. CI runs `npm test` and `cargo test` (full suite, not just `--lib`). One macOS `launchctl` test is `#[ignore]`d; run it on a real GUI session with `cargo test -- --ignored`.
