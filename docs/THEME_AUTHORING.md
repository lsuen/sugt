# SUGT Theme Authoring Guide

Create skins without touching component CSS. Themes only fill a **fixed token contract**; the shell and components consume those variables.

## Layout

```
src/theme/
  tokens.css                 # contract defaults (do not bloat)
  schema.ts                  # THEME_TOKEN_KEYS — source of truth
  shell-theme.css            # binds tokens to .app-shell / .card / .btn…
  themes/<id>.css            # one file per builtin palette theme
  packs/<id>/
    theme.json               # importable pack (recommended for custom skins)
    theme.css                # builtin pack stylesheet (optional duplicate of tokens)
    bg.jpg                   # optional backdrop
```

## Token contract (keep ~40)

See `THEME_TOKEN_KEYS` in `src/theme/schema.ts`. Highlights:

| Token | Role |
| --- | --- |
| `--bg` / `--bg-glow-*` | App wash |
| `--bg-image` / `--bg-image-opacity` | Illustration backdrop (`none` or `url(...)`) |
| `--surface` / `--surface-2` / `--surface-glass` | Panels |
| `--glass-blur` | Frosted glass (`0px` disables) |
| `--text` / `--muted` / `--accent` / `--accent-2` | Typography & brand |
| `--btn-primary-bg` / `--btn-ghost-*` / `--chip-*` | Controls |
| `--radius` / `--shadow` / `--font-ui` | Shape & type |

**Forbidden**

- Adding public tokens like `--btn-ghost-bg-2` / `--btn-active-pink-3`
- Writing `[data-theme="x"] .SomeBusinessClass { … }` patches
- Hardcoding hex colors in new component CSS (use `var(--*)`)

Private aliases **inside one theme file** are fine:

```css
[data-theme="street"] {
  --_pink: #ff2d95;
  --accent: var(--_pink);
}
```

## Builtin theme (CSS file)

1. Copy `src/theme/themes/cyber.css` → `src/theme/themes/my-skin.css`
2. Change the selector to `[data-theme="my-skin"]` and fill every contract token
3. Register in `src/theme/registry.ts` (`BUILTIN_THEME_IDS` + `BUILTIN_THEMES`)
4. Add `theme.mySkin` (or similar) keys to `src/i18n.ts` (zh + en)
5. Run `npm run test:themes` and `npm run check`

## Importable pack (`theme.json`)

Preferred for **external / user** skins. Example:

```json
{
  "version": 1,
  "id": "my-skin",
  "label": { "zh": "我的皮肤", "en": "My Skin" },
  "assets": { "bgImage": "bg.jpg" },
  "tokens": {
    "--bg": "#0a0a12",
    "--bg-image": "url(\"./bg.jpg\")",
    "--bg-image-opacity": "0.4",
    "--glass-blur": "14px",
    "...": "all THEME_TOKEN_KEYS required"
  }
}
```

- Format is **JSON** (not YAML/TOML) — easy for the browser, agents, and validation.
- Ship a folder: `theme.json` + optional `bg.jpg` / `bg.webp`.
- In Settings → Appearance → **Import theme pack**, pick the JSON. For remote packs, resolve `assets.bgImage` against the JSON URL base (see `fetchThemePack`).

Reference packs: `src/theme/packs/anime-boy/`, `src/theme/packs/anime-girl/`.

## Background, transparency, glass

```css
--bg-image: url("./bg.jpg");
--bg-image-opacity: 0.4;          /* 0 = hide art */
--surface: rgba(12, 20, 34, 0.72);
--surface-glass: rgba(12, 20, 34, 0.45);
--glass-blur: 16px;               /* 0px = flat panels */
```

The shell paints `--bg-image` on `.app-shell::before`. Cards/modals use `backdrop-filter: blur(var(--glass-blur))`.

## Agent checklist

See root `AGENTS.md` § Theme system. Short version: **new theme = new file + registry row**; never grow the God CSS.
