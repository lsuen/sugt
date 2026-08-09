# SUGT 分支说明

私有完整树常用分支：

| 分支 | 用途 |
| --- | --- |
| `dev-feature` | 功能版日常开发 |
| `dev-store` | 全功能版（含技能中控台） |
| `master` | 发布就绪 |

构建变量：

| 变量 | 取值 | 含义 |
| --- | --- | --- |
| `SUGT_PRODUCT` | `feature` / `store` | 功能版 / 全功能版 |
| 打包变体 | `trial` / `self` | 试用 / 自用 |

公开仓默认 `main`，只接收导出后的开源子集。底座改动在 `dev-feature` 完成后再合入 `dev-store`。细节见私有树发版脚本与 [open-source.md](./open-source.md)。
