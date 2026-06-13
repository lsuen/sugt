# SUGT 商店版 PRD（v0.2.0）

> 状态：实现中 · 产品线：`store` · 目标版本：**0.2.0**  
> 关联：[分支与版本管理](./BRANCHING.md)

## 1. 目标

在功能版网关底座上，为 Claude Code 新手提供 **技能发现 + 暂存安装 + 挂载 Claude** 能力；插件 **只读**，在线安装标「开发中」。

| 维度 | 功能版 `feature` | 商店版 `store` v0.2.0 |
| --- | --- | --- |
| 客户端 Tab | 接管为主 | Claude/Codex 大标签 + 技能商店 |
| Skill | — | 在线安装 → 暂存区 → 挂 Claude |
| 插件 | — | 只读列表 |
| Codex | 完整接管 | 占位，仅 env |

## 2. 信息架构

```
客户端 Tab（商店版）
├── Claude | Codex 大标签
├── 接管操作区（与功能版共用）
├── Claude：插件只读 + 暂存技能列表
├── 右上角：发现技能 | 管理仓库 | 暂存目录
└── 子页
    ├── 发现技能（搜索、仓库筛选、安装/挂载/卸载）
    └── 管理仓库（URL owner/repo/branch、刷新、编辑器设置）
```

## 3. 默认技能仓库

| owner | repo | branch |
| --- | --- | --- |
| ComposioHQ | awesome-claude-skills | master |
| JimLiu | baoyu-skills | main |
| anthropics | skills | main |
| cexll | myclaude | master |

## 4. 路径

| 用途 | 路径 |
| --- | --- |
| 商店根 | `Documents/.sugt/store/` |
| 暂存技能 | `store/skills/{skill_id}/` |
| Git 缓存 | `store/cache/{repo_id}/` |
| 挂 Claude | `%USERPROFILE%\.claude\skills\{folder}/` |
| 插件扫描 | `%USERPROFILE%\.claude\plugins/` |

## 5. v0.2.0 范围

- [x] 仓库 CRUD + git shallow clone
- [x] SKILL.md 扫描与目录索引
- [x] 安装到暂存 / 卸载 / 挂 Claude / 取消挂载
- [x] 插件只读 + 在线安装 disabled
- [x] 商店设置（编辑器命令）
- [ ] v0.3.0：插件在线安装（官方机制）

## 6. 技术模块

`src-tauri/src/store/`：`paths`, `repos`, `catalog`, `install`, `plugins`, `settings`, `commands`

前端：`src/store/StoreClientsPanel.tsx`
