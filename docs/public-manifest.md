# 公开树清单（仅私有完整树维护，不进入公开仓）

本清单供 `scripts/export-public-tree.ps1` 使用。路径相对仓库根。未列出的路径默认不导出。

## 1. 导出（允许）

### 根文件

- `LICENSE`
- `NOTICE`
- `README.md`
- `package.json`
- `package-lock.json`
- `index.html`
- `tsconfig.json`
- `tsconfig.node.json`
- `vite.config.ts`
- `.gitignore`

### 文档

- `docs/open-source.md`
- `docs/BRANCHING.md`
- `docs/provider-vendors.md`
- `docs/build.md`
- `docs/images/`（README 截图与说明）

### 前端（功能版）

- `src/` 下除技能中控台以外的源文件（见禁止项）

### 后端

- `src-tauri/Cargo.toml`
- `src-tauri/Cargo.lock`
- `src-tauri/tauri.conf.json`
- `src-tauri/build.rs`（若存在）
- `src-tauri/capabilities/`
- `src-tauri/permissions/`
- `src-tauri/icons/`（若存在）
- `src-tauri/src/**` 中除禁止项以外的源文件

### CI

- `.github/workflows/ci.yml`
- `.github/workflows/release-feature.yml`

## 2. 不导出（禁止）

### 技能中控台

- `src/store/`
- `src/SkillsPanel.tsx`
- `src-tauri/src/store/`

### 内部文档与导出元数据

- `docs/PRD-*.md`
- `docs/功能点文档-v0.2.5.md`
- `docs/技术实现文档-v0.2.5.md`
- `docs/public-manifest.md`
- `运行调试.md`
- `debug.md`
- `PUBLIC_EXPORT.txt`

### 发版与内部工具（留在私有完整树）

- `scripts/export-public-tree.ps1`
- `scripts/package-release.ps1`
- `scripts/package-release-all.ps1`
- `scripts/package-release-wizard.ps1`
- `scripts/release-settings.json`
- `scripts/RELEASE.md`
- `scripts/test-connectivity.ps1`
- `scripts/dingtalk-*.json`
- `scripts/.notify-*.json`

### 密钥与本地状态

- `.secrets/`
- `.env` / `.env.*`
- 名为 `.github` 的文件（非目录）

### 构建与缓存

- `node_modules/`、`dist/`、`dist-public/`、`release/`、`src-tauri/target/`、`.vite/`、`.MemoryForAI/`、`baks/` 等

## 3. Release 资产

| 资产 | 公开 GitHub 源码 | 公开 GitHub Release |
| --- | --- | --- |
| 功能版源码子集 | 是 | — |
| 功能版 zip | — | 可选 |
| 全功能试用 zip | 否 | 允许（推广试用） |
| 导出脚本与试用注入配置 | 否 | — |

私有完整树（Codeup / 本机）为权威源；公开仓由维护者按需导出合并，不自动双向同步。
