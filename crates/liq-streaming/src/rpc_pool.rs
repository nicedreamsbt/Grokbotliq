//! Rotating / failover JSON-RPC pool over multiple endpoints.
//! On 429 / 5xx / timeout, advances to the next URL. Tracks per-endpoint latency + errors.
//! Never logs full URLs (host-only via redact).

use crate::bootstrap::{BootstrapError, HttpJsonRpcTransport, JsonRpcTransport};
use crate::redact::{rpc_url_host_only, RedactedUrl};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Per-endpoint telemetry counters (safe to serialize into reports).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndpointStats {
    pub host: String,
    pub requests: u64,
    pub successes: u64,
    pub errors: u64,
    pub rotations_away: u64,
    /// Cumulative latency in microseconds.
    pub latency_us_sum: u64,
    pub last_error_kind: Option<String>,
}

impl EndpointStats {
    pub fn avg_latency_us(&self) -> Option<u64> {
        if self.successes == 0 {
            None
        } else {
            Some(self.latency_us_sum / self.successes)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcFailKind {
    RateLimited,
    ServerError,
    Timeout,
    Transport,
    RpcApp,
    Other,
}

impl RpcFailKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RateLimited => "429",
            Self::ServerError => "5xx",
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::RpcApp => "rpc",
            Self::Other => "other",
        }
    }

    pub fn should_rotate(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::ServerError | Self::Timeout | Self::Transport
        )
    }
}

/// Classify bootstrap/http errors for rotation decisions.
pub fn classify_error(err: &BootstrapError) -> RpcFailKind {
    match err {
        BootstrapError::Http(s) | BootstrapError::Rpc(s) => {
            let lower = s.to_lowercase();
            if lower.contains("429") || lower.contains("too many request") {
                RpcFailKind::RateLimited
            } else if lower.contains("500")
                || lower.contains("502")
                || lower.contains("503")
                || lower.contains("504")
                || lower.contains("http 5")
            {
                RpcFailKind::ServerError
            } else if lower.contains("timeout")
                || lower.contains("timed out")
                || lower.contains("deadline")
            {
                RpcFailKind::Timeout
            } else if matches!(err, BootstrapError::Http(_)) {
                RpcFailKind::Transport
            } else {
                RpcFailKind::RpcApp
            }
        }
        BootstrapError::Config(_) | BootstrapError::Decode(_) => RpcFailKind::Other,
    }
}

struct PoolInner {
    urls: Vec<String>,
    index: AtomicUsize,
    stats: Mutex<Vec<EndpointStats>>,
    /// Max attempts per call (= urls.len() unless overridden).
    max_attempts: usize,
    timeout: Duration,
}

/// Rotating JSON-RPC transport: tries current endpoint, on retriable failure advances.
#[derive(Clone)]
pub struct RotatingRpcPool {
    inner: Arc<PoolInner>,
    /// Optional override for unit tests (inject mock per-URL handlers).
    mock: Option<Arc<Mutex<Box<dyn FnMut(usize, &Value) -> Result<Value, BootstrapError> + Send>>>>,
}

impl std::fmt::Debug for RotatingRpcPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hosts: Vec<_> = self
            .inner
            .urls
            .iter()
            .map(|u| rpc_url_host_only(u))
            .collect();
        f.debug_struct("RotatingRpcPool")
            .field("hosts", &hosts)
            .field("index", &self.inner.index.load(Ordering::Relaxed))
            .finish()
    }
}

impl RotatingRpcPool {
    pub fn from_urls(urls: Vec<String>) -> Result<Self, BootstrapError> {
        let urls: Vec<String> = urls
            .into_iter()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty())
            .collect();
        if urls.is_empty() {
            return Err(BootstrapError::Config(
                "no RPC URLs configured (set RPC_URLS or RPC_URL in config/local.env)".into(),
            ));
        }
        for u in &urls {
            if u.contains("YOUR_") {
                return Err(BootstrapError::Config(format!(
                    "placeholder RPC URL ({})",
                    rpc_url_host_only(u)
                )));
            }
        }
        let stats = urls
            .iter()
            .map(|u| EndpointStats {
                host: rpc_url_host_only(u),
                ..Default::default()
            })
            .collect();
        let n = urls.len();
        Ok(Self {
            inner: Arc::new(PoolInner {
                urls,
                index: AtomicUsize::new(0),
                stats: Mutex::new(stats),
                max_attempts: n,
                timeout: Duration::from_secs(20),
            }),
            mock: None,
        })
    }

    /// Test constructor with mock transport (no real HTTP).
    pub fn mock_for_tests<F>(url_count: usize, f: F) -> Self
    where
        F: FnMut(usize, &Value) -> Result<Value, BootstrapError> + Send + 'static,
    {
        let urls: Vec<String> = (0..url_count)
            .map(|i| format!("https://mock-{i}.example/key"))
            .collect();
        let stats = urls
            .iter()
            .map(|u| EndpointStats {
                host: rpc_url_host_only(u),
                ..Default::default()
            })
            .collect();
        Self {
            inner: Arc::new(PoolInner {
                urls,
                index: AtomicUsize::new(0),
                stats: Mutex::new(stats),
                max_attempts: url_count.max(1),
                timeout: Duration::from_secs(5),
            }),
            mock: Some(Arc::new(Mutex::new(Box::new(f)))),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.urls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.urls.is_empty()
    }

    pub fn current_index(&self) -> usize {
        self.inner.index.load(Ordering::Relaxed) % self.inner.urls.len().max(1)
    }

    pub fn current_host(&self) -> String {
        let i = self.current_index();
        rpc_url_host_only(&self.inner.urls[i])
    }

    pub fn stats_snapshot(&self) -> Vec<EndpointStats> {
        self.inner.stats.lock().clone()
    }

    fn advance(&self, from: usize, kind: RpcFailKind) {
        let n = self.inner.urls.len();
        if n == 0 {
            return;
        }
        let next = (from + 1) % n;
        self.inner.index.store(next, Ordering::Relaxed);
        {
            let mut stats = self.inner.stats.lock();
            if let Some(s) = stats.get_mut(from) {
                s.rotations_away += 1;
                s.last_error_kind = Some(kind.as_str().into());
            }
        }
        warn!(
            from_host = %RedactedUrl(&self.inner.urls[from]),
            to_host = %RedactedUrl(&self.inner.urls[next]),
            kind = kind.as_str(),
            "rpc pool rotated endpoint"
        );
    }

    async fn post_once(&self, idx: usize, body: &Value) -> Result<Value, BootstrapError> {
        {
            let mut stats = self.inner.stats.lock();
            if let Some(s) = stats.get_mut(idx) {
                s.requests += 1;
            }
        }
        let start = Instant::now();
        let result = if let Some(mock) = &self.mock {
            let mut h = mock.lock();
            h(idx, body)
        } else {
            let url = &self.inner.urls[idx];
            let transport = HttpJsonRpcTransport::unchecked(url.clone());
            // Soft timeout via tokio — reqwest timeout is also set on client in unchecked (default).
            match tokio::time::timeout(self.inner.timeout, transport.post_json(body)).await {
                Ok(r) => r,
                Err(_) => Err(BootstrapError::Http("timeout waiting for RPC".into())),
            }
        };
        let elapsed = start.elapsed().as_micros() as u64;
        match &result {
            Ok(_) => {
                let mut stats = self.inner.stats.lock();
                if let Some(s) = stats.get_mut(idx) {
                    s.successes += 1;
                    s.latency_us_sum = s.latency_us_sum.saturating_add(elapsed);
                }
                debug!(
                    host = %RedactedUrl(&self.inner.urls[idx]),
                    latency_us = elapsed,
                    "rpc ok"
                );
            }
            Err(e) => {
                let kind = classify_error(e);
                let mut stats = self.inner.stats.lock();
                if let Some(s) = stats.get_mut(idx) {
                    s.errors += 1;
                    s.last_error_kind = Some(kind.as_str().into());
                }
            }
        }
        result
    }
}

#[async_trait]
impl JsonRpcTransport for RotatingRpcPool {
    async fn post_json(&self, body: &Value) -> Result<Value, BootstrapError> {
        let n = self.inner.urls.len();
        let attempts = self.inner.max_attempts.max(1).min(n.max(1));
        let start = self.current_index();
        let mut last_err = BootstrapError::Rpc("no attempts".into());
        for attempt in 0..attempts {
            let idx = (start + attempt) % n;
            match self.post_once(idx, body).await {
                Ok(v) => {
                    // Stick to successful endpoint.
                    self.inner.index.store(idx, Ordering::Relaxed);
                    return Ok(v);
                }
                Err(e) => {
                    let kind = classify_error(&e);
                    last_err = e;
                    if kind.should_rotate() && attempt + 1 < attempts {
                        self.advance(idx, kind);
                        // Brief backoff on 429.
                        if kind == RpcFailKind::RateLimited {
                            tokio::time::sleep(Duration::from_millis(150 * (attempt as u64 + 1)))
                                .await;
                        }
                        continue;
                    }
                    if !kind.should_rotate() {
                        return Err(last_err);
                    }
                }
            }
        }
        Err(last_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_429_and_5xx() {
        assert_eq!(
            classify_error(&BootstrapError::Http("HTTP 429: ...".into())),
            RpcFailKind::RateLimited
        );
        assert_eq!(
            classify_error(&BootstrapError::Http("HTTP 503: unavailable".into())),
            RpcFailKind::ServerError
        );
        assert_eq!(
            classify_error(&BootstrapError::Http("timeout waiting".into())),
            RpcFailKind::Timeout
        );
    }

    #[tokio::test]
    async fn rotates_on_429_then_succeeds() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls2 = calls.clone();
        let pool = RotatingRpcPool::mock_for_tests(3, move |idx, body| {
            calls2.lock().push(idx);
            let method = body.get("method").and_then(|m| m.as_str()).unwrap_or("");
            assert_eq!(method, "getSlot");
            if idx == 0 {
                Err(BootstrapError::Http("HTTP 429: rate limited".into()))
            } else if idx == 1 {
                Err(BootstrapError::Http("HTTP 503: overload".into()))
            } else {
                Ok(json!({"jsonrpc":"2.0","id":1,"result":42}))
            }
        });
        let resp = pool
            .post_json(&json!({"jsonrpc":"2.0","id":1,"method":"getSlot","params":[]}))
            .await
            .unwrap();
        assert_eq!(resp["result"], 42);
        assert_eq!(*calls.lock(), vec![0, 1, 2]);
        let stats = pool.stats_snapshot();
        assert_eq!(stats[0].errors, 1);
        assert_eq!(stats[0].rotations_away, 1);
        assert_eq!(stats[2].successes, 1);
        // Hosts must not contain raw key path secrets in reports — mock uses /key but host-only strips path.
        for s in &stats {
            assert!(!s.host.contains("/key"), "host leaked path: {}", s.host);
            assert!(s.host.starts_with("https://mock-"));
        }
    }

    #[tokio::test]
    async fn does_not_rotate_on_rpc_app_error() {
        let calls = Arc::new(Mutex::new(0u32));
        let c2 = calls.clone();
        let pool = RotatingRpcPool::mock_for_tests(2, move |_idx, _body| {
            *c2.lock() += 1;
            Err(BootstrapError::Rpc(r#"{"code":-32602,"message":"invalid"}"#.into()))
        });
        let err = pool
            .post_json(&json!({"method":"getSlot"}))
            .await
            .unwrap_err();
        assert!(matches!(err, BootstrapError::Rpc(_)));
        assert_eq!(*calls.lock(), 1);
    }

    #[test]
    fn from_urls_rejects_empty_and_placeholder() {
        assert!(RotatingRpcPool::from_urls(vec![]).is_err());
        assert!(RotatingRpcPool::from_urls(vec!["https://YOUR_PRIVATE_RPC".into()]).is_err());
    }
}
