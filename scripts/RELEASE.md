# SUGT 便携版打包说明

本目录产物由 `scripts/package-release.ps1` 生成，输出位于仓库根下 `release/`，该目录已加入 `.gitignore`，不要提交二进制。

## 目录结构

```text
release/
└── SUGT-{version}-{product?}-{variant}-windows-x64/
    ├── SUGT.exe
    ├── sugt-cli.exe
    ├── SUGT.ico
    ├── README.txt
    ├── VERSION.txt
    └── manifest.json
```

## 打包命令

```powershell
npm run package:release:self:zip
npm run package:release:trial:zip
npm run package:release:store:self:zip
npm run package:release:store:trial:zip
npm run package:release:wizard
```

构建顺序：前端 `dist` → `tauri build`（GUI）→ `cargo build`（`sugt-cli`）。

连通性测试：

```powershell
$env:SUGT_TEST_API_KEY = "..."
npm run test:connectivity
```

## 开源 Release 边界

- GitHub 开源 Release 只发布功能版（`SUGT_PRODUCT=feature`）资产。
- 全功能版与试用版仅作定向分发，不挂到开源 Release。
- 公开源码导出：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/export-public-tree.ps1
```

策略见 [docs/open-source.md](../docs/open-source.md)。

## 运行说明

- 产物为 portable 目录，无需在目标机安装 Rust / Node。
- 需要 WebView2 Runtime（Windows 10/11 通常已具备）。
- 将整个文件夹复制到目标机器即可使用。
