//! Trusted reverse-proxy IP resolution.
//!
//! When the relay sits behind nginx/Caddy (or Cloudflare), the TCP peer the
//! server sees is the proxy, not the client. Per-IP rate limiting would then
//! key every client to the proxy's IP unless the real client address is
//! recovered from a forwarded header.
//!
//! SECURITY MODEL
//! --------------
//! - Forwarded headers are ONLY honored when the direct peer address is in
//!   the operator-configured trust list (`WHISPER_TRUSTED_PROXIES`).
//! - A client that connects directly can never spoof `X-Forwarded-For`: its
//!   address is not trusted, so the headers are ignored and the direct
//!   address is used for rate limiting.
//! - The operator proxy MUST overwrite (not append) `X-Forwarded-For`, and
//!   the trust list must contain exactly the proxies in front of the relay
//!   (an open trust list would let any client claim any IP).
//! - `CF-Connecting-IP` is honored for Cloudflare deployments; it carries a
//!   single verified client address set by Cloudflare's edge.
//! - For `X-Forwarded-For` chains, the left-most valid entry is the original
//!   client (the value the trusted proxy wrote); entries are parsed left to
//!   right and the first syntactically valid IP wins.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;

/// Header name for the Cloudflare-verified client address (single IP).
const CF_CONNECTING_IP: &str = "cf-connecting-ip";
/// Header name for the RFC 7239-style proxy chain (comma-separated IPs).
const X_FORWARDED_FOR: &str = "x-forwarded-for";

/// Trusted reverse proxies in front of the relay.
///
/// Built from `WHISPER_TRUSTED_PROXIES` — a comma- and/or space-separated
/// list of IP addresses (e.g. `127.0.0.1, 10.0.0.5`). When the list is empty
/// no forwarded header is ever honored and every connection is treated as
/// direct — the safe default.
#[derive(Clone, Debug, Default)]
pub(crate) struct TrustedProxies {
    ips: HashSet<IpAddr>,
}

impl TrustedProxies {
    /// Build from the `WHISPER_TRUSTED_PROXIES` environment variable.
    pub(crate) fn from_env() -> Self {
        let raw = std::env::var("WHISPER_TRUSTED_PROXIES").unwrap_or_default();
        Self::parse(&raw)
    }

    /// Parse a comma/space-separated proxy list; invalid entries are skipped
    /// with a warning, never a panic.
    fn parse(raw: &str) -> Self {
        let mut ips = HashSet::new();
        for part in raw.split([',', ' ', '\t', '\n']) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match part.parse::<IpAddr>() {
                Ok(ip) => {
                    ips.insert(ip);
                }
                Err(_) => {
                    tracing::warn!(value = %part, "ignoring invalid trusted-proxy entry");
                }
            }
        }
        Self { ips }
    }

    /// Whether the direct TCP peer address is a configured trusted proxy.
    fn is_trusted(&self, direct: &SocketAddr) -> bool {
        self.ips.contains(&direct.ip())
    }

    /// Resolve the real client address for rate limiting.
    ///
    /// - Direct (untrusted) peers resolve to their own address — forwarded
    ///   headers are ignored, so spoofing is impossible.
    /// - Trusted proxies resolve to the client address from
    ///   `CF-Connecting-IP`, or the left-most valid entry of
    ///   `X-Forwarded-For`; when the headers are absent or malformed the
    ///   proxy's own address is used as the fallback.
    pub(crate) fn resolve_client_ip(&self, direct: SocketAddr, headers: &HeaderMap) -> SocketAddr {
        if !self.is_trusted(&direct) {
            return direct;
        }

        // Cloudflare sets CF-Connecting-IP to a single verified client IP.
        if let Some(ip) = single_header_ip(headers, CF_CONNECTING_IP) {
            return SocketAddr::new(ip, direct.port());
        }

        // X-Forwarded-For: the left-most valid entry is the original client.
        if let Some(ip) = headers
            .get(X_FORWARDED_FOR)
            .and_then(|v| v.to_str().ok())
            .and_then(first_valid_ip)
        {
            return SocketAddr::new(ip, direct.port());
        }

        // Nothing usable in the headers — fall back to the proxy itself.
        direct
    }
}

/// Read a header expected to hold exactly one IP address.
fn single_header_ip(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
}

/// First syntactically valid IP in a comma/space-separated chain.
fn first_valid_ip(chain: &str) -> Option<IpAddr> {
    chain
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .find_map(|part| part.parse::<IpAddr>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn socket(ip: &str) -> SocketAddr {
        // IPv6 literals must be bracketed before appending the port.
        if ip.contains(':') && !ip.starts_with('[') {
            format!("[{ip}]:9999").parse().unwrap()
        } else {
            format!("{ip}:9999").parse().unwrap()
        }
    }

    fn xff(headers: &mut HeaderMap, value: &str) {
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_str(value).expect("valid header value"),
        );
    }

    fn cf(headers: &mut HeaderMap, value: &str) {
        headers.insert(
            "cf-connecting-ip",
            HeaderValue::from_str(value).expect("valid header value"),
        );
    }

    #[test]
    fn parse_builds_set_from_comma_and_space_list() {
        let p = TrustedProxies::parse("127.0.0.1, ::1 10.0.0.5");
        assert!(p.is_trusted(&socket("127.0.0.1")));
        assert!(p.is_trusted(&socket("::1")));
        assert!(p.is_trusted(&socket("10.0.0.5")));
        assert!(!p.is_trusted(&socket("192.168.1.9")));
    }

    #[test]
    fn parse_skips_invalid_entries_without_panicking() {
        let p = TrustedProxies::parse("127.0.0.1, not-an-ip, 10.0.0.5");
        assert!(p.is_trusted(&socket("127.0.0.1")));
        assert!(p.is_trusted(&socket("10.0.0.5")));
        assert!(!p.is_trusted(&socket("192.168.1.9")));
    }

    #[test]
    fn empty_list_never_trusts_anyone() {
        let p = TrustedProxies::default();
        assert!(!p.is_trusted(&socket("127.0.0.1")));
    }

    #[test]
    fn untrusted_direct_peer_ignores_forwarded_headers() {
        // The classic spoofing attempt: a direct client claims a victim IP.
        let p = TrustedProxies::parse("10.0.0.5");
        let mut headers = HeaderMap::new();
        xff(&mut headers, "203.0.113.7");
        let resolved = p.resolve_client_ip(socket("198.51.100.3"), &headers);
        assert_eq!(resolved.ip().to_string(), "198.51.100.3");
    }

    #[test]
    fn trusted_proxy_reads_x_forwarded_for() {
        let p = TrustedProxies::parse("10.0.0.5");
        let mut headers = HeaderMap::new();
        xff(&mut headers, "203.0.113.7");
        let resolved = p.resolve_client_ip(socket("10.0.0.5"), &headers);
        assert_eq!(resolved.ip().to_string(), "203.0.113.7");
    }

    #[test]
    fn trusted_proxy_takes_leftmost_valid_xff_entry() {
        let p = TrustedProxies::parse("10.0.0.5");
        let mut headers = HeaderMap::new();
        xff(&mut headers, "203.0.113.7, 10.0.0.9");
        let resolved = p.resolve_client_ip(socket("10.0.0.5"), &headers);
        assert_eq!(resolved.ip().to_string(), "203.0.113.7");
    }

    #[test]
    fn trusted_proxy_prefers_cf_connecting_ip() {
        let p = TrustedProxies::parse("10.0.0.5");
        let mut headers = HeaderMap::new();
        xff(&mut headers, "198.51.100.9");
        cf(&mut headers, "203.0.113.7");
        let resolved = p.resolve_client_ip(socket("10.0.0.5"), &headers);
        assert_eq!(resolved.ip().to_string(), "203.0.113.7");
    }

    #[test]
    fn malformed_forwarded_headers_fall_back_to_proxy_ip() {
        let p = TrustedProxies::parse("10.0.0.5");
        let mut headers = HeaderMap::new();
        xff(&mut headers, "garbage, also-not-an-ip");
        let resolved = p.resolve_client_ip(socket("10.0.0.5"), &headers);
        assert_eq!(resolved.ip().to_string(), "10.0.0.5");
    }

    #[test]
    fn missing_headers_fall_back_to_proxy_ip() {
        let p = TrustedProxies::parse("10.0.0.5");
        let resolved = p.resolve_client_ip(socket("10.0.0.5"), &HeaderMap::new());
        assert_eq!(resolved.ip().to_string(), "10.0.0.5");
    }
}
