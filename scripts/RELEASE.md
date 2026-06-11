# SUGT Portable Release Package

> 本目录由 `scripts/package-release.ps1` 自动生成，已加入 `.gitignore`，请勿提交到 Git。

## 目录结构

```
release/
└── SUGT-{version}-portable-windows-x64/
    ├── SUGT.exe          # GUI 主程序（Tauri）
    ├── sugt-cli.exe      # 命令行工具
    ├── SUGT.ico          # 图标
    ├── README.txt        # 使用说明
    └── VERSION.txt       # 版本信息
```

## 打包命令

一键构建 **GUI (SUGT.exe) + CLI (sugt-cli.exe)**：

```powershell
npm run package:release
```

或：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-release.ps1
```

构建步骤：前端 dist → `tauri build`（GUI）→ `cargo build sugt-cli`（CLI）。

连通性测试（魔搭等上游 + 本地网关代理）：

```powershell
$env:SUGT_TEST_API_KEY = "ms-..."
npm run test:connectivity
```

## 说明

- 产物为 **portable 绿色版**，无需安装 Rust / Node / 开发环境
- 目标系统需已安装 **WebView2 Runtime**（Windows 10/11 通常自带）
- 将整个文件夹复制到目标机器即可使用
