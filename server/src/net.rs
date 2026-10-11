//! Which URLs a phone can use to reach this server.

use std::net::{IpAddr, Ipv4Addr};

/// Tailscale hands out addresses from the CGNAT block 100.64.0.0/10.
pub fn is_tailscale(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64
}

/// Bridges and container plumbing: private addresses no phone can reach.
fn is_virtual_iface(name: &str) -> bool {
    [
        "lo", "docker", "br-", "veth", "virbr", "cni", "flannel", "podman", "lxc",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}

/// `public` first, then tailscale addresses, then private LAN addresses.
/// A specific `listen` address limits the automatic entries to that address.
pub fn pick_urls(
    public: &[String],
    ifaces: &[(String, IpAddr)],
    listen: IpAddr,
    port: u16,
) -> Vec<String> {
    let mut tailscale = Vec::new();
    let mut lan = Vec::new();
    for (name, ip) in ifaces {
        let IpAddr::V4(v4) = ip else { continue };
        if is_virtual_iface(name) || (!listen.is_unspecified() && listen != *ip) {
            continue;
        }
        if is_tailscale(*v4) {
            tailscale.push(*v4);
        } else if v4.is_private() {
            lan.push(*v4);
        }
    }
    tailscale.sort();
    lan.sort();
    let mut urls: Vec<String> = public
        .iter()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .collect();
    urls.extend(
        tailscale
            .into_iter()
            .chain(lan)
            .map(|ip| format!("http://{ip}:{port}")),
    );
    urls.retain(|u| !u.is_empty());
    let mut seen = std::collections::HashSet::new();
    urls.retain(|u| seen.insert(u.clone()));
    urls
}

pub fn server_urls(public: &[String], listen: IpAddr, port: u16) -> Vec<String> {
    let ifaces = local_ip_address::list_afinet_netifas().unwrap_or_default();
    pick_urls(public, &ifaces, listen, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ifaces() -> Vec<(String, IpAddr)> {
        [
            ("lo", "127.0.0.1"),
            ("wlan0", "192.168.1.20"),
            ("docker0", "172.17.0.1"),
            ("eth0", "8.8.8.8"),
            ("tailscale0", "100.81.125.107"),
            ("eth1", "10.0.0.5"),
            ("eth1", "fe80::1"),
            ("eth2", "100.128.0.1"),
        ]
        .iter()
        .map(|(n, ip)| (n.to_string(), ip.parse().unwrap()))
        .collect()
    }

    #[test]
    fn tailscale_first_then_lan() {
        let urls = pick_urls(&[], &ifaces(), "0.0.0.0".parse().unwrap(), 55566);
        assert_eq!(
            urls,
            [
                "http://100.81.125.107:55566",
                "http://10.0.0.5:55566",
                "http://192.168.1.20:55566"
            ]
        );
    }

    #[test]
    fn public_urls_prepended() {
        let public = vec!["https://t.example.org/".to_string()];
        let urls = pick_urls(&public, &ifaces(), "0.0.0.0".parse().unwrap(), 1);
        assert_eq!(urls[0], "https://t.example.org");
        assert_eq!(urls.len(), 4);
    }

    #[test]
    fn specific_listen_addr_limits_urls() {
        let urls = pick_urls(&[], &ifaces(), "10.0.0.5".parse().unwrap(), 9);
        assert_eq!(urls, ["http://10.0.0.5:9"]);
        assert!(pick_urls(&[], &ifaces(), "127.0.0.1".parse().unwrap(), 9).is_empty());
    }

    #[test]
    fn cgnat_range() {
        assert!(is_tailscale("100.64.0.0".parse().unwrap()));
        assert!(is_tailscale("100.127.255.255".parse().unwrap()));
        assert!(!is_tailscale("100.63.255.255".parse().unwrap()));
        assert!(!is_tailscale("100.128.0.0".parse().unwrap()));
    }
}
