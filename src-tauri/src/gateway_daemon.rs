//! 网关守护进程：后台启动 / 停止 `sugt-cli serve`，PID 文件管理，
//! 健康检查与进程查杀。

use anyhow::{Context, Result};
use std::{fs, path::Path, time::Duration};
use tokio::time::sleep;

use crate::platform::{self, CommandPlatformExt};

const GATEWAY_PID_FILE: &str = "gateway.pid";

pub fn health_url(host: &str, port: u16) -> String {
    format!("http://{}:{}/health", host, port)
}

pub async fn is_reachable(host: &str, port: u16) -> bool {
    is_health_url_ok(&health_url(host, port)).await
}

pub async fn is_health_url_ok(url: &str) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client.get(url).send().await.map(|r| r.status().is_success()).unwrap_or(false)
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