# SUGT

SUGT（su gateway）是面向内部研发场景的本地 OpenAI 兼容网关，用于让 Claude Code、OpenAI Codex 等 CLI 通过统一本地代理访问大模型服务。

| 项 | 说明 |
| --- | --- |
| 产品名 | SUGT |
| 全称 | su gateway |
| 作者 | 孙文龙 |
| 授权 | QM 科技内部定制，未经授权禁止传播 |

## 产品线

| 构建变量 `SUGT_PRODUCT` | 说明 |
| --- | --- |
| `feature`（默认） | 功能版：网关、模型配置、客户端接管 |
| `store` | 商店版：在功能版底座上增加 Claude/Codex 技能发现、暂存安装与挂载 |

两条产品线共用网关与接管逻辑；商店相关代码在 `dev-store` 分支演进，网关核心在 `dev-feature` 回流。

## 环境要求

- Windows 10/11 x64
- Node.js 18+
- Rust 1.78+（含 `cargo`、`tauri-cli`）
- Git for Windows（商店版刷新技能仓库时需要）

## 开发

```cmd
npm install
npm run dev:app              REM 功能版 GUI
npm run dev:app:store        REM 商店版 GUI（SUGT_PRODUCT=store）
npm run check                REM TypeScript 检查
```

Rust 单元测试：

```cmd
cd src-tauri
cargo test --lib
```

配置与日志默认目录：`Documents\.sugt\`（可通过环境变量 `SUGT_CONFIG_DIR` 覆盖）。

## 打包（Portable 绿色版）

产物输出到 `release/`（已 git 排除）。详见 `scripts/RELEASE.md` 与本地 `debug.md`（开发者备忘，不纳入版本库）。

常用命令：

```cmd
npm run package:release:self:zip           REM 功能版自用
npm run package:release:store:self:zip     REM 商店版自用
npm run package:release:all:zip            REM 功能版试用 + 自用
npm run package:release:wizard             REM 交互式选择
```

商店版打包示例产物：

```text
release/SUGT-0.2.1-store-self-windows-x64/
├── SUGT.exe
├── sugt-cli.exe
├── README.txt
├── VERSION.txt
└── manifest.json
```


详细说明见 `scripts/RELEASE.md`。

## 内部试用版有效期配置

有效期配置集中放在：

```text
scripts/release-settings.json
```

默认字段：

```json
{
  "defaultTrialDays": 30,
  "defaultTrialExpiresAt": null,
  "buildTrialByDefault": true,
  "buildSelfByDefault": true,
  "defaultTarget": "all",
  "createZipByDefault": true
}
```

修改默认试用天数：

- 文件：`scripts/release-settings.json`
- 字段：`defaultTrialDays`

指定固定截止日期：

- 文件：`scripts/release-settings.json`
- 字段：`defaultTrialExpiresAt`
- 示例：`"2026-07-10T23:59:59+08:00"`

临时指定截止日期：

```cmd
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-release.ps1 -Variant trial -TrialExpiresAt "2026-07-10T23:59:59+08:00" -Zip
```

有效期信息会在打包编译时写入 exe。程序运行后不会在 exe 所在目录生成运行时文件；机器绑定状态写入：

```text
Documents\.sugt\trial-state.json
```

关于页只显示一行低调说明：`内部试用版本，有效期至 yyyy-MM-dd` 或 `内部自用版本，无有效期限制`。

## 开发命令

国内 LLM 服务商 Base URL / 模型列表约定见 [docs/provider-vendors.md](docs/provider-vendors.md)。

```cmd
set "PATH=C:\Users\swl\.cargo\bin;%PATH%"
call "C:\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
npm install
npm run tauri:build
```

## CLI

```cmd
sugt-cli status
sugt-cli serve --host 127.0.0.1 --port 8787
sugt-cli env install --start-gateway
```

## 商店版（0.2.x）概要

- 客户端页：Claude / Codex 大标签；其下 **环境 / 技能 / 插件** 平级 Tab
- **发现技能**：全部 / 可安装 / 已暂存 / 已挂载；安装到 `Documents\.sugt\store\skills\`
- **GitHub 代理**：可选加速前缀（如 `https://ghfast.top`），刷新仓库时用于 git clone
- 挂载目录：Claude `~\.claude\skills`；Codex `~\.agents\skills`（与官方 SKILL.md 规范一致）
- 插件：只读列表 + 官方安装命令说明（不提供在线安装）
- 分支策略见 `docs/BRANCHING.md`；商店 PRD 见 `docs/PRD-store-v0.2.md`

## 构建注意

GUI 须通过 `tauri build` 或 `npm run package:release*` 构建。仅 `cargo build --release --bin sugt` 可能导致 WebView 资源未嵌入，界面出现「拒绝连接」。

窗口尺寸：`src-tauri/tauri.conf.json` 中的 `width` / `height`。

## 目录结构（简要）

```text
src/                 React 前端
src/store/           商店版 UI
src-tauri/src/       Rust 后端
src-tauri/src/store/ 商店模块（仓库、索引、安装、挂载）
scripts/             打包与通知脚本
docs/                产品/分支文档（部分开发文档在 docs/dev，不跟踪）
```

## 许可证与分发

内部使用。禁止将构建产物或源码向外部人员传播。
