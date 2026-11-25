//! 网关守护进程：后台启动 / 停止 `sugt-cli serve`，PID 文件管理，
//! 健康检查与进程查杀。

use anyhow::{Context, Result};
use std::{fs, path::Path, sync::OnceLock, time::Duration};
use tokio::time::sleep;

use crate::platform::{self, CommandPlatformExt};

const GATEWAY_PID_FILE: &str = "gateway.pid";

/// 探测连接超时。期望远小于「系统拒绝一个未监听端口」的耗时：安全软件/VPN 的
/// 过滤驱动会静默丢弃回环 SYN，让一次「连接被拒」等到 2s 才返回（实测本机
/// 关闭端口 2040ms、开放端口 2ms）。启动与状态轮询都靠这个探测判定网关状态，
/// 串行扫四个候选口就会把界面拖住 8s 以上。
const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
/// 探测整体超时：连接成功后等 /health 首字节。
const PROBE_TOTAL_TIMEOUT: Duration = Duration::from_millis(800);

/// 探测共用客户端。期望只建一次：每次探测新建 reqwest::Client 会重复初始化
/// 连接池与 TLS，本身也是启动期可感知的开销。
fn probe_client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(PROBE_CONNECT_TIMEOUT)
                .timeout(PROBE_TOTAL_TIMEOUT)
                .build()
                .ok()
        })
        .as_ref()
}

pub fn health_url(host: &str, port: u16) -> String {
    format!("http://{}:{}/health", host, port)
}

pub async fn is_reachable(host: &str, port: u16) -> bool {
    is_health_url_ok(&health_url(host, port)).await
}

pub async fn is_health_url_ok(url: &str) -> bool {
    let Some(client) = probe_client() else {
        return false;
    };
    client.get(url).send().await.map(|r| r.status().is_success()).unwrap_or(false)
}

/// 并行探测多个候选端口，按候选顺序返回第一个可达口。
/// 期望并行而不是串行：无监听者时每个口的拒绝/超时是独立成本，串行会把
/// 启动路径放大成「候选口数量 × 单次超时」。
pub async fn probe_first_reachable(host: &str, ports: &[u16]) -> Option<u16> {
    let tasks: Vec<_> = ports
        .iter()
        .copied()
        .map(|port| {
            let host = host.to_string();
            async move {
                if is_reachable(&host, port).await {
                    Some(port)
                } else {
                    None
                }
            }
        })
        .collect();
    // join_all 保持输入顺序，因此同时候选命中时仍然优先首选口。
    futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .flatten()
        .next()
}

pub async fn wait_reachable(host: &str, port: u16, attempts: u32) -> bool {
    for _ in 0..attempts {
        if is_reachable(host, port).await {
            return true;
        }
        sleep(Duration::from_millis(500)).await;
    }
    false
}

/// 定位 CLI 可执行文件。主程序同目录下找，找不到则返回当前 exe（即 CLI 本身）。
pub fn locate_cli_exe() -> Result<std::path::PathBuf> {
    let current = std::env::current_exe().context("无法定位当前可执行文件")?;
    let name = current.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let cli_name = platform::cli_exe_name("sugt-cli");

    if platform::is_app_exe(name, "sugt") {
        let cli = current.with_file_name(&cli_name);
        if cli.exists() {
            return Ok(cli);
        }
        return Err(anyhow::anyhow!(
            "未找到 {}（应与主程序同目录）",
            cli_name
        ));
    }
    Ok(current)
}

/// 后台启动 `sugt-cli serve`（如已运行则跳过）。
pub fn spawn_detached(config_dir: &Path) -> Result<()> {
    if is_pid_file_alive(config_dir) {
        return Ok(());
    }
    let exe = locate_cli_exe()?;
    let mut cmd = crate::process_util::hidden_command(&exe);
    cmd.arg("serve").detached();
    let child = cmd.spawn().context("后台启动 sugt-cli serve 失败")?;
    fs::write(config_dir.join(GATEWAY_PID_FILE), child.id().to_string())?;
    Ok(())
}

/// 停止后台守护进程。
pub fn stop_detached(config_dir: &Path) -> Result<()> {
    let pid_path = config_dir.join(GATEWAY_PID_FILE);
    if let Ok(content) = fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            platform::kill_pid(pid);
        }
        let _ = fs::remove_file(&pid_path);
    }
    Ok(())
}

/// 强制结束残留的独立网关进程（pid 文件丢失时的兜底）。
pub fn kill_sugt_cli_processes() {
    platform::kill_process_by_name("sugt-cli");
}

// ---------- 内部 ----------

/// 独立网关（`sugt-cli serve`）是否有活着的 PID 文件。
/// 期望状态判定先看进程再探端口：没有任何独立网关进程时，候选口必然全空，
/// 此时逐个探测只会白等超时。
pub fn detached_pid_alive(config_dir: &Path) -> bool {
    is_pid_file_alive(config_dir)
}

fn is_pid_file_alive(config_dir: &Path) -> bool {
    let pid_path = config_dir.join(GATEWAY_PID_FILE);
    let content = match fs::read_to_string(&pid_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let pid = match content.trim().parse::<u32>() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if platform::is_pid_running(pid) {
        return true;
    }
    let _ = fs::remove_file(&pid_path);
    false
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 期望候选口探测是并行的：本机对未监听端口的拒绝可能被安全软件拖到 2s，
    /// 串行扫四个候选口会把启动与状态轮询一起卡 8s（本仓曾实测启动固定 +10s）。
    /// 这里用四个必定关闭的端口作回归：并行应在毫秒级返回。
    #[tokio::test]
    async fn probe_first_reachable_runs_candidates_in_parallel() {
        let ports = [59_996u16, 59_997, 59_998, 59_999];
        let started = std::time::Instant::now();
        let found = probe_first_reachable("127.0.0.1", &ports).await;
        let elapsed = started.elapsed();
        assert!(found.is_none(), "测试端口不应有服务在监听: {found:?}");
        assert!(
            elapsed < Duration::from_secs(3),
            "候选口探测疑似串行，耗时 {elapsed:?}"
        );
    }
}
