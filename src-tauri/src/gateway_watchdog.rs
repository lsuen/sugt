use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::{clients, commands::AppRuntime, gateway_daemon};

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
    let config = runtime.config.read().await.clone();
    let client_host = config.client_host();

    if !runtime.gateway.is_running().await {
        if let Err(err) = runtime.gateway.start().await {
            warn!(error = %err, "watchdog embedded start failed");
        }
    }

    if gateway_daemon::wait_reachable(&client_host, config.port, 6).await {
        return Ok(());
    }

    gateway_daemon::spawn_detached(&runtime.paths.config_dir)
        .map_err(|e| e.to_string())?;

    if !gateway_daemon::wait_reachable(&client_host, config.port, 20).await {
        return Err("watchdog: health check still failing after recovery".to_string());
    }

  let gw_config = runtime.gateway.config().await;
    if gw_config.port != config.port {
        let mut cfg = runtime.config.write().await;
        cfg.port = gw_config.port;
        let updated = cfg.clone();
        runtime.persist().await.map_err(|e| e.to_string())?;
        let _ = clients::write_launch_scripts(&runtime.paths, &updated);
        let _ = clients::repair(&runtime.paths.config_dir, &updated);
    }

    Ok(())
}
