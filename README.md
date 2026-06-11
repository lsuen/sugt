# SUGT - su gateway

SUGT 是面向内部研发场景的轻量级 OpenAI 兼容本地网关，用于让 Claude Code、Codex 等工具通过本地代理访问国内大模型服务。

- 软件名：SUGT
- 全称：su gateway
- 作者：孙文龙
- 定制：QM科技定制开发，仅限内部人员使用，未经授权禁止私自传播。

## 打包 Release（portable 绿色版）

无需开发环境即可运行的 exe 包，输出到 `release/` 目录（已 git 排除）。

给自己使用的无有效期版本：

```cmd
npm run package:release:self:zip
```

给同事试用的 30 天有效期版本：

```cmd
npm run package:release:trial:zip
```

一次打出试用版和自用版：

```cmd
npm run package:release:all:zip
```

交互式选择 GUI/CLI、是否试用、有效期和 zip：

```cmd
npm run package:release:wizard
```

构建后自动跑连通性测试（需先设置 `SUGT_TEST_API_KEY`）：

```cmd
set SUGT_TEST_API_KEY=ms-...
npm run package:release:test
```

仅测连通性（上游 + SUGT 网关）：

```cmd
set SUGT_TEST_API_KEY=ms-...
npm run test:connectivity
```

产物目录示例：

```
release/SUGT-0.1.0-trial-windows-x64/
├── SUGT.exe       # GUI
├── sugt-cli.exe   # CLI
├── SUGT.ico
├── README.txt
├── VERSION.txt
└── manifest.json

release/SUGT-0.1.0-self-windows-x64/
├── SUGT.exe
├── sugt-cli.exe
├── SUGT.ico
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
```

> **注意**：GUI 必须用 `tauri build` 或 `npm run package:release` 构建。仅用 `cargo build --release --bin sugt` 会导致界面显示「拒绝连接」。

窗口大小：`src-tauri/tauri.conf.json` → `width` / `height`