# SUGT 分支与版本管理

## 1. 产品线与构建变体

构建时通过环境变量区分，互相独立：

| 变量 | 取值 | 含义 |
| --- | --- | --- |
| `SUGT_PRODUCT` | `feature`（默认） / `store` | 功能版 vs 全功能版（含技能中控台） |
| `SUGT_EDITION` + trial 元数据 | `trial` / `self` / dev | 试用 vs 自用 vs 本地开发 |

打包产物命名示例：

| 产物 | 目录/包名示例 |
| --- | --- |
| 功能版自用 | `SUGT-1.0.0-self-windows-x64` |
| 功能版试用 | `SUGT-1.0.0-trial-windows-x64` |
| 全功能自用 | `SUGT-1.0.0-store-self-windows-x64` |
| 全功能试用 | `SUGT-1.0.0-store-trial-windows-x64` |

开源同步只覆盖功能版可公开子集，见 [open-source.md](./open-source.md)。同步白名单留在私有完整树的 `public-manifest.md`，不进入公开仓。

## 2. 分支模型（私有完整树）

```text
master          发布就绪；功能版可从此打 tag
dev-feature     功能版日常开发（网关、接管、模型）
dev-store       全功能版开发；定期合并 dev-feature 同步底座
```

规则：

1. 功能修复 / 网关 / 接管：在 `dev-feature` 开发，合并入 `master` 后再发功能版。
2. 技能中控台 UI 与商店后端：在 `dev-store` 开发。
3. 底座同步：按里程碑将 `dev-feature` 合并进 `dev-store`。
4. 禁止仅在 `dev-store` 修改网关 / 接管核心逻辑而不回流 `dev-feature`。
5. 全功能发版从 `dev-store` 打私有 tag；功能版 tag 从 `master` 或已同步的公开提交打出。

## 3. 公开仓分支

公开仓默认使用 `main`，只接收导出脚本生成的公开树。不在公开仓保留 `dev-store`，避免商店实现误推送。

## 4. 本地开发

```powershell
npm run dev:app
npm run dev:app:store
```

## 5. 打包命令

```powershell
npm run package:release:self
npm run package:release:trial
npm run package:release:store:self
npm run package:release:store:trial
```

## 6. 合并检查清单（feature 到 store）

- `cargo test` / `npm run check` 通过
- 功能版冒烟：网关启停、接管
- 全功能冒烟：关于页产品形态正确；技能页与仓库刷新无崩溃
- 无商店专用改动污染功能版独有路径（或已用 feature 门控隔离）
