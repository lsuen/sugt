# SUGT 主题制作说明书

做皮肤时**不要改组件 CSS**。主题只填写固定的**令牌契约**；外壳与组件统一消费这些 CSS 变量。

## 目录结构

```
src/theme/
  tokens.css                 # 契约默认值（禁止膨胀）
  schema.ts                  # THEME_TOKEN_KEYS — 唯一真相源
  shell-theme.css            # 把令牌绑到 .app-shell / .card / 按钮等
  themes/<id>.css            # 每个内置调色板一文件
  packs/<id>/
    theme.json               # 可导入主题包（自定义皮肤推荐）
    theme.css                # 内置打包样式
    bg.jpg                   # 可选背景图
```

## 令牌契约（控制在约 40 个）

以 `src/theme/schema.ts` 的 `THEME_TOKEN_KEYS` 为准。常用：

| 令牌 | 作用 |
| --- | --- |
| `--bg` / `--bg-glow-*` | 底色与光晕 |
| `--bg-image` / `--bg-image-opacity` | 插画背景（`none` 或 `url(...)`） |
| `--surface` / `--surface-2` / `--surface-glass` | 面板层级 |
| `--glass-blur` | 毛玻璃（`0px` 关闭） |
| `--text` / `--muted` / `--accent` / `--accent-2` | 文字与品牌色 |
| `--btn-primary-bg` / `--btn-ghost-*` / `--chip-*` | 控件 |
| `--radius` / `--shadow` / `--font-ui` | 圆角、阴影、字体 |

**禁止**

- 新增 `--btn-ghost-bg-2`、`--btn-active-pink-3` 这类变体海
- 写 `[data-theme="x"] .业务类名 { … }` 大段补丁
- 新组件 CSS 里硬编码色值（用 `var(--*)`）

主题文件**内部**临时名可以：

```css
[data-theme="street"] {
  --_pink: #ff2d95;
  --accent: var(--_pink);
}
```

## 内置主题（CSS）

1. 复制 `src/theme/themes/cyber.css` → `src/theme/themes/my-skin.css`
2. 选择器改成 `:root[data-theme="my-skin"]`（必须带 `:root`，否则会被默认令牌盖住），填满契约
3. 在 `src/theme/registry.ts` 注册（`BUILTIN_THEME_IDS` + `BUILTIN_THEMES`）
4. 在 `src/i18n.ts` 中英字典补 `theme.*` 文案
5. 跑 `npm test`（主题契约 + 导入冒烟 + 类型检查）

## 可导入主题包（`theme.json`）

自定义 / 外载皮肤推荐 JSON：

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
    "--glass-blur": "14px"
  }
}
```

- 格式固定为 **JSON**（不用 YAML/TOML），方便浏览器与 Agent 校验。
- 目录：`theme.json` + 可选 `bg.jpg` / `bg.webp`（建议 ≤ 1.6MB）。仅当文件确实在目录中时才声明 `assets.bgImage`。
- 设置 → 外观：
  - **导入主题包**：`theme.json` 或 **`.zip`**（含背景图会嵌入本地持久化）
  - **导入文件夹**：选中含 `theme.json` 的目录
  - **自定义背景图**：只换插画、不改配色（叠在当前主题上）
- 导入包缺省令牌会用默认契约补全；未知 `--*` 键会报错。
- 远程包用 JSON URL 的目录作为资源基路径（见 `fetchThemePack`）。

模板与 Agent 说明：`theme-packs/example/`（可整夹交给任意 Agent 改皮；模板为纯配色，需先放入 `bg.jpg` 再声明）。  
内置参考：`src/theme/packs/anime-boy/`、`src/theme/packs/anime-girl/`。

## 背景图、透明、毛玻璃

```css
--bg-image: url("./bg.jpg");
--bg-image-opacity: 0.4;
--surface: rgba(12, 20, 34, 0.72);
--surface-glass: rgba(12, 20, 34, 0.45);
--glass-blur: 16px;
```

壳层在 `.app-shell::before` 绘制背景图；卡片/模态使用 `backdrop-filter: blur(var(--glass-blur))`。

## 校验

`npm run test:themes` 会校验内置 CSS 皮肤、内置 JSON 包与模板包：`:root[data-theme="<id>"]` 选择器、38 个契约令牌齐全、无契约外令牌、声明的背景资源存在，以及 `registry.ts` 与源文件的双向一致性。用户自行导入的包允许缺省令牌（导入时由默认值补全）。`npm test` 还会运行 `tests/theme-pack-smoke.mjs`，在 Node 中真实执行主题包加载逻辑。

## Agent 检查清单

见根目录 `AGENTS.md`「主题系统」一节。一句话：**新主题 = 新文件 + registry 一行**，禁止长成上帝 CSS。
