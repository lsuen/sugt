# Theme pack — agent / human guide

This folder is a **portable SUGT skin**. Open it in any agent (Cursor, Claude, Soul, …) and ask it to restyle the pack; then zip the folder and import in **Settings → Appearance**.

This starter is **color-only on purpose**: it ships no `bg.jpg`, so it imports cleanly as-is. Add the file before declaring it, or `npm run test:themes` will fail.

## Layout

```
theme-packs/<id>/
  theme.json     # required — version 1 contract
  bg.jpg         # optional background (jpg/png/webp, ≤ ~1.6MB); not included here
  AGENTS.md      # this file (optional but recommended)
```

Zip for users: include `theme.json` + `bg.*` at the archive root or one level deep.

## Rules (do not break)

1. `version` must be `1`.
2. `id` = `[a-z0-9][a-z0-9_-]{0,47}` (stable, unique).
3. `label.zh` + `label.en` required.
4. Only keys in the SUGT token contract (~40). **No** `--foo-2` / component class overrides.
5. Missing tokens are filled from SUGT defaults on import; prefer filling all for predictable look.
6. Prefer translucent `--surface*` + `--glass-blur` for glass; set `--glass-blur: 0px` to disable.
7. Background: add `bg.jpg` first, then set `assets.bgImage` and `--bg-image: url("./bg.jpg")` + `--bg-image-opacity` (0–1). If you declare an asset that is not in the folder, the validator fails and the import falls back to a flat color.

## Contract keys

See `docs/THEME_AUTHORING.md` / `docs/THEME_AUTHORING.zh.md` and `src/theme/schema.ts` (`THEME_TOKEN_KEYS`).

## Prompt starters

- “Based on this theme.json, make a soft anime-girl dusk pack; keep the contract; suggest bg.jpg mood.”
- “Shift accent to teal, denser surfaces for readability over busy art.”
- “Export a zip-ready folder; do not invent extra CSS files.”

## Import in app

Settings → Appearance → **Import pack** (`.zip` / `theme.json`) or **Import folder**.
