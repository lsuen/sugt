# 从源码构建（功能版）

适用公开树与私有完整树中的功能版（`SUGT_PRODUCT` 未设为 `store`）。

## 环境

- Windows 10 / 11 x64
- Node.js 18+
- Rust 1.78+（含 `cargo`）
- WebView2 Runtime

## 安装依赖

```cmd
npm install
```

## 开发运行

```cmd
npm run dev:app
```

## 检查

```cmd
npm run check
cd src-tauri
cargo test --lib
```

## 发布构建

```cmd
npm run build
npx tauri build --no-bundle
cd src-tauri
cargo build --release --bin sugt-cli
```

GUI 产物通常位于 `src-tauri/target/release/sugt.exe`。仅执行 `cargo build` 而不走 Tauri 前端嵌入流程时，界面资源可能缺失。

便携目录组装、试用注入与全功能版打包属于作者侧发版流程，不在公开树提供。
