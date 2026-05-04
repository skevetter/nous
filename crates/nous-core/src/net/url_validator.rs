use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

use crate::error::NousError;

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    matches!(
        octets,
        [127 | 10 | 0, ..]
            | [172, 16..=31, ..]
            | [192, 168, ..]
            | [169, 254, ..]
    )
}

fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()                                     // ::1
        || ip.is_unspecified()                           // ::
        || matches!(ip.segments()[0], 0xfc00..=0xfdff)   // fc00::/7 unique local
        || matches!(ip.segments()[0], 0xfe80..=0xfebf) // fe80::/10 link-local
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_ipv4(v4),
        IpAddr::V6(v6) => is_private_ipv6(v6),
    }
}

/// Validates a URL is safe for server-side requests (anti-SSRF).
///
/// Rejects non-http(s) schemes, private/loopback IPs, and hostnames
/// that resolve to private addresses.
pub fn validate_url(url: &str) -> Result<(), NousError> {
    let parsed =
        reqwest::Url::parse(url).map_err(|e| NousError::Validation(format!("invalid URL: {e}")))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(NousError::Validation(format!(
                "scheme '{scheme}' not allowed, only http and https are permitted"
            )));
        }
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| NousError::Validation("URL has no host".to_string()))?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err(NousError::Validation(format!(
                "requests to private IP {ip} are not allowed"
            )));
        }
        return Ok(());
    }

    let port = parsed.port_or_known_default().unwrap_or(80);
    let socket_addr = format!("{host}:{port}");
    let resolved: Vec<_> = socket_addr
        .to_socket_addrs()
        .map_err(|e| NousError::Validation(format!("DNS resolution failed for {host}: {e}")))?
        .collect();

    if resolved.is_empty() {
        return Err(NousError::Validation(format!(
            "DNS resolution returned no addresses for {host}"
        )));
    }

    for addr in &resolved {
        if is_private_ip(addr.ip()) {
            return Err(NousError::Validation(format!(
                "host {host} resolves to private IP {}, requests not allowed",
                addr.ip()
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_public_http_url() {
        assert!(validate_url("http://example.com/path").is_ok());
    }

    #[test]
    fn allows_public_https_url() {
        assert!(validate_url("https://example.com/path").is_ok());
    }

    #[test]
    fn rejects_ftp_scheme() {
        let err = validate_url("ftp://example.com/file").unwrap_err();
        assert!(err.to_string().contains("scheme 'ftp' not allowed"));
    }

    #[test]
    fn rejects_file_scheme() {
        let err = validate_url("file:///etc/passwd").unwrap_err();
        assert!(err.to_string().contains("scheme 'file' not allowed"));
    }

    #[test]
    fn rejects_gopher_scheme() {
        let err = validate_url("gopher://evil.com").unwrap_err();
        assert!(err.to_string().contains("scheme 'gopher' not allowed"));
    }

    #[test]
    fn rejects_localhost_127() {
        let err = validate_url("http://127.0.0.1/admin").unwrap_err();
        assert!(err.to_string().contains("private IP"));
    }

    #[test]
    fn rejects_localhost_name() {
        let err = validate_url("http://localhost/admin").unwrap_err();
        assert!(err.to_string().contains("private IP") || err.to_string().contains("resolves to"));
    }

    #[test]
    fn rejects_10_network() {
        let err = validate_url("http://10.0.0.1/internal").unwrap_err();
        assert!(err.to_string().contains("private IP"));
    }

    #[test]
    fn rejects_172_16_network() {
        let err = validate_url("http://172.16.0.1/internal").unwrap_err();
        assert!(err.to_string().contains("private IP"));
    }

    #[test]
    fn rejects_192_168_network() {
        let err = validate_url("http://192.168.1.1/internal").unwrap_err();
        assert!(err.to_string().contains("private IP"));
    }

    #[test]
    fn rejects_169_254_link_local() {
        let err = validate_url("http://169.254.169.254/metadata").unwrap_err();
        assert!(err.to_string().contains("private IP"));
    }

    #[test]
    fn rejects_ipv6_loopback() {
        let err = validate_url("http://[::1]/admin").unwrap_err();
        assert!(err.to_string().contains("private IP"));
    }

    #[test]
    fn rejects_invalid_url() {
        assert!(validate_url("not a url").is_err());
    }

    #[test]
    fn allows_172_non_private() {
        assert!(validate_url("http://172.32.0.1/path").is_ok());
    }

    #[test]
    fn rejects_zero_network() {
        let err = validate_url("http://0.0.0.0/").unwrap_err();
        assert!(err.to_string().contains("private IP"));
    }
}
