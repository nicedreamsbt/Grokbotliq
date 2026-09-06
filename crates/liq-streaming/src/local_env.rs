//! Load gitignored `config/local.env` (and optional `.env`) at runtime.
//! Never logs secret values — only keys and host-redacted URLs.

use crate::redact::rpc_url_host_only;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parse KEY=VALUE lines (no export, simple quotes).
pub fn parse_env_file(contents: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        if k.is_empty() {
            continue;
        }
        let mut v = v.trim().to_string();
        if (v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')) {
            v = v[1..v.len() - 1].to_string();
        }
        map.insert(k.to_string(), v);
    }
    map
}

fn default_local_env_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("config/local.env"),
        PathBuf::from(".env"),
    ]
}

/// Load env file(s) into process env if the key is not already set.
/// Returns list of (path, keys_loaded) for logging (values never included).
pub fn load_local_env_files(extra: Option<&Path>) -> Vec<(PathBuf, Vec<String>)> {
    let mut paths = default_local_env_paths();
    if let Some(p) = extra {
        paths.insert(0, p.to_path_buf());
    }
    let mut loaded = Vec::new();
    for path in paths {
        let Ok(s) = std::fs::read_to_string(&path) else {
            continue;
        };
        let map = parse_env_file(&s);
        let mut keys = Vec::new();
        for (k, v) in map {
            keys.push(k.clone());
            if std::env::var_os(&k).is_none() {
                std::env::set_var(&k, &v);
            }
        }
        keys.sort();
        loaded.push((path, keys));
    }
    loaded
}

/// Collect RPC endpoint URLs from env: `RPC_URLS` (comma-separated) then `RPC_URL`.
pub fn rpc_urls_from_env() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(urls) = std::env::var("RPC_URLS") {
        for part in urls.split(',') {
            let t = part.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
        }
    }
    if out.is_empty() {
        if let Ok(u) = std::env::var("RPC_URL") {
            let t = u.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
        }
    }
    // Dedup preserving order
    let mut seen = std::collections::HashSet::new();
    out.retain(|u| seen.insert(u.clone()));
    out
}

/// Host-only list for logs / reports.
pub fn rpc_hosts_redacted(urls: &[String]) -> Vec<String> {
    urls.iter().map(|u| rpc_url_host_only(u)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_env() {
        let m = parse_env_file("RPC_URL=https://a.example/key\n# c\nDRY_RUN=true\n");
        assert_eq!(m.get("RPC_URL").unwrap(), "https://a.example/key");
        assert_eq!(m.get("DRY_RUN").unwrap(), "true");
    }

    #[test]
    fn rpc_urls_split() {
        std::env::remove_var("RPC_URLS");
        std::env::remove_var("RPC_URL");
        std::env::set_var("RPC_URLS", "https://a/x, https://b/y");
        let v = rpc_urls_from_env();
        assert_eq!(v.len(), 2);
        std::env::remove_var("RPC_URLS");
    }
}
