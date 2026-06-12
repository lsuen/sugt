# SUGT 分支与版本管理

## 1. 产品线与授权（两个维度）

构建时通过环境变量区分，**互相独立**：

| 变量 | 取值 | 含义 |
| --- | --- | --- |
| `SUGT_PRODUCT` | `feature`（默认） / `store` | 功能版 vs 商店版 |
| `SUGT_EDITION` + trial 元数据 | `trial` / `self` / dev | 试用 vs 自用 vs 本地开发 |

打包产物命名：

| 产物 | 目录/包名示例 |
| --- | --- |
| 功能版自用 | `SUGT-0.1.2-self-windows-x64` |
| 功能版试用 | `SUGT-0.1.2-trial-windows-x64` |
| 商店版自用 | `SUGT-0.2.0-store-self-windows-x64` |
| 商店版试用 | `SUGT-0.2.0-store-trial-windows-x64` |

## 2. 分支模型

```
master          ← 仅合并 dev-feature 的发布就绪提交；打功能版 tag
dev-feature     ← 功能版日常开发（网关、接管、模型…）
dev-store       ← 商店版开发；定期 merge/rebase dev-feature 同步底座
```

### 规则

1. **功能修复 / 网关 / 接管**：只在 `dev-feature` 开发 → 合并 `master` 发版。
2. **商店 UI / Claude 清单 / skill 编辑器**：只在 `dev-store` 开发。
3. **底座同步**：每 1～2 个 feature 里程碑，将 `dev-feature` merge 进 `dev-store`。
4. **禁止**：仅在 `dev-store` 修改 `gateway` / `clients` 核心逻辑（应回流 feature）。
5. **商店发版**：从 `dev-store` 打 tag，如 `v0.2.0-store.1`；功能版 tag 如 `v0.1.3`。

## 3. 版本号策略

| 线 | 版本 | 说明 |
| --- | --- | --- |
| 功能版 | 0.1.x → 0.2.0 | patch 修优化；minor 加能力 |
| 商店版 | 0.2.0 起 | 首版查看；与 feature 可同号不同产物 |

关于页 / `VERSION.txt` 同时展示 **semver + product_line + edition**。

## 4. 本地开发

```powershell
# 功能版 GUI 开发（默认）
npm run dev:app

# 商店版 GUI 开发（编译期 SUGT_PRODUCT=store）
npm run dev:app:store
```

## 5. 打包命令

```powershell
# 功能版（与现有一致）
npm run package:release:self
npm run package:release:trial

# 商店版
npm run package:release:store:self
npm run package:release:store:trial
```

## 6. 合并检查清单（feature → store）

- [ ] `cargo test` / `npm run check` 通过
- [ ] 功能版 smoke：网关启停、接管
- [ ] 商店版 smoke：关于页显示商店版；Claude 面板无 crash
- [ ] 无 store 专用改动污染 feature 独有路径（或已拆模块）

## 7. 当前分支初始化

首次建立（仅一次）：

```powershell
git checkout master
git branch dev-feature
git branch dev-store
# 日常：git checkout dev-feature  或  dev-store
```

---

*与 [PRD-store-v0.2.md](./PRD-store-v0.2.md) 配套使用。*
