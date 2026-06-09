# SUGT - su gateway

SUGT 是面向内部研发场景的轻量级 OpenAI 兼容本地网关，用于让 Claude Code、Codex 等工具通过本地代理访问国内大模型服务。

- 软件名：SUGT
- 全称：su gateway
- 作者：孙文龙
- 定制：QM科技定制开发，仅限内部人员使用，未经授权禁止私自传播。

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
