use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::{commands::AppRuntime, gateway_daemon};

/// 后台循环：网关不可达时自动拉起（嵌入式 +  detached 双路径）。
pub fn spawn(runtime: Arc<AppRuntime>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let config = runtime.config.read().await.clone();
            if !config.gateway_watchdog_enabled {
                continue;
            }
            let client_host = config.client_host();
            if gateway_daemon::is_reachable(&client_host, config.port).await {
                continue;
            }

            warn!("gateway watchdog: unreachable, attempting recovery");
            if let Err(err) = recover_gateway(&runtime).await {
                warn!(error = %err, "gateway watchdog recovery failed");
            } else {
                info!("gateway watchdog: recovery succeeded");
            }
        }
    });
}

async fn recover_gateway(runtime: &AppRuntime) -> Result<(), String> {
    let preferred = runtime.config.read().await.clone();
    let client_host = preferred.client_host();

    if !runtime.gateway.is_running().await {
        if let Err(err) = runtime.gateway.start().await {
            warn!(error = %err, "watchdog embedded start failed");
        }
    }

    let after_embed = crate::commands::config_with_listen_port(runtime).await;
    if gateway_daemon::wait_reachable(&client_host, after_embed.port, 6).await {
        let _ = crate::commands::sync_client_endpoints_to_listen_port(runtime).await;
        return Ok(());
    }

    gateway_daemon::spawn_detached(&runtime.paths.config_dir)
        .map_err(|e| e.to_string())?;

    // 扫候选口；成功只刷新接管脚本，不把临时口写进首选 port。
    let mut found = false;
    for port in preferred.port_candidates() {
        if gateway_daemon::wait_reachable(&client_host, port, 4).await {
            found = true;
            break;
        }
    }
    if !found {
        return Err("watchdog: health check still failing after recovery".to_string());
    }
    crate::commands::sync_client_endpoints_to_listen_port(runtime).await?;
    Ok(())
}
