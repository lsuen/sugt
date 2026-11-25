//! 单实例保护。
//!
//! 期望重复启动时聚焦已有主窗口而不是再开一个完整实例，因为第二个实例会同时
//! 抢网关端口、重复拉起托盘与精灵窗口，并与第一个实例共享同一个 WebView2
//! 用户数据目录（`%LOCALAPPDATA%\com.qmtech.sugt\EBWebView`），实测会表现为
//! 「点了没反应 / 启动很慢 / 两个托盘图标」。判据用回环控制口而不是文件锁：
//! 端口 bind 是即时的，不受「未监听端口拒绝耗时」影响。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use tauri::{AppHandle, Manager};

const PING: &[u8] = b"SUGT-SINGLE-INSTANCE-PING";
const PONG: &[u8] = b"SUGT-SINGLE-INSTANCE-PONG";

/// 控制口只绑回环、不对局域网开放。
fn control_port() -> u16 {
    // 开发构建与安装包分开，避免 `npm run tauri dev` 被已安装的正式版顶掉。
    if cfg!(debug_assertions) {
        39675
    } else {
        39674
    }
}

pub enum Acquire {
    /// 本进程是第一个实例，持有控制口监听负责应答后来者。
    Primary(TcpListener),
    /// 已有实例在运行，且已请求它显示主窗口；本进程应直接退出。
    AlreadyRunning,
    /// 控制口被别的程序占用且无 SUGT 应答：无法判断，放弃保护但不阻断启动。
    Unknown,
}

pub fn acquire() -> Acquire {
    match TcpListener::bind(("127.0.0.1", control_port())) {
        Ok(listener) => Acquire::Primary(listener),
        Err(_) => {
            if notify_existing() {
                Acquire::AlreadyRunning
            } else {
                Acquire::Unknown
            }
        }
    }
}

/// 请求已有实例显示主窗口；只有收到 SUGT 自己的应答才算命中，避免把占用
/// 同一端口的其他程序误判成 SUGT 实例。
fn notify_existing() -> bool {
    let addr: SocketAddr = match format!("127.0.0.1:{}", control_port()).parse() {
        Ok(addr) => addr,
        Err(_) => return false,
    };
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_millis(400)) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
    if stream.write_all(PING).is_err() {
        return false;
    }
    let mut buf = [0u8; 32];
    match stream.read(&mut buf) {
        Ok(n) => &buf[..n] == PONG,
        Err(_) => false,
    }
}

/// 在独立线程里应答后来者：收到 PING 就回 PONG，并把主窗口叫到前台。
pub fn serve(listener: TcpListener, app: AppHandle) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
            let mut buf = [0u8; 32];
            let matched = matches!(stream.read(&mut buf), Ok(n) if &buf[..n] == PING);
            if !matched {
                continue;
            }
            let _ = stream.write_all(PONG);
            let _ = stream.flush();
            let handle = app.clone();
            // 窗口操作必须回到主线程，直接在工作线程调用会被 Tauri 拒绝。
            let _ = app.run_on_main_thread(move || {
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 期望后来者只在收到 SUGT 自己的 PONG 时才判定「已有实例」，
    /// 避免把占用同一端口的其他程序误认为 SUGT 而拒绝启动。
    #[test]
    fn notify_existing_detects_a_sugt_peer() {
        let Ok(listener) = TcpListener::bind(("127.0.0.1", control_port())) else {
            // 端口已被真实实例占用：本测试无法自建对端，跳过。
            return;
        };
        let peer = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 32];
                let n = stream.read(&mut buf).unwrap_or(0);
                if &buf[..n] == PING {
                    let _ = stream.write_all(PONG);
                }
            }
        });
        assert!(notify_existing());
        let _ = peer.join();
    }
}
