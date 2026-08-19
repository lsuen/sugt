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
    #[cfg(windows)]
    let cli_name = "sugt-cli.exe";
    #[cfg(not(windows))]
    let cli_name = "sugt-cli";
    let name = current
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    #[cfg(windows)]
    let is_app = name.eq_ignore_ascii_case("sugt.exe");
    #[cfg(not(windows))]
    let is_app = name == "sugt";
    if is_app {
        let cli = current.with_file_name(cli_name);
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

pub fn spawn_detached(config_dir: &Path) -> Result<()> {
    if is_pid_file_alive(config_dir) {
        return Ok(());
    }
    let exe = locate_cli_exe()?;
    let mut cmd = crate::process_util::hidden_command(&exe);
    cmd.arg("serve");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        // 覆盖 hidden_command 的 flags，额外分离进程
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组，避免随父进程退出被 SIGHUP 终止
        cmd.process_group(0);
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

/// 强制结束残留的独立网关进程（pid 文件丢失时的兜底）。
pub fn kill_sugt_cli_processes() {
    #[cfg(windows)]
    {
        use std::process::Stdio;
        let _ = crate::process_util::hidden_command("taskkill")
            .args(["/IM", "sugt-cli.exe", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        use std::process::Stdio;
        let _ = std::process::Command::new("pkill")
            .args(["-f", "sugt-cli"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
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
    use std::process::Stdio;
    crate::process_util::hidden_command("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|output| {
            let text = String::from_utf8_lossy(&output.stdout);
            text.contains(&pid.to_string())
        })
        .unwrap_or(false)
}

#[cfg(unix)]
fn is_pid_running(pid: u32) -> bool {
    // kill -0 不发送信号，仅探测进程是否存在
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(any(windows, unix)))]
fn is_pid_running(_pid: u32) -> bool {
    false
}

#[cfg(windows)]
fn kill_pid(pid: u32) {
    use std::process::Stdio;
    let _ = crate::process_util::hidden_command("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn kill_pid(pid: u32) {
    use std::process::Stdio;
    let _ = std::process::Command::new("kill")
        .args([&pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(windows, unix)))]
fn kill_pid(_pid: u32) {}
