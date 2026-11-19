# 局域网 WebUI 管理

> 一台机器开着小su控制台，同一局域网内任意设备用浏览器即可管理。纯局域网、本地优先，**不做互联网中转、不开账号系统、无云同步**。

## 它解决什么

CLI 用户（尤其中央网关 / systemd 常驻）以前只能靠敲命令改配置、翻日志、装技能。开启 WebUI 后，浏览器打开一个地址就能图形化管理这台网关——Provider、技能、接入信息、日志、策略规则等，覆盖与桌面 GUI 相同的管理面。

## 快速开始

### CLI（本次运行参数，不写配置）

```bash
# 启动网关 + WebUI，默认端口 8788
sugt-cli serve --lan --webui

# 指定 WebUI 端口（仅本次生效）
sugt-cli serve --lan --webui --webui-port 9000
```

启动后终端打印访问地址：

```
SUGT WebUI listening on http://0.0.0.0:8788
  LAN: http://192.168.1.10:8788
  本机: http://127.0.0.1:8788
  登录密码：gateway_client_api_key（见配置文件）
```

同网段设备浏览器打开 `http://192.168.1.10:8788`，输入 `gateway_client_api_key` 登录即可。

**关键语义**：`--webui` / `--webui-port` 与 `--port` 一样只影响本次运行，**不会写进配置文件**。退出进程后下次不带 `--webui` 启动，WebUI 不监听、不占内存和端口。

### GUI（配置持久化）

桌面版在设置页打开「局域网 WebUI」开关并设端口，配置写入 `config.toml`。此后每次启动 GUI 都会按配置自动监听 WebUI；关掉开关则完全不监听。

也可以用 CLI 子命令管理这份配置：

```bash
sugt-cli webui on --port 8788   # 开启并设端口（持久化，GUI 启动自动生效）
sugt-cli webui off              # 关闭（持久化）
sugt-cli webui status           # 查看当前配置与访问地址
```

> 两种语义不要混：**CLI 的 `serve --webui` 是临时开关**；**`webui on/off` 和 GUI 设置页是持久配置**。CLI 临时启动时端口未指定则取配置里的 `webui_port` 作为默认值，但不会改写它。

## 登录与鉴权

- 登录密码就是 `gateway_client_api_key`（局域网部署时由 `team setup` 自动生成的强密钥）。
- 也接受客户端 Key 池里所有「已启用」的 Key——与网关入站校验完全同一套语义（`gateway_auth::validate_client_request_with_keys`）。
- 登录成功后换发短期 HMAC 令牌（有效期 12 小时），会话存在浏览器 `sessionStorage`，关闭标签页即失效。
- 令牌密钥派生自 `gateway_client_api_key`，**改 Key 后所有已发会话立即失效**。
- 未登录访问 API 返回 401；伪造 / 篡改令牌一律拒绝（constant-time 签名比对）。

## 可管理功能（P0 + P1）

| 分组 | 能力 |
| --- | --- |
| 状态总览 | 网关运行状态、活跃模型、流量统计 |
| Provider | 增删查改、切换、启用禁用、连通性测试、模型发现 |
| 网关设置 | 端口、局域网、failover、客户端 Key |
| 技能 | 搜索、安装（含 GitHub/Gitee 地址）、卸载、挂载、仓库管理 |
| 客户端接入 | LAN 分享信息、接入片段、Key 池、入站密钥 |
| 日志 | 查看、维护设置、清缓存 |
| 规则 | 策略盒、请求规则、API Key 池 |
| 接管配置 | Profile 管理、Agent 发现、预览 |

桌面专属能力（窗口控制、桌面精灵、overlay、打开本地文件/编辑器）不通过 WebUI 暴露。

## HTTP 接口（供二次集成）

WebUI 服务同时托管前端静态资源与管理 API，同源部署。

| 方法 | 路径 | 鉴权 | 说明 |
| --- | --- | --- | --- |
| `GET` | `/api/health` | 否 | 健康检查 `{"ok":true,"service":"SUGT WebUI"}` |
| `POST` | `/api/login` | 否 | body `{"key":"..."}` → `{"token":"...","expires_in":43200}` |
| `POST` | `/api/invoke` | Bearer token | body `{"cmd":"<命令名>","args":{...}}` → `{"ok":true,"data":...}` 或 `{"ok":false,"error":"..."}` |
| 其它 | `/*` | 否 | 内嵌 SPA 静态资源 |

`/api/invoke` 的 `cmd` 就是桌面端 Tauri command 名，参数格式也一致——前端 transport 层据此做到同一份 UI 在桌面走 IPC、在浏览器走 HTTP，命令名零改动。错误以 HTTP 200 + body `ok:false` 返回，前端按 `error` 字段提示。

## 配置项

`config.toml`（字段均有 serde default，旧配置无需手动补）：

```toml
webui_enabled = false   # GUI 是否自动监听 WebUI（CLI 的 serve --webui 不看这个）
webui_port = 8788       # 默认端口
```

## 构建与发布

- 前端产物由 `npm run build:web` 生成到 `src-tauri/webui-dist/`，编译期通过 `include_dir!` 嵌入二进制——CLI 是独立二进制，运行时不依赖外部 dist 目录。
- CI 在编译 `sugt-cli` 的 Windows / Linux job 中先执行 `npm ci && npm run build:web`，正式包内嵌完整界面。
- 源码仓库只保留 `webui-dist/index.html` 占位页，保证未跑前端构建时 `cargo build` 也能通过。
- WebUI 产物约 1.2MB（gzip 后 JS ~157KB + CSS ~19KB），对安装包体积影响很小。

## 安全边界

- 只绑局域网，不 bind 公网、不做端口转发 / 穿透、不提供互联网中转。
- 信任内网场景使用 HTTP；跨不信任网段请挂反向代理 + TLS。
- 所有写操作走既有确定性命令，带后果预览与明确确认；WebUI 不开放任意路径文件读写。
- 关闭 WebUI 时（`webui_enabled=false` 且 CLI 不带 `--webui`）不创建 listener、不起后台任务，零内存零端口占用。
