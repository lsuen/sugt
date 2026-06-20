use anyhow::{Context, Result};
use std::{fs, path::Path, time::Duration};
use tokio::time::sleep;

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
        Ok(client) => client,
        Err(_) => return false,
    };
    match client.get(url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
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

pub fn locate_cli_exe() -> Result<std::path::PathBuf> {
    let current = std::env::current_exe().context("无法定位当前可执行文件")?;
    let name = current
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if name.eq_ignore_ascii_case("sugt.exe") {
        let cli = current.with_file_name("sugt-cli.exe");
        if cli.exists() {
            return Ok(cli);
        }
        return Err(anyhow::anyhow!(
            "未找到 sugt-cli.exe（应与 SUGT.exe 同目录）"
        ));
    }
    Ok(current)
}

pub fn spawn_detached(config_dir: &Path) -> Result<()> {
    if is_pid_file_alive(config_dir) {
        return Ok(());
    }
    let exe = locate_cli_exe()?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("serve");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    let child = cmd.spawn().context("后台启动 sugt-cli serve 失败")?;
    fs::write(
        config_dir.join(GATEWAY_PID_FILE),
        child.id().to_string(),
    )?;
    Ok(())
}

pub fn stop_detached(config_dir: &Path) -> Result<()> {
    let pid_path = config_dir.join(GATEWAY_PID_FILE);
    if let Ok(content) = fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<u32>() {
            kill_pid(pid);
        }
        let _ = fs::remove_file(&pid_path);
    }
    Ok(())
}

fn is_pid_file_alive(config_dir: &Path) -> bool {
    let pid_path = config_dir.join(GATEWAY_PID_FILE);
    let content = match fs::read_to_string(&pid_path) {
        Ok(content) => content,
        Err(_) => return false,
    };
    let pid = match content.trim().parse::<u32>() {
        Ok(pid) => pid,
        Err(_) => return false,
    };
    if is_pid_running(pid) {
        return true;
    }
    let _ = fs::remove_file(&pid_path);
    false
}

#[cfg(windows)]
fn is_pid_running(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|output| {
            let text = String::from_utf8_lossy(&output.stdout);
            text.contains(&pid.to_string())
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_pid_running(_pid: u32) -> bool {
    false
}

#[cfg(windows)]
fn kill_pid(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(not(windows))]
fn kill_pid(_pid: u32) {}
