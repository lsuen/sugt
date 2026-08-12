---
name: sugt-cli
description: 接入 SUGT 本地 AI 网关的技能。能搜索、安装、管理 SUGT 技能仓中的技能。当用户希望发现/安装/管理技能，或希望使用 SUGT CLI 工具时使用。
---

# SUGT CLI 技能

你可以调用 `sugt-cli` 命令来使用 SUGT 的全部能力。SUGT 是一款本地 AI 网关工具，把 LLM API Key、Agent 接管、Skills 挂载收拢到一个本机小工具里。

## 何时使用本技能

- 用户希望**搜索**可用的技能
- 用户希望**安装**某个技能
- 用户希望**列出**已安装的技能
- 用户希望**获取**基于当前项目上下文的技能建议
- 用户希望**卸载**某个技能
- 任何涉及 SUGT 技能管理的场景

## 命令清单

> 提示：先 `sugt-cli status` 检查网关是否可用；如果显示"网关未运行"，告诉用户需要先在 SUGT 应用中启动网关。

### 搜索技能

```bash
sugt-cli skill search <关键词>
```

返回匹配的技能列表（含 ID、名称、描述、来源仓库）。

### 列出已安装的技能

```bash
sugt-cli skill list
```

显示本地已安装的技能、挂载状态。

### 查看技能详情

```bash
sugt-cli skill info <技能ID>
```

显示技能的完整元信息：名称、描述、依赖、SKILL.md 内容预览。

### 安装技能

```bash
sugt-cli skill install <技能ID>
```

下载技能到本地暂存区。

### 挂载技能

```bash
sugt-cli skill mount <技能ID> --target claude
sugt-cli skill mount <技能ID> --target codex
sugt-cli skill mount <技能ID> --target both  # 默认
```

把技能挂载到 Claude Code / Codex 等 Agent。

### 卸载技能

```bash
sugt-cli skill uninstall <技能ID>
```

### 获取技能建议

```bash
sugt-cli skill suggest
```

根据当前项目目录分析（语言、框架、常见任务），推荐适合的技能。

## 工作流程建议

当用户进入新项目时：

1. 先执行 `sugt-cli skill suggest`，看是否有匹配的技能可以提升工作效率
2. 询问用户是否要安装推荐的技能
3. 安装后执行 `sugt-cli skill list --mounted` 确认挂载状态

当用户提出具体需求时：

1. 执行 `sugt-cli skill search <关键词>` 查找相关技能
2. 展示找到的技能给用户
3. 用户确认后执行安装 + 挂载

## 注意事项

- `sugt-cli` 在 PATH 中必须可用（`which sugt-cli` 验证）。如果不可用，告诉用户重新启动一个新终端让环境变量生效，或者在 SUGT 客户端界面重新点一次"接管"
- 所有命令在 Windows 上都已支持，命令格式跨平台一致
- 网关未运行时，部分命令会返回错误；可让用户先用 `sugt-cli status` 检查
