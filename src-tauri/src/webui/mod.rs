//! 局域网 WebUI 管理层。
//!
//! 架构约定（2026-11-20 订正）：
//! - 独立 axum listener，不并入 gateway router——后者 State 是 `GatewayState`，
//!   与管理面所需的 `AppRuntime` 不是同一类型，硬并会引入强绑定。
//! - 统一 RPC 入口 `POST /api/invoke`：前端命令名保持不变，transport 层只换通道。
//! - 业务实现只有一份：`commands.rs` 的 `*_core` 是桌面端与 WebUI 的共用入口，
//!   `service.rs` 只负责参数解析与结果序列化，不再复制命令体。
//! - 监听地址跟随「局域网访问」：未开启 `allow_lan_access` 时只绑回环，
//!   避免管理面在用户不知情时暴露到所有网卡（含公网 IP）。
//! - `webui_enabled = false` 时根本不构建/不监听，零内存零端口占用。

pub mod api;
pub mod assets;
pub mod auth;
pub mod service;

use crate::commands::AppRuntime;
use std::net::SocketAddr;
use tokio::sync::oneshot;

/// 运行中的 WebUI 服务句柄，drop 或调用 stop 即优雅关闭。
pub struct WebUiServer {
    pub addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
}

impl WebUiServer {
    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

/// 按配置决定是否启动 WebUI（GUI 版入口，配置持久化）。
/// 关闭或端口非法时返回 None，不产生任何监听。
pub async fn start_if_enabled(runtime: &AppRuntime) -> anyhow::Result<Option<WebUiServer>> {
    let (enabled, port) = {
        let cfg = runtime.config.read().await;
        (cfg.webui_enabled, cfg.webui_port)
    };
    if !enabled || port == 0 {
        return Ok(None);
    }
    start_on_port(runtime, port).await.map(Some)
}

/// 用显式端口启动 WebUI（CLI `serve --webui` 入口，只影响本次运行）。
/// 不读也不写 `webui_enabled`，CLI 参数语义与 `--port` 一致。
pub async fn start_on_port(runtime: &AppRuntime, port: u16) -> anyhow::Result<WebUiServer> {
    if port == 0 {
        return Err(anyhow::anyhow!("WebUI 端口不能为 0"));
    }
    start_listener(runtime, port).await
}

async fn start_listener(runtime: &AppRuntime, port: u16) -> anyhow::Result<WebUiServer> {
    // 管理面默认只对本机开放；要让同网段设备访问，必须先显式开启「局域网访问」
    // （与网关的 opt-in 语义一致），否则等于把改配置的入口暴露给所有网卡。
    let allow_lan = runtime.config.read().await.allow_lan_access;
    let bind_addr = if allow_lan {
        SocketAddr::from(([0, 0, 0, 0], port))
    } else {
        SocketAddr::from(([127, 0, 0, 1], port))
    };
    let listener = tokio::net::TcpListener::bind(bind_addr).await.map_err(|err| {
        anyhow::anyhow!(
            "WebUI 端口 {} 绑定失败：{}（可用 --webui-port 更换）",
            port,
            err
        )
    })?;
    let bound = listener.local_addr()?;
    let (tx, rx) = oneshot::channel();
    let app = api::build_router(runtime.clone());

    tokio::spawn(async move {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
        if let Err(err) = result {
            tracing::error!(error = %err, "WebUI server error");
        }
    });

    tracing::info!(%bound, "SUGT WebUI started");
    Ok(WebUiServer {
        addr: bound,
        shutdown: Some(tx),
    })
}
