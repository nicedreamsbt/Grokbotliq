//! Geyser subscriber trait, mock provider, multi-provider freshness failover,
//! fixture loading, rotating RPC pool, mainnet discovery, RPC bootstrap,
//! and Yellowstone integration stubs (compile without live gRPC creds).

mod bootstrap;
mod discovery;
mod failover;
mod fixtures;
mod local_env;
mod redact;
mod rpc_pool;
mod yellowstone;

pub use bootstrap::*;
pub use discovery::*;
pub use failover::*;
pub use fixtures::*;
pub use local_env::*;
pub use redact::*;
pub use rpc_pool::*;
pub use yellowstone::*;

use async_trait::async_trait;
use liq_core::{Pubkey, UpdateSource};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUpdate {
    pub pubkey: Pubkey,
    pub slot: u64,
    pub write_version: u64,
    pub data: Vec<u8>,
    pub owner: Pubkey,
    pub source: UpdateSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotUpdate {
    pub slot: u64,
    pub parent: Option<u64>,
    pub root: bool,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Account(AccountUpdate),
    Slot(SlotUpdate),
    /// Oracle-ish account shortcut used by mock / fixture tests.
    Price {
        asset: Pubkey,
        price_fx: u128,
        slot: u64,
        write_version: u64,
    },
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("disconnected: {0}")]
    Disconnected(String),
    #[error("subscribe failed: {0}")]
    Subscribe(String),
    #[error("channel closed")]
    ChannelClosed,
    #[error("stale: {0}")]
    Stale(String),
    #[error("fixture: {0}")]
    Fixture(String),
}

#[derive(Debug, Clone, Default)]
pub struct SubscribeFilter {
    pub owners: Vec<Pubkey>,
    pub accounts: Vec<Pubkey>,
}

#[async_trait]
pub trait GeyserSubscriber: Send + Sync {
    fn name(&self) -> &str {
        "unnamed"
    }

    async fn subscribe(
        &self,
        filter: SubscribeFilter,
    ) -> Result<mpsc::Receiver<StreamEvent>, StreamError>;
}

/// In-memory mock that replays a preloaded event sequence.
pub struct MockGeyser {
    name: String,
    events: Vec<StreamEvent>,
}

impl MockGeyser {
    pub fn new(events: Vec<StreamEvent>) -> Self {
        Self {
            name: "mock".into(),
            events,
        }
    }

    pub fn named(name: impl Into<String>, events: Vec<StreamEvent>) -> Self {
        Self {
            name: name.into(),
            events,
        }
    }
}

#[async_trait]
impl GeyserSubscriber for MockGeyser {
    fn name(&self) -> &str {
        &self.name
    }

    async fn subscribe(
        &self,
        _filter: SubscribeFilter,
    ) -> Result<mpsc::Receiver<StreamEvent>, StreamError> {
        let (tx, rx) = mpsc::channel(self.events.len().max(1));
        for ev in &self.events {
            tx.send(ev.clone())
                .await
                .map_err(|_| StreamError::ChannelClosed)?;
        }
        Ok(rx)
    }
}

/// Helper: drain mock stream into a vec (tests / replay).
pub async fn drain_all(sub: Arc<dyn GeyserSubscriber>) -> Result<Vec<StreamEvent>, StreamError> {
    let mut rx = sub.subscribe(SubscribeFilter::default()).await?;
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        out.push(ev);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use liq_core::UpdateSource;

    #[tokio::test]
    async fn mock_replays_events() {
        let pk = Pubkey::test(3, 1);
        let mock = Arc::new(MockGeyser::new(vec![
            StreamEvent::Slot(SlotUpdate {
                slot: 10,
                parent: Some(9),
                root: false,
            }),
            StreamEvent::Account(AccountUpdate {
                pubkey: pk,
                slot: 10,
                write_version: 1,
                data: b"abc".to_vec(),
                owner: Pubkey::test(4, 1),
                source: UpdateSource::Mock,
            }),
        ]));
        let events = drain_all(mock).await.unwrap();
        assert_eq!(events.len(), 2);
    }
}
