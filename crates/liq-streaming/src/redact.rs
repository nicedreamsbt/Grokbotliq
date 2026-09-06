//! Redact secrets from RPC URLs for logs / reports (host-only).

use std::fmt;

/// Return `scheme://host` only — strip path, query, fragment, userinfo.
pub fn rpc_url_host_only(url: &str) -> String {
    let t = url.trim();
    if t.is_empty() {
        return String::new();
    }
    // Avoid pulling in url crate: parse manually.
    let without_scheme = if let Some(rest) = t.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = t.strip_prefix("http://") {
        ("http", rest)
    } else {
        return "<redacted>".into();
    };
    let (scheme, rest) = without_scheme;
    let hostport = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split('@')
        .next_back()
        .unwrap_or("");
    if hostport.is_empty() {
        return "<redacted>".into();
    }
    format!("{scheme}://{hostport}")
}

/// Short base58 pubkey for logs (first 4 + … + last 4 chars).
pub fn short_b58(s: &str) -> String {
    let t = s.trim();
    if t.len() <= 10 {
        return t.to_string();
    }
    format!("{}…{}", &t[..4], &t[t.len() - 4..])
}

/// Wrapper that Display-redacts an RPC URL to host-only.
pub struct RedactedUrl<'a>(pub &'a str);

impl fmt::Display for RedactedUrl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", rpc_url_host_only(self.0))
    }
}

impl fmt::Debug for RedactedUrl<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", rpc_url_host_only(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_api_key_path_and_query() {
        let u = "https://mainnet.helius-rpc.com/?api-key=SECRET123";
        assert_eq!(rpc_url_host_only(u), "https://mainnet.helius-rpc.com");
        let u2 = "https://mainnet.helius-rpc.com/SECRET123";
        assert_eq!(rpc_url_host_only(u2), "https://mainnet.helius-rpc.com");
    }

    #[test]
    fn short_b58_truncates() {
        let s = "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF";
        let out = short_b58(s);
        assert!(out.contains('…'));
        assert!(!out.contains("CoErrtyc"));
    }
}
