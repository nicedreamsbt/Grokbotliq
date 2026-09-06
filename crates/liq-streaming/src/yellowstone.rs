//! Yellowstone gRPC integration points.
//!
//! Real yellowstone-grpc client when feature is enabled + credentials present,
//! otherwise falls back to stub for DRY_RUN / fixture tests.

use crate::{redact::RedactedUrl, GeyserSubscriber, StreamError, StreamEvent, SubscribeFilter};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{error, info};

#[cfg(feature = "yellowstone")]
use {
    futures::{SinkExt, StreamExt},
    std::collections::HashMap,
    yellowstone_grpc_client::GeyserGrpcClient,
    yellowstone_grpc_proto::geyser::{
        subscribe_request_filter_accounts_filter::Filter as AccountsFilterOneof,
        subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest,
        SubscribeRequestFilterAccounts, SubscribeRequestFilterAccountsFilter,
    },
};

/// Env / config knobs expected for a live Yellowstone connection.
///
/// ## Alchemy Example
/// ```bash
/// export GEYSER_ENDPOINT="https://solana-mainnet.streaming.alchemy.com"
/// export GEYSER_X_TOKEN="your-alchemy-api-key"
/// export GEYSER_COMMITMENT="processed"  # optional, defaults to "processed"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YellowstoneConfig {
    /// gRPC endpoint (e.g., `https://solana-mainnet.streaming.alchemy.com`)
    pub endpoint: String,
    /// `x-token` auth header (Alchemy API key or provider-specific token). NEVER logged.
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

/// Real Yellowstone gRPC subscriber (when feature enabled + credentials present).
///
/// ## Integration Path
/// 1. Maps [`SubscribeFilter`] → `SubscribeRequest` (accounts / owners / slots + datasize).
/// 2. Translates `SubscribeUpdate` → [`StreamEvent`] (Account / Slot).
/// 3. Returns `mpsc::Receiver<StreamEvent>` consumed by liquidator ingest loop.
/// 4. **TODO(StreamDetectionPath)**: When detection.rs / HotPathCache seam merges,
///    wire receiver into `StreamDetectionPath::ingest()` instead of direct loop.
///
/// ## Klend Filters
/// - Owner: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
/// - Obligations datasize: 3344 bytes
/// - Reserves datasize: 8624 bytes
#[derive(Debug, Clone)]
pub struct YellowstoneSubscriber {
    pub config: YellowstoneConfig,
}

impl YellowstoneSubscriber {
    pub fn new(config: YellowstoneConfig) -> Self {
        Self { config }
    }
}

#[cfg(feature = "yellowstone")]
#[async_trait]
impl GeyserSubscriber for YellowstoneSubscriber {
    fn name(&self) -> &str {
        "yellowstone"
    }

    async fn subscribe(
        &self,
        filter: SubscribeFilter,
    ) -> Result<mpsc::Receiver<StreamEvent>, StreamError> {
        if !self.config.has_credentials() {
            return Err(StreamError::Subscribe(
                "Yellowstone: GEYSER_X_TOKEN missing — cannot connect".into(),
            ));
        }

        info!(
            endpoint = %RedactedUrl(&self.config.endpoint),
            has_token = true,
            commitment = %self.config.commitment,
            owners_count = filter.owners.len(),
            accounts_count = filter.accounts.len(),
            "Yellowstone subscribe starting"
        );

        // Build gRPC client with TLS + x-token auth
        let mut client = GeyserGrpcClient::build_from_shared(self.config.endpoint.clone())
            .map_err(|e| StreamError::Subscribe(format!("gRPC client build failed: {e}")))?
            .x_token(self.config.x_token.clone())
            .map_err(|e| StreamError::Subscribe(format!("x-token config failed: {e}")))?
            .connect()
            .await
            .map_err(|e| StreamError::Subscribe(format!("gRPC connect failed: {e}")))?;

        info!("Yellowstone gRPC client connected");

        // Build subscribe request with Klend filters (owners + datasize pins)
        let mut accounts_filters = HashMap::new();

        // Klend obligations: datasize=3344
        if !filter.owners.is_empty() {
            let klend_obligations = SubscribeRequestFilterAccounts {
                owner: filter
                    .owners
                    .iter()
                    .map(|pk| pk.to_string())
                    .collect::<Vec<_>>(),
                filters: vec![SubscribeRequestFilterAccountsFilter {
                    filter: Some(AccountsFilterOneof::Datasize(3344)), // KLEND_OBLIGATION_DATASIZE
                }],
                ..Default::default()
            };
            accounts_filters.insert("klend_obligations".to_string(), klend_obligations);

            // Klend reserves: datasize=8624
            let klend_reserves = SubscribeRequestFilterAccounts {
                owner: filter
                    .owners
                    .iter()
                    .map(|pk| pk.to_string())
                    .collect::<Vec<_>>(),
                filters: vec![SubscribeRequestFilterAccountsFilter {
                    filter: Some(AccountsFilterOneof::Datasize(8624)), // KLEND_RESERVE_DATASIZE
                }],
                ..Default::default()
            };
            accounts_filters.insert("klend_reserves".to_string(), klend_reserves);
        }

        // Specific accounts (no owner/datasize filter)
        if !filter.accounts.is_empty() {
            let specific_accounts = SubscribeRequestFilterAccounts {
                account: filter
                    .accounts
                    .iter()
                    .map(|pk| pk.to_string())
                    .collect::<Vec<_>>(),
                ..Default::default()
            };
            accounts_filters.insert("specific_accounts".to_string(), specific_accounts);
        }

        let commitment = match self.config.commitment.as_str() {
            "finalized" => CommitmentLevel::Finalized,
            "confirmed" => CommitmentLevel::Confirmed,
            _ => CommitmentLevel::Processed,
        };

        let request = SubscribeRequest {
            accounts: accounts_filters,
            slots: HashMap::from([("slots".to_string(), Default::default())]),
            commitment: Some(commitment as i32),
            ..Default::default()
        };

        // Open bidirectional stream
        let (mut subscribe_tx, mut stream) = client
            .subscribe()
            .await
            .map_err(|e| StreamError::Subscribe(format!("subscribe stream failed: {e}")))?;

        // Send initial subscribe request
        subscribe_tx
            .send(request)
            .await
            .map_err(|e| StreamError::Subscribe(format!("failed to send subscribe request: {e}")))?;

        info!("Yellowstone subscribe request sent, listening for updates");

        // Spawn task to translate yellowstone updates → StreamEvent
        let (event_tx, event_rx) = mpsc::channel(1024);
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(update) => {
                        if let Err(e) = handle_update(update, &event_tx).await {
                            error!("Failed to handle yellowstone update: {e}");
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Yellowstone stream error: {e}");
                        break;
                    }
                }
            }
            info!("Yellowstone stream closed");
        });

        Ok(event_rx)
    }
}

#[cfg(feature = "yellowstone")]
async fn handle_update(
    update: yellowstone_grpc_proto::geyser::SubscribeUpdate,
    tx: &mpsc::Sender<StreamEvent>,
) -> Result<(), StreamError> {
    use liq_core::{Pubkey, UpdateSource};

    match update.update_oneof {
        Some(UpdateOneof::Account(account_update)) => {
            if let Some(account_info) = account_update.account {
                // Parse pubkey (32 bytes expected)
                let pubkey_bytes: [u8; 32] = account_info
                    .pubkey
                    .try_into()
                    .map_err(|_| StreamError::Subscribe("invalid pubkey length".into()))?;
                let pubkey = Pubkey::new(pubkey_bytes);

                let owner_bytes: [u8; 32] = account_info
                    .owner
                    .try_into()
                    .map_err(|_| StreamError::Subscribe("invalid owner length".into()))?;
                let owner = Pubkey::new(owner_bytes);

                let event = StreamEvent::Account(crate::AccountUpdate {
                    pubkey,
                    slot: account_update.slot,
                    write_version: account_info.write_version,
                    data: account_info.data,
                    owner,
                    source: UpdateSource::Geyser,
                });

                tx.send(event)
                    .await
                    .map_err(|_| StreamError::ChannelClosed)?;
            }
        }
        Some(UpdateOneof::Slot(slot_update)) => {
            // SlotStatus enum: Processed=0, Confirmed=1, Rooted=2
            let event = StreamEvent::Slot(crate::SlotUpdate {
                slot: slot_update.slot,
                parent: slot_update.parent,
                root: slot_update.status == 2, // Rooted
            });
            tx.send(event)
                .await
                .map_err(|_| StreamError::ChannelClosed)?;
        }
        _ => {
            // Ignore other update types (transactions, blocks, etc.)
        }
    }
    Ok(())
}

// Stub implementation when yellowstone feature is disabled (for DRY_RUN / tests)
#[cfg(not(feature = "yellowstone"))]
#[async_trait]
impl GeyserSubscriber for YellowstoneSubscriber {
    fn name(&self) -> &str {
        "yellowstone"
    }

    async fn subscribe(
        &self,
        _filter: SubscribeFilter,
    ) -> Result<mpsc::Receiver<StreamEvent>, StreamError> {
        warn!(
            "Yellowstone feature disabled — returning stub error (compile without live gRPC)"
        );
        Err(StreamError::Subscribe(
            "Yellowstone feature not enabled; rebuild with --features yellowstone".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_env_ignores_placeholders() {
        // Clean any existing env vars first
        std::env::remove_var("GEYSER_ENDPOINT");
        std::env::remove_var("GEYSER_X_TOKEN");
        std::env::remove_var("GEYSER_COMMITMENT");
        
        std::env::set_var("GEYSER_ENDPOINT", "https://YOUR_GEYSER_GRPC");
        assert!(YellowstoneConfig::from_env().is_none());
        std::env::set_var("GEYSER_ENDPOINT", "https://grpc.example.test:443");
        std::env::remove_var("GEYSER_X_TOKEN");
        let cfg = YellowstoneConfig::from_env().unwrap();
        assert!(!cfg.has_credentials());
        std::env::remove_var("GEYSER_ENDPOINT");
    }

    #[tokio::test]
    async fn subscribe_errors_without_token() {
        let sub = YellowstoneSubscriber::new(YellowstoneConfig {
            endpoint: "https://grpc.example.test:443".into(),
            x_token: None, // No token
            commitment: "processed".into(),
            ping_interval_ms: 1000,
        });
        let err = sub.subscribe(SubscribeFilter::default()).await.unwrap_err();
        match err {
            StreamError::Subscribe(msg) => assert!(msg.contains("missing")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn klend_datasize_constants() {
        // Verify constants match discovery.rs / kamino decode.rs
        assert_eq!(3344, crate::discovery::known::KLEND_OBLIGATION_DATASIZE);
        assert_eq!(8624, crate::discovery::known::KLEND_RESERVE_DATASIZE);
    }

    #[test]
    fn alchemy_endpoint_config() {
        std::env::set_var("GEYSER_ENDPOINT", "https://solana-mainnet.streaming.alchemy.com");
        std::env::set_var("GEYSER_X_TOKEN", "test-alchemy-key-123");
        std::env::set_var("GEYSER_COMMITMENT", "confirmed");
        
        let cfg = YellowstoneConfig::from_env().expect("config should load");
        assert_eq!(cfg.endpoint, "https://solana-mainnet.streaming.alchemy.com");
        assert_eq!(cfg.x_token, Some("test-alchemy-key-123".into()));
        assert_eq!(cfg.commitment, "confirmed");
        assert!(cfg.has_credentials());
        
        // Verify token is never in debug output via RedactedUrl
        use crate::redact::RedactedUrl;
        let redacted = format!("{}", RedactedUrl(&cfg.endpoint));
        assert_eq!(redacted, "https://solana-mainnet.streaming.alchemy.com");
        assert!(!redacted.contains("test-alchemy-key"));
        
        std::env::remove_var("GEYSER_ENDPOINT");
        std::env::remove_var("GEYSER_X_TOKEN");
        std::env::remove_var("GEYSER_COMMITMENT");
    }
}
