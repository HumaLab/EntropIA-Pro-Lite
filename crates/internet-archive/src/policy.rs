//! Strict fetch policy: which URLs may be fetched and which addresses are
//! public internet (mirroring `url_policy.rs` in the desktop app).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use crate::error::IaError;

/// Which URLs may be fetched.
#[derive(Debug, Clone)]
pub struct FetchPolicy {
    allow_http_loopback: bool,
}

impl FetchPolicy {
    /// Production: only `https` archive.org hosts on port 443.
    pub fn production() -> Self {
        Self {
            allow_http_loopback: false,
        }
    }

    /// Relaxed policy for local tests: additionally allows `http` on
    /// `127.0.0.1` with any port. Only compiled for tests or with the
    /// `test-support` feature so production builds cannot enable it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn loopback_for_tests() -> Self {
        Self {
            allow_http_loopback: true,
        }
    }

    /// Reject anything that is not an allowed scheme/host/port combination.
    /// Host comparison is lowercased with a label boundary so lookalikes such
    /// as `archive.org.evil.com` or `xarchive.org` are rejected.
    pub fn check_url(&self, url: &Url) -> Result<(), IaError> {
        if !url.username().is_empty() || url.password().is_some() {
            return Err(IaError::Blocked {
                reason: "userinfo in URL is not allowed".to_string(),
            });
        }
        let host = url.host_str().ok_or_else(|| IaError::Blocked {
            reason: "URL has no host".to_string(),
        })?;
        let host_lc = host.to_ascii_lowercase();
        // Relaxed local case first: `http://127.0.0.1:<any>/`, userinfo
        // already rejected above. Everything else falls through to the
        // production rules (so `https://archive.org` also works for tests).
        if self.allow_http_loopback && host_lc == "127.0.0.1" && url.scheme() == "http" {
            return Ok(());
        }
        if url.scheme() != "https" {
            return Err(IaError::Blocked {
                reason: format!("scheme '{}' is not allowed", url.scheme()),
            });
        }
        match url.host() {
            Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) => {
                return Err(IaError::Blocked {
                    reason: "IP-literal hosts are not allowed".to_string(),
                });
            }
            Some(Host::Domain(_)) => {}
            None => {
                return Err(IaError::Blocked {
                    reason: "URL has no host".to_string(),
                });
            }
        }
        let host_ok = host_lc == "archive.org"
            || (host_lc.ends_with(".archive.org")
                && host_lc.len() > ".archive.org".len()
                && !host_lc.starts_with('.')
                && !host_lc.contains(".."));
        if !host_ok {
            return Err(IaError::Blocked {
                reason: format!("host '{host}' is not allowed"),
            });
        }
        let port = url.port_or_known_default().unwrap_or(443);
        if port != 443 {
            return Err(IaError::Blocked {
                reason: format!("port {port} is not allowed"),
            });
        }
        Ok(())
    }

    pub(crate) fn ensure_addr_allowed(&self, ip: IpAddr) -> Result<(), IaError> {
        if is_public_ip(ip) {
            return Ok(());
        }
        // The relaxed test policy still blocks every non-public class except
        // loopback, which is where the local test server lives.
        if self.allow_http_loopback && ip.is_loopback() {
            return Ok(());
        }
        Err(IaError::Blocked {
            reason: format!("non-public address: {ip}"),
        })
    }
}

/// True when the address is on the public internet. The predicates mirror
/// `is_public_v4` / `is_public_v6` in
/// `apps/desktop/src-tauri/src/navegador/url_policy.rs`, plus documentation
/// ranges and IPv4-compatible IPv6 forms checked as their embedded IPv4.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _d] = ip.octets();
    if a == 0 {
        return false; // this network / unspecified
    }
    if a == 10 {
        return false;
    }
    if a == 100 && (64..=127).contains(&b) {
        return false; // CGNAT 100.64/10
    }
    if a == 127 {
        return false; // loopback
    }
    if a == 169 && b == 254 {
        return false; // link-local (cloud metadata lives here)
    }
    if a == 172 && (16..=31).contains(&b) {
        return false;
    }
    if a == 192 && b == 0 && c == 0 {
        return false; // IETF protocol assignments
    }
    if a == 192 && b == 0 && c == 2 {
        return false; // documentation TEST-NET-1
    }
    if a == 192 && b == 168 {
        return false;
    }
    if a == 198 && (b == 18 || b == 19) {
        return false; // benchmarking
    }
    if a == 198 && b == 51 && c == 100 {
        return false; // documentation TEST-NET-2
    }
    if a == 203 && b == 0 && c == 113 {
        return false; // documentation TEST-NET-3
    }
    if a >= 224 {
        return false; // multicast, reserved, broadcast
    }
    if ip.is_broadcast() {
        return false;
    }
    true
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    // IPv4-mapped (::ffff:0:0/96) and IPv4-compatible (::: /96) forms carry an
    // IPv4 address; judge them as that IPv4.
    if s[0] == 0
        && s[1] == 0
        && s[2] == 0
        && s[3] == 0
        && s[4] == 0
        && (s[5] == 0 || s[5] == 0xffff)
    {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_public_v4(v4);
    }
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    if ip.is_unspecified() || ip.is_loopback() {
        return false;
    }
    if s[0] & 0xfe00 == 0xfc00 {
        return false; // unique-local fc00::/7
    }
    if s[0] & 0xffc0 == 0xfe80 {
        return false; // link-local fe80::/10
    }
    if s[0] & 0xff00 == 0xff00 {
        return false; // multicast ff00::/8
    }
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return false; // documentation 2001:db8::/32
    }
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) embed an IPv4 address.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_public_v4(v4);
    }
    if s[0] == 0x2002 {
        let v4 = Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8);
        return is_public_v4(v4);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IaError;

    fn check_production(url: &str) -> Result<(), IaError> {
        let parsed = Url::parse(url).expect("test URL must parse");
        FetchPolicy::production().check_url(&parsed)
    }

    #[test]
    fn host_rules_accept() {
        for url in [
            "https://archive.org/",
            "https://ARCHIVE.ORG/",
            "https://dn711501.ca.archive.org/",
            "https://ia801501.us.archive.org/",
            "https://foo.bar.archive.org/",
        ] {
            assert!(check_production(url).is_ok(), "{url} should be accepted");
        }
    }

    #[test]
    fn host_rules_reject_lookalikes() {
        for url in [
            "https://archive.org.evil.com/",
            "https://xarchive.org/",
            "https://fooarchive.org/",
            "https://evil.com/",
            "https://127.0.0.1/",
            "https://1.2.3.4/",
            "https://user@archive.org/",
            "https://user:pass@archive.org/",
        ] {
            assert!(
                matches!(check_production(url), Err(IaError::Blocked { .. })),
                "{url} should be blocked"
            );
        }
    }

    #[test]
    fn scheme_and_port_rejected() {
        assert!(matches!(
            check_production("http://archive.org/"),
            Err(IaError::Blocked { .. })
        ));
        assert!(matches!(
            check_production("ftp://archive.org/"),
            Err(IaError::Blocked { .. })
        ));
        assert!(matches!(
            check_production("https://archive.org:8443/"),
            Err(IaError::Blocked { .. })
        ));
        assert!(matches!(
            check_production("https://archive.org:80/"),
            Err(IaError::Blocked { .. })
        ));
        assert!(check_production("https://archive.org:443/").is_ok());
    }

    #[test]
    fn non_public_ips_blocked() {
        // (addr, should_be_public)
        let cases: Vec<(IpAddr, bool)> = vec![
            (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(172, 31, 255, 255)), false),
            (IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), false),
            (IpAddr::V4(Ipv4Addr::new(169, 254, 10, 20)), false),
            (IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(100, 127, 255, 255)), false),
            (IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)), false),
            (IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(198, 51, 100, 2)), false),
            (IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)), false),
            (IpAddr::V4(Ipv4Addr::new(192, 0, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), true),
            (IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), true),
            (IpAddr::V6(Ipv6Addr::LOCALHOST), false),
            (IpAddr::V6(Ipv6Addr::UNSPECIFIED), false),
            ("fe80::1".parse().unwrap(), false),
            ("fc00::1".parse().unwrap(), false),
            ("fd00::1".parse().unwrap(), false),
            ("ff02::1".parse().unwrap(), false),
            ("2001:db8::1".parse().unwrap(), false),
            ("::ffff:10.0.0.1".parse().unwrap(), false),
            ("::ffff:8.8.8.8".parse().unwrap(), true),
            (
                IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0x0a00, 0x0001)),
                false,
            ),
            ("2606:4700:4700::1111".parse().unwrap(), true),
        ];
        for (ip, public) in cases {
            assert_eq!(is_public_ip(ip), public, "{ip} public={public}");
        }
    }

    #[test]
    fn relaxed_policy_allows_http_loopback() {
        let policy = FetchPolicy::loopback_for_tests();
        let url = Url::parse("http://127.0.0.1:12345/").unwrap();
        assert!(policy.check_url(&url).is_ok());
    }
}
