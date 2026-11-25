use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::{commands::AppRuntime, gateway_daemon};

/// 看门狗轮询间隔。期望 20 秒一轮，因为过短会频繁探测网关、过长会让故障恢复太慢。
const WATCHDOG_POLL_INTERVAL_SECS: u64 = 20;
/// 嵌入式网关启动后等待可达的秒数。期望 6 秒，因为本地进程启动通常秒级完成。
const EMBEDDED_START_WAIT_SECS: u32 = 6;
/// 候选口短探的秒数。期望 4 秒，因为备选口要尽快失败并换下一个。
const CANDIDATE_PROBE_WAIT_SECS: u32 = 4;

/// 后台循环：网关不可达时自动拉起（嵌入式 +  detached 双路径）。
pub fn spawn(runtime: Arc<AppRuntime>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(WATCHDOG_POLL_INTERVAL_SECS)).await;
            // 退出流程已开始就不要再拉起网关：否则 stop() 刚执行完，
            // watchdog 20s 后把网关复活，用户看到「点了退出却又活过来、进程还在」。
            if crate::commands::QUITTING.load(std::sync::atomic::Ordering::SeqCst) {
                info!("gateway watchdog: quitting, stop watching");
                break;
            }
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
    if gateway_daemon::wait_reachable(&client_host, after_embed.port, EMBEDDED_START_WAIT_SECS).await {
        let _ = crate::commands::sync_client_endpoints_to_listen_port(runtime).await;
        return Ok(());
    }

    gateway_daemon::spawn_detached(&runtime.paths.config_dir)
        .map_err(|e| e.to_string())?;

    // 扫候选口；成功只刷新接管脚本，不把临时口写进首选 port。
    let mut found = false;
    for port in preferred.port_candidates() {
        if gateway_daemon::wait_reachable(&client_host, port, CANDIDATE_PROBE_WAIT_SECS).await {
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
