# SUGT theme packs

Portable skins for **Settings → Appearance**.

| Path | Purpose |
| --- | --- |
| `example/` | Starter pack + `AGENTS.md` for humans / any coding agent |
| (your fork) | Publish zip on GitHub Releases; users import the zip |

## Quick start

1. Copy `example/` → `my-skin/`
2. Edit `theme.json` (`id`, labels, tokens). The starter is color-only; to add art, drop `bg.jpg` (≤ ~1.6MB) into the folder, declare `assets.bgImage` and set `--bg-image: url("./bg.jpg")`
3. Zip the folder (or import the folder in-app)
4. In SUGT: **导入主题包** / **导入文件夹**

Authoring contract: `docs/THEME_AUTHORING.md` · `docs/THEME_AUTHORING.zh.md`
