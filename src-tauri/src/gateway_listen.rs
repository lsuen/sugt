use anyhow::{anyhow, Result};
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::{error_hint, model::AppConfig};

/// 按配置顺序尝试绑定端口；全部失败时返回最后一次错误。
pub async fn bind_with_fallback(config: &AppConfig) -> Result<(TcpListener, std::net::SocketAddr)> {
    let host = config.bind_host();
    let ports = config.port_candidates();
    // 期望候选表可观测，因为端口漂移是常见故障，日志里要能看出「到底扫了哪些口」。
    info!(host = %host, requested_port = config.port, candidates = ?ports, "gateway bind started");
    let mut last_err: Option<anyhow::Error> = None;

    for (idx, port) in ports.into_iter().enumerate() {
        let addr: std::net::SocketAddr = format!("{}:{}", host, port)
            .parse()
            .map_err(|err| anyhow!("监听地址无效 {}:{} — {}", host, port, err))?;
        match TcpListener::bind(addr).await {
            Ok(listener) => {
                let bound = listener.local_addr()?;
                info!(host = %host, port = bound.port(), requested_port = config.port, candidate_index = idx, "gateway listener bound");
                if bound.port() != config.port {
                    warn!(
                        host = %host,
                        requested = config.port,
                        bound = bound.port(),
                        "port fallback: bound alternate port"
                    );
                }
                return Ok((listener, bound));
            }
            Err(err) => {
                let hint = error_hint::format_bind_error(addr, &err);
                warn!(host = %host, port, candidate_index = idx, error = %hint, "bind failed, trying next candidate");
                last_err = Some(anyhow!(hint));
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("没有可用的监听端口，请检查 port_fallback_ports 配置")))
}

#[cfg(test)]
mod tests {
    use crate::model::AppConfig;

    #[test]
    fn port_candidates_include_primary_and_fallbacks() {
        let mut config = AppConfig::default();
        config.port = 8787;
        config.port_fallback_ports = vec![9878, 18887];
        let ports = config.port_candidates();
        assert_eq!(ports, vec![8787, 9878, 18887]);
    }

    #[test]
    fn port_candidates_skip_duplicate_fallbacks() {
        let mut config = AppConfig::default();
        config.port = 8787;
        config.port_fallback_ports = vec![8787, 9878];
        let ports = config.port_candidates();
        assert_eq!(ports, vec![8787, 9878]);
    }
}
