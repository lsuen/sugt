//! 实验·免费通道闲时测速：后台循环，默认关闭。
//! 复用 `test_free_models`；开启 auto_rotate 时按 t/s 切当前路由。

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::{
    commands::AppRuntime,
    experimental_free::{self, is_experimental_free},
    model::MIN_EXPERIMENTAL_FREE_IDLE_PROBE_INTERVAL_SECS,
};

/// 启动闲时探测循环（与看门狗并列，不阻塞启动）。
pub fn spawn(runtime: Arc<AppRuntime>) {
    tokio::spawn(async move {
        loop {
            let (enabled, interval_secs, auto_rotate, has_free) = {
                let config = runtime.config.read().await;
                let has_free = config.providers.iter().any(|p| {
                    is_experimental_free(p) && p.enabled && config.experimental_free_disclaimer_accepted
                });
                (
                    config.experimental_free_idle_probe_enabled,
                    config
                        .experimental_free_idle_probe_interval_secs
                        .max(MIN_EXPERIMENTAL_FREE_IDLE_PROBE_INTERVAL_SECS),
                    config.experimental_free_auto_rotate,
                    has_free,
                )
            };

            tokio::time::sleep(Duration::from_secs(interval_secs)).await;

            if !enabled || !has_free {
                continue;
            }

            if let Err(err) = run_idle_cycle(&runtime, auto_rotate).await {
                warn!(error = %err, "experimental free idle probe failed");
            }
        }
    });
}

async fn run_idle_cycle(runtime: &AppRuntime, auto_rotate: bool) -> Result<(), String> {
    info!("experimental free idle probe: starting");
    let probes =
        experimental_free::test_free_models(runtime.gateway.http_client()).await;
    if !auto_rotate {
        return Ok(());
    }

    let mut config = runtime.config.write().await;
    let Some(provider) = config
        .providers
        .iter_mut()
        .find(|p| is_experimental_free(p) && p.enabled)
    else {
        return Ok(());
    };
    let current_route_id = provider.model_name.clone();
    // 先取候选指标用于日志：期望轮换日志能判定「为什么换、换到哪、快多少」，
    // 因为只有 'rotated' 这种无字段日志无法在事后定位误切问题。
    let candidate = experimental_free::pick_best_probe(&probes).map(|probe| {
        (
            probe.name.clone(),
            probe.tokens_per_sec,
            probe.latency_ms,
        )
    });
    if !experimental_free::apply_best_from_probes(provider, &probes) {
        info!(
            action = "idle_auto_rotate",
            reason = "no_better_route",
            current_route_id,
            candidate_route_id = %candidate.as_ref().map(|c| c.0.as_str()).unwrap_or_default(),
            "experimental free idle probe: kept current route"
        );
        return Ok(());
    }
    let snapshot = config.clone();
    drop(config);
    runtime.persist().await.map_err(|e| e.to_string())?;
    runtime.gateway.replace_config(snapshot).await;
    info!(
        action = "idle_auto_rotate",
        reason = "tps_latency_probe",
        current_route_id,
        candidate_route_id = %candidate.as_ref().map(|c| c.0.as_str()).unwrap_or_default(),
        tokens_per_sec = ?candidate.as_ref().and_then(|c| c.1),
        latency_ms = ?candidate.as_ref().and_then(|c| c.2),
        "experimental free idle probe: rotated to best route"
    );
    Ok(())
}
