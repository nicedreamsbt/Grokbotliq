//! Yellowstone gRPC integration points.
//!
//! Compiles without live gRPC credentials or the `yellowstone-grpc-client` crate.
//! Wire a real client behind these types when `GEYSER_ENDPOINT` + auth token are available.

use crate::{GeyserSubscriber, StreamError, StreamEvent, SubscribeFilter};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::info;

/// Env / config knobs expected for a live Yellowstone connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YellowstoneConfig {
    /// e.g. `https://grpc.example.com:443`
    pub endpoint: String,
    /// `x-token` (or provider-specific) auth header value.
    pub x_token: Option<String>,
    /// Commitment for account/slot subscriptions.
    #[serde(default = "default_commitment")]
    pub commitment: String,
    /// Optional ping interval to keep the HTTP/2 stream alive.
    #[serde(default = "default_ping_ms")]
    pub ping_interval_ms: u64,
}

fn default_commitment() -> String {
    "processed".into()
}

fn default_ping_ms() -> u64 {
    15_000
}

impl YellowstoneConfig {
    /// Build from environment without requiring credentials to be present.
    /// Missing endpoint yields `None` (safe for dry-run / shadow).
    pub fn from_env() -> Option<Self> {
        let endpoint = std::env::var("GEYSER_ENDPOINT").ok()?;
        if endpoint.is_empty() || endpoint.contains("YOUR_") {
            return None;
        }
        Some(Self {
            endpoint,
            x_token: std::env::var("GEYSER_X_TOKEN").ok(),
            commitment: std::env::var("GEYSER_COMMITMENT").unwrap_or_else(|_| default_commitment()),
            ping_interval_ms: std::env::var("GEYSER_PING_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(default_ping_ms),
        })
    }

    pub fn has_credentials(&self) -> bool {
        self.x_token
            .as_ref()
            .map(|t| !t.is_empty() && !t.contains("YOUR_"))
            .unwrap_or(false)
    }
}

/// Documented integration surface for a future Yellowstone client.
///
/// Integration checklist (no live deps required to compile):
/// 1. Add optional dep `yellowstone-grpc-client` + proto crates behind a feature flag.
/// 2. Map [`SubscribeFilter`] → `SubscribeRequest` (accounts / owners / slots).
/// 3. Translate `SubscribeUpdate` → [`StreamEvent`] (Account / Slot / decoded Price).
/// 4. Feed slots into `FreshnessTracker` for multi-provider failover.
/// 5. On `Status::Unavailable` / auth errors, mark provider disconnected and fail over.
#[derive(Debug, Clone)]
pub struct YellowstoneSubscriber {
    pub config: YellowstoneConfig,
}

impl YellowstoneSubscriber {
    pub fn new(config: YellowstoneConfig) -> Self {
        Self { config }
    }

    /// Would open a gRPC channel; currently returns a descriptive error so
    /// binaries can compile and tests can assert the wiring point exists.
    pub fn connect_placeholder(&self) -> Result<(), StreamError> {
        if !self.config.has_credentials() {
            return Err(StreamError::Subscribe(
                "Yellowstone: GEYSER_X_TOKEN missing — stub only (no live gRPC)".into(),
            ));
        }
        Err(StreamError::Subscribe(format!(
            "Yellowstone client not linked (endpoint={}); enable yellowstone feature + creds",
            self.config.endpoint
        )))
    }
}

#[async_trait]
impl GeyserSubscriber for YellowstoneSubscriber {
    fn name(&self) -> &str {
        "yellowstone"
    }

    async fn subscribe(
        &self,
        _filter: SubscribeFilter,
    ) -> Result<mpsc::Receiver<StreamEvent>, StreamError> {
        info!(
            endpoint = %self.config.endpoint,
            has_token = self.config.has_credentials(),
            commitment = %self.config.commitment,
            "Yellowstone subscribe requested (stub)"
        );
        self.connect_placeholder()?;
        unreachable!("connect_placeholder always errors until client is wired");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_env_ignores_placeholders() {
        std::env::set_var("GEYSER_ENDPOINT", "https://YOUR_GEYSER_GRPC");
        assert!(YellowstoneConfig::from_env().is_none());
        std::env::set_var("GEYSER_ENDPOINT", "https://grpc.example.test:443");
        std::env::remove_var("GEYSER_X_TOKEN");
        let cfg = YellowstoneConfig::from_env().unwrap();
        assert!(!cfg.has_credentials());
        std::env::remove_var("GEYSER_ENDPOINT");
    }

    #[tokio::test]
    async fn stub_subscribe_errors_without_live_client() {
        let sub = YellowstoneSubscriber::new(YellowstoneConfig {
            endpoint: "https://grpc.example.test:443".into(),
            x_token: Some("test-token".into()),
            commitment: "processed".into(),
            ping_interval_ms: 1000,
        });
        let err = sub.subscribe(SubscribeFilter::default()).await.unwrap_err();
        match err {
            StreamError::Subscribe(msg) => assert!(msg.contains("not linked")),
            other => panic!("unexpected {other:?}"),
        }
    }
}
