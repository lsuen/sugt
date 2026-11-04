//! 局域网可达地址探测（供「允许局域网」分享连接信息）。

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// 本机可用于局域网访问的 IPv4（排除环回 / 链路本地）。
pub fn lan_ipv4_addrs() -> Vec<String> {
    let mut set = BTreeSet::new();

    // 默认路由出口地址：多数单网卡场景即用户要分享的 IP
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        // 不真正发包，只用来解析本地出口
        if sock.connect("1.1.1.1:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                if let IpAddr::V4(ip) = addr.ip() {
                    if is_shareable_v4(ip) {
                        set.insert(ip.to_string());
                    }
                }
            }
        }
    }

    set.into_iter().collect()
}

fn is_shareable_v4(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_unspecified() && !ip.is_link_local() && !ip.is_multicast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_loopback_and_link_local() {
        assert!(!is_shareable_v4(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!is_shareable_v4(Ipv4Addr::new(169, 254, 1, 1)));
        assert!(!is_shareable_v4(Ipv4Addr::UNSPECIFIED));
        assert!(is_shareable_v4(Ipv4Addr::new(192, 168, 1, 10)));
        assert!(is_shareable_v4(Ipv4Addr::new(10, 0, 0, 2)));
    }

    #[test]
    fn discover_does_not_panic() {
        let _ = lan_ipv4_addrs();
    }
}
