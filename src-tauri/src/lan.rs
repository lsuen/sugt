//! 局域网可达地址探测（供「允许局域网」分享连接信息）。

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// 本机可用于局域网访问的 IPv4（排除环回 / 链路本地）。
///
/// 枚举所有网卡，而不是只看默认路由出口：开了 VPN 时默认出口是 utun 隧道
/// （实测 utun7 = `28.0.0.1`），只探默认路由会把隧道 IP 当成「局域网地址」展示，
/// 而真正的 `192.168.x.x` 永远出不来。RFC1918 私网地址优先；存在私网地址时
/// 隧道 / 公网地址不再列入，因为它们对同一局域网的其它设备不可达。
pub fn lan_ipv4_addrs() -> Vec<String> {
    let mut found: Vec<Ipv4Addr> = Vec::new();
    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for interface in interfaces {
            if let if_addrs::IfAddr::V4(v4) = interface.addr {
                let ip = v4.ip;
                if is_shareable_v4(ip) && !found.contains(&ip) {
                    found.push(ip);
                }
            }
        }
    }
    // 兜底：个别平台接口枚举为空时，用默认路由出口探一次（可能拿到隧道 IP）。
    if found.is_empty() {
        if let Some(ip) = default_route_ipv4() {
            found.push(ip);
        }
    }
    order_lan_addrs(found)
}

/// 默认路由出口地址：不真正发包，只用来解析本地出口。
fn default_route_ipv4() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    if sock.connect("1.1.1.1:80").is_err() {
        return None;
    }
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if is_shareable_v4(ip) => Some(ip),
        _ => None,
    }
}

/// 私网地址排前；只要存在私网地址，隧道 / 公网地址就不展示。
fn order_lan_addrs(mut addrs: Vec<Ipv4Addr>) -> Vec<String> {
    // 先数值排序 + 去重，再按「私网优先」稳定排序，保证输出不随网卡枚举顺序抖动。
    addrs.sort_unstable();
    addrs.dedup();
    addrs.sort_by_key(|ip| !is_private_v4(*ip));
    let has_private = addrs.iter().any(|ip| is_private_v4(*ip));
    addrs
        .into_iter()
        .filter(|ip| !has_private || is_private_v4(*ip))
        .map(|ip| ip.to_string())
        .collect()
}

fn is_shareable_v4(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_unspecified() && !ip.is_link_local() && !ip.is_multicast()
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// 候选中优先返回 RFC1918 私网地址；没有私网时退回首个非空候选。
/// 供接入卡片 / CLI 共用，保证「隧道 IP 不抢首位」的全局语义。
pub fn preferred_lan_host(candidates: &[String]) -> Option<String> {
    let trimmed: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.trim())
        .filter(|candidate| !candidate.is_empty())
        .collect();
    trimmed
        .iter()
        .find(|candidate| {
            candidate
                .parse::<Ipv4Addr>()
                .map(is_private_v4)
                .unwrap_or(false)
        })
        .or_else(|| trimmed.first())
        .map(|candidate| (*candidate).to_string())
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

    #[test]
    fn preferred_lan_host_prefers_private_over_tunnel_ip() {
        let candidates = vec!["28.0.0.1".to_string(), "192.168.31.134".to_string()];
        assert_eq!(
            preferred_lan_host(&candidates).as_deref(),
            Some("192.168.31.134")
        );
    }

    #[test]
    fn preferred_lan_host_falls_back_to_first_candidate() {
        let candidates = vec!["28.0.0.1".to_string(), "  ".to_string()];
        assert_eq!(preferred_lan_host(&candidates).as_deref(), Some("28.0.0.1"));
        assert_eq!(preferred_lan_host(&[]), None);
    }

    #[test]
    fn order_lan_addrs_prefers_private_and_drops_tunnel() {
        let ordered = order_lan_addrs(vec![
            Ipv4Addr::new(28, 0, 0, 1),
            Ipv4Addr::new(192, 168, 31, 134),
            Ipv4Addr::new(10, 0, 0, 2),
            Ipv4Addr::new(192, 168, 31, 134),
        ]);
        assert_eq!(ordered, vec!["10.0.0.2", "192.168.31.134"]);
    }

    #[test]
    fn order_lan_addrs_keeps_public_when_no_private() {
        let ordered = order_lan_addrs(vec![Ipv4Addr::new(28, 0, 0, 1)]);
        assert_eq!(ordered, vec!["28.0.0.1"]);
    }
}
