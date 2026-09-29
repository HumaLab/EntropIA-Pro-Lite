//! Which addresses the embedded browser may load.
//!
//! A pure decision over a URL and how it was reached; it does no I/O. The
//! webview layer calls it for the address the person types and again for every
//! navigation, redirect and new window inside the page.
//!
//! Rules:
//! - `https` is allowed. `http` is allowed only when the person typed it
//!   ([`NavigationKind::Typed`]); a link, script or redirect may not downgrade.
//!   The webview layer must remember the typed URL so the initial load of a
//!   typed `http` address (which reaches `on_navigation` as a plain navigation)
//!   is let through, and only that one.
//! - `about:blank` is allowed; every other scheme (`file`, `data`,
//!   `javascript`, `blob`, `ftp`, `ws(s)`, the app's own `tauri`/`asset`/`ipc`,
//!   other `about:` pages) is blocked.
//! - Hosts that are not on the public internet are blocked: `localhost` and
//!   `*.localhost`, loopback, unspecified, private, link-local (cloud metadata
//!   lives at 169.254.169.254), CGNAT, multicast and reserved ranges, IPv4 in
//!   IPv6 forms, and every legacy IPv4 spelling (decimal, octal, hex, short
//!   forms), which the `url` crate's host parser already normalises to an
//!   address before this module sees it. `metadata.google.internal` is blocked
//!   by name.
//!
//! Known limitation: a public hostname whose DNS record points at a private
//! address (DNS rebinding) cannot be recognised without resolving it, and
//! `on_navigation` has no hook for that. It is not covered here.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use tauri::Url;

/// How the address was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationKind {
    /// The person typed it in the address bar.
    Typed,
    /// A link, script or redirect inside the page.
    Navigation,
    /// The page asked for a new window or tab.
    NewWindow,
}

/// Why an address was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocked {
    /// Nothing but whitespace.
    Empty,
    /// Not a URL, or it contains whitespace or control characters.
    Malformed,
    /// A scheme the browser never loads.
    Scheme(String),
    /// `http` where only `https` is allowed.
    InsecureHttp,
    /// A host that is local, private or otherwise not on the public internet.
    Host(String),
}

impl fmt::Display for Blocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Blocked::Empty => write!(f, "The address is empty"),
            Blocked::Malformed => write!(f, "The address is not valid"),
            Blocked::Scheme(scheme) => write!(f, "The \"{scheme}\" scheme is not allowed"),
            Blocked::InsecureHttp => {
                write!(f, "Only https pages load unless you type the http address")
            }
            Blocked::Host(host) => write!(f, "The host \"{host}\" is not on the public internet"),
        }
    }
}

impl std::error::Error for Blocked {}

/// Decide whether `input` may be loaded. Typed input is normalised first
/// (trimmed, `https://` added to a bare host); other kinds are parsed as given.
pub fn check(input: &str, kind: NavigationKind) -> Result<Url, Blocked> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Blocked::Empty);
    }
    if input.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Blocked::Malformed);
    }
    let url = if kind == NavigationKind::Typed && !has_explicit_scheme(input) {
        Url::parse(&format!("https://{}", input.trim_start_matches('/')))
    } else {
        Url::parse(input)
    }
    .map_err(|_| Blocked::Malformed)?;
    check_url(&url, kind)?;
    Ok(url)
}

/// Decide about an already parsed URL, as `on_navigation` receives it.
pub fn check_url(url: &Url, kind: NavigationKind) -> Result<(), Blocked> {
    match url.scheme() {
        "https" => {}
        "http" if kind == NavigationKind::Typed => {}
        "http" => return Err(Blocked::InsecureHttp),
        "about" if url.as_str() == "about:blank" => return Ok(()),
        other => return Err(Blocked::Scheme(other.to_string())),
    }
    // The parser has already turned every IPv4 spelling into dotted decimal
    // and every IPv6 into a bracketed canonical form, so parsing the host
    // string back is exact.
    let host = url.host_str().ok_or(Blocked::Malformed)?;
    let public = if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        inner.parse::<Ipv6Addr>().is_ok_and(is_public_v6)
    } else if let Ok(ip) = host.parse::<Ipv4Addr>() {
        is_public_v4(ip)
    } else {
        is_public_domain(host)
    };
    if public {
        Ok(())
    } else {
        Err(Blocked::Host(host.to_string()))
    }
}

/// True when typed text starts with `scheme:` (`javascript:`, `about:`,
/// `https://`), false for a bare host, with or without a port.
fn has_explicit_scheme(input: &str) -> bool {
    let Some((scheme, rest)) = input.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    let valid_scheme = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let is_port =
        digits > 0 && matches!(rest[digits..].chars().next(), None | Some('/' | '?' | '#'));
    valid_scheme && !is_port
}

fn is_public_domain(domain: &str) -> bool {
    // `localhost.` and `localhost..` name the same host as `localhost`.
    let name = domain.trim_end_matches('.');
    !(name.is_empty()
        || name == "localhost"
        || name.ends_with(".localhost")
        || name == "metadata.google.internal")
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let blocked = a == 0 // this network, unspecified
        || a == 10
        || (a == 100 && (64..=127).contains(&b)) // CGNAT
        || a == 127
        || (a == 169 && b == 254) // link-local, cloud metadata
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19)) // benchmarking
        || a >= 224; // multicast, reserved, broadcast
    !blocked
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    let v4 = |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    if ip.is_unspecified() || ip.is_loopback() {
        return false;
    }
    if s[0] & 0xfe00 == 0xfc00 || s[0] & 0xffc0 == 0xfe80 || s[0] & 0xff00 == 0xff00 {
        return false; // unique local, link-local, multicast
    }
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) carry an IPv4 address.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        return is_public_v4(v4(s[6], s[7]));
    }
    if s[0] == 0x2002 {
        return is_public_v4(v4(s[1], s[2]));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use NavigationKind::*;

    fn allowed(input: &str, kind: NavigationKind) -> String {
        match check(input, kind) {
            Ok(url) => url.to_string(),
            Err(reason) => panic!("{input:?} ({kind:?}) should pass, got {reason:?}"),
        }
    }

    fn blocked(input: &str, kind: NavigationKind) -> Blocked {
        match check(input, kind) {
            Ok(url) => panic!("{input:?} ({kind:?}) should be blocked, passed as {url}"),
            Err(reason) => reason,
        }
    }

    fn is_host(reason: &Blocked) -> bool {
        matches!(reason, Blocked::Host(_))
    }

    #[test]
    fn https_passes_for_every_kind() {
        for kind in [Typed, Navigation, NewWindow] {
            assert_eq!(
                allowed("https://example.com/a?b=1#c", kind),
                "https://example.com/a?b=1#c"
            );
        }
    }

    #[test]
    fn http_passes_only_when_typed() {
        assert_eq!(allowed("http://example.com/", Typed), "http://example.com/");
        assert_eq!(
            blocked("http://example.com/", Navigation),
            Blocked::InsecureHttp
        );
        assert_eq!(
            blocked("http://example.com/", NewWindow),
            Blocked::InsecureHttp
        );
    }

    #[test]
    fn a_bare_typed_host_becomes_https() {
        assert_eq!(allowed("example.com", Typed), "https://example.com/");
        assert_eq!(
            allowed("example.com/path?q=1", Typed),
            "https://example.com/path?q=1"
        );
        assert_eq!(allowed("  example.com  ", Typed), "https://example.com/");
        assert_eq!(
            allowed("example.com:8443/x", Typed),
            "https://example.com:8443/x"
        );
        assert_eq!(allowed("//example.com/x", Typed), "https://example.com/x");
    }

    #[test]
    fn bare_hosts_are_not_normalised_for_non_typed_kinds() {
        assert_eq!(blocked("example.com", Navigation), Blocked::Malformed);
        assert_eq!(blocked("example.com", NewWindow), Blocked::Malformed);
    }

    #[test]
    fn empty_and_malformed_input_is_rejected() {
        assert_eq!(blocked("", Typed), Blocked::Empty);
        assert_eq!(blocked("   \t ", Typed), Blocked::Empty);
        assert_eq!(blocked("", Navigation), Blocked::Empty);
        for input in [
            "exa mple.com",
            "https://exa mple.com/",
            "example.com/a\nb",
            "example.com/\u{0}",
            "https://example.com/\u{7f}",
            "https://",
            "https:///",
            "http://[::1",
        ] {
            assert_eq!(blocked(input, Typed), Blocked::Malformed, "{input:?}");
        }
    }

    #[test]
    fn dangerous_schemes_are_blocked_however_they_are_spelled() {
        for (input, scheme) in [
            ("file:///C:/Windows/win.ini", "file"),
            ("FILE:///etc/passwd", "file"),
            ("data:text/html,<script>1</script>", "data"),
            ("javascript:alert(1)", "javascript"),
            ("JavaScript:alert(1)", "javascript"),
            ("blob:https://example.com/1234", "blob"),
            ("about:config", "about"),
            ("about:blank#x", "about"),
            ("ftp://example.com/", "ftp"),
            ("ws://example.com/", "ws"),
            ("wss://example.com/", "wss"),
            ("tauri://localhost/", "tauri"),
            ("asset://localhost/x", "asset"),
            ("ipc://localhost/x", "ipc"),
            ("mailto:a@example.com", "mailto"),
            ("view-source:https://example.com/", "view-source"),
        ] {
            for kind in [Typed, Navigation, NewWindow] {
                assert_eq!(
                    blocked(input, kind),
                    Blocked::Scheme(scheme.into()),
                    "{input:?} {kind:?}"
                );
            }
        }
    }

    #[test]
    fn about_blank_is_the_only_about_page() {
        for kind in [Typed, Navigation, NewWindow] {
            assert_eq!(allowed("about:blank", kind), "about:blank");
        }
    }

    #[test]
    fn local_hostnames_are_blocked() {
        for input in [
            "https://localhost/",
            "https://LOCALHOST/",
            "https://localhost./",
            "https://localhost../",
            "https://app.localhost/",
            "https://a.b.localhost./",
            "http://tauri.localhost/",
            "https://ipc.localhost/",
            "https://asset.localhost/",
            "https://metadata.google.internal/",
            "https://METADATA.GOOGLE.INTERNAL./computeMetadata/v1/",
        ] {
            assert!(is_host(&blocked(input, Typed)), "{input}");
        }
        assert!(is_host(&blocked("localhost", Typed)));
        assert!(is_host(&blocked("localhost:8080", Typed)));
    }

    #[test]
    fn non_public_ipv4_ranges_are_blocked() {
        for host in [
            "0.0.0.0",
            "0.1.2.3",
            "127.0.0.1",
            "127.255.255.254",
            "10.0.0.1",
            "10.255.255.255",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "192.168.255.255",
            "169.254.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "192.0.0.1",
            "198.18.0.1",
            "198.19.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(
                is_host(&blocked(&format!("https://{host}/"), Typed)),
                "{host}"
            );
            assert!(
                is_host(&blocked(&format!("http://{host}:8080/x"), Typed)),
                "{host}"
            );
        }
    }

    #[test]
    fn public_neighbours_of_the_blocked_ipv4_ranges_pass() {
        for host in [
            "1.1.1.1",
            "8.8.8.8",
            "11.0.0.1",
            "126.255.255.255",
            "128.0.0.1",
            "172.15.255.255",
            "172.32.0.1",
            "192.167.255.255",
            "192.169.0.1",
            "169.253.255.255",
            "169.255.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "198.17.255.255",
            "198.20.0.1",
            "223.255.255.255",
        ] {
            assert_eq!(
                allowed(&format!("https://{host}/"), Typed),
                format!("https://{host}/")
            );
        }
    }

    #[test]
    fn legacy_ipv4_spellings_are_blocked() {
        for input in [
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0X7F.0.0.1/",
            "http://127.1/",
            "http://127.0.1/",
            "http://0177.0.0.1/",
            "http://017700000001/",
            "http://0x7f.1/",
            "http://0/",
            "http://0.0.0.0./",
            "http://127.0.0.1./",
            "http://2852039166/",
            "http://0xa9fea9fe/",
            "http://167772161/",
            "http://3232235521/",
        ] {
            assert!(is_host(&blocked(input, Typed)), "{input}");
        }
    }

    #[test]
    fn non_public_ipv6_is_blocked_with_brackets() {
        for host in [
            "[::1]",
            "[::]",
            "[0:0:0:0:0:0:0:1]",
            "[fc00::1]",
            "[fd12:3456::1]",
            "[fe80::1]",
            "[febf::1]",
            "[ff02::1]",
            "[::ffff:127.0.0.1]",
            "[::ffff:7f00:1]",
            "[::ffff:10.0.0.1]",
            "[::ffff:192.168.1.1]",
            "[::ffff:169.254.169.254]",
            "[::ffff:a9fe:a9fe]",
            "[::ffff:100.64.0.1]",
            "[64:ff9b::7f00:1]",
            "[64:ff9b::a9fe:a9fe]",
            "[2002:7f00:1::]",
            "[2002:a9fe:a9fe::1]",
        ] {
            assert!(
                is_host(&blocked(&format!("https://{host}/"), Typed)),
                "{host}"
            );
            assert!(
                is_host(&blocked(&format!("https://{host}:8443/"), Navigation)),
                "{host}"
            );
        }
    }

    #[test]
    fn public_ipv6_passes() {
        for host in [
            "[2606:4700:4700::1111]",
            "[2001:4860:4860::8888]",
            "[::ffff:8.8.8.8]",
            "[fe00::1]",
            "[fbff::1]",
        ] {
            assert!(check(&format!("https://{host}/"), Typed).is_ok(), "{host}");
        }
    }

    #[test]
    fn userinfo_does_not_hide_a_blocked_host() {
        assert!(is_host(&blocked("https://user@127.0.0.1/", Typed)));
        assert!(is_host(&blocked("https://user:pw@localhost/", Navigation)));
        assert!(is_host(&blocked("https://example.com@127.0.0.1/", Typed)));
        assert!(is_host(&blocked(
            "https://example.com%40127.0.0.1@localhost/",
            Typed
        )));
        assert_eq!(
            allowed("https://127.0.0.1@example.com/", Typed),
            "https://127.0.0.1@example.com/"
        );
    }

    #[test]
    fn ports_do_not_change_the_verdict() {
        assert_eq!(
            allowed("https://example.com:8443/x", Navigation),
            "https://example.com:8443/x"
        );
        assert_eq!(
            allowed("http://example.com:8080/", Typed),
            "http://example.com:8080/"
        );
        assert!(is_host(&blocked("https://localhost:443/", Typed)));
    }

    #[test]
    fn idn_and_punycode_hosts_pass() {
        assert_eq!(
            allowed("https://xn--bcher-kva.example/", Typed),
            "https://xn--bcher-kva.example/"
        );
        assert_eq!(
            allowed("https://bücher.example/", Typed),
            "https://xn--bcher-kva.example/"
        );
        assert_eq!(allowed("münchen.de", Typed), "https://xn--mnchen-3ya.de/");
    }

    #[test]
    fn hosts_that_only_look_local_pass() {
        for input in [
            "https://localhost.example.com/",
            "https://notlocalhost/",
            "https://example.localhost.com/",
            "https://metadata.google.internal.example.com/",
            "https://127.0.0.1.example.com/",
        ] {
            assert!(check(input, Typed).is_ok(), "{input}");
        }
    }

    #[test]
    fn check_url_matches_check_for_parsed_urls() {
        let good: Url = "https://example.com/".parse().unwrap();
        let bad: Url = "https://192.168.1.1/".parse().unwrap();
        let http: Url = "http://example.com/".parse().unwrap();
        assert_eq!(check_url(&good, Navigation), Ok(()));
        assert!(is_host(&check_url(&bad, Navigation).unwrap_err()));
        assert_eq!(check_url(&http, Navigation), Err(Blocked::InsecureHttp));
        assert_eq!(check_url(&http, Typed), Ok(()));
    }

    #[test]
    fn blocked_reasons_read_as_messages() {
        assert!(Blocked::Empty.to_string().contains("empty"));
        assert!(Blocked::Scheme("file".into()).to_string().contains("file"));
        assert!(Blocked::Host("localhost".into())
            .to_string()
            .contains("localhost"));
    }
}
