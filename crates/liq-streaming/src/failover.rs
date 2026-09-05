//! Multi-provider freshness tracking and failover selection.

use crate::{GeyserSubscriber, StreamError, StreamEvent, SubscribeFilter};
use async_trait::async_trait;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{info, warn};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct ProviderFreshness {
    pub name: String,
    pub last_slot: u64,
    pub last_event_ms: u64,
    pub healthy: bool,
}

#[derive(Debug, Clone)]
pub struct FreshnessPolicy {
    /// Max slot lag behind the freshest known tip before a provider is stale.
    pub max_slot_lag: u64,
    /// Max wall-clock silence (ms) before a provider is stale.
    pub max_silence_ms: u64,
}

impl Default for FreshnessPolicy {
    fn default() -> Self {
        Self {
            max_slot_lag: 32,
            max_silence_ms: 2_000,
        }
    }
}

/// Tracks last-seen slot / wall time per named provider and picks a live primary.
pub struct FreshnessTracker {
    policy: FreshnessPolicy,
    providers: RwLock<Vec<ProviderFreshness>>,
    tip_slot: RwLock<u64>,
}

impl FreshnessTracker {
    pub fn new(names: Vec<String>, policy: FreshnessPolicy) -> Self {
        let providers = names
            .into_iter()
            .map(|name| ProviderFreshness {
                name,
                last_slot: 0,
                last_event_ms: 0,
                healthy: true,
            })
            .collect();
        Self {
            policy,
            providers: RwLock::new(providers),
            tip_slot: RwLock::new(0),
        }
    }

    pub fn record(&self, name: &str, slot: u64) {
        let ts = now_ms();
        {
            let mut tip = self.tip_slot.write();
            if slot > *tip {
                *tip = slot;
            }
        }
        let tip = *self.tip_slot.read();
        let mut providers = self.providers.write();
        if let Some(p) = providers.iter_mut().find(|p| p.name == name) {
            p.last_slot = slot;
            p.last_event_ms = ts;
            p.healthy = tip.saturating_sub(slot) <= self.policy.max_slot_lag;
        }
    }

    pub fn mark_disconnected(&self, name: &str) {
        let mut providers = self.providers.write();
        if let Some(p) = providers.iter_mut().find(|p| p.name == name) {
            p.healthy = false;
        }
    }

    /// Re-evaluate silence-based health using wall clock.
    pub fn refresh_silence(&self) {
        let now = now_ms();
        let mut providers = self.providers.write();
        for p in providers.iter_mut() {
            if p.last_event_ms == 0 {
                continue;
            }
            if now.saturating_sub(p.last_event_ms) > self.policy.max_silence_ms {
                p.healthy = false;
            }
        }
    }

    pub fn snapshot(&self) -> Vec<ProviderFreshness> {
        self.providers.read().clone()
    }

    /// First healthy provider in registration order; otherwise the freshest by slot.
    pub fn select_primary(&self) -> Option<String> {
        self.refresh_silence();
        let providers = self.providers.read();
        if let Some(p) = providers.iter().find(|p| p.healthy) {
            return Some(p.name.clone());
        }
        providers
            .iter()
            .max_by_key(|p| p.last_slot)
            .map(|p| p.name.clone())
    }

    pub fn is_stale(&self, name: &str) -> bool {
        self.refresh_silence();
        let providers = self.providers.read();
        providers
            .iter()
            .find(|p| p.name == name)
            .map(|p| !p.healthy)
            .unwrap_or(true)
    }
}

struct NamedSub {
    name: String,
    inner: Arc<dyn GeyserSubscriber>,
}

/// Fan-in multiple providers; prefer the primary selected by [`FreshnessTracker`].
/// On primary staleness, fail over to the next healthy provider.
pub struct FailoverMux {
    providers: Vec<NamedSub>,
    tracker: Arc<FreshnessTracker>,
}

impl FailoverMux {
    pub fn new(providers: Vec<(String, Arc<dyn GeyserSubscriber>)>, policy: FreshnessPolicy) -> Self {
        let names: Vec<String> = providers.iter().map(|(n, _)| n.clone()).collect();
        let tracker = Arc::new(FreshnessTracker::new(names, policy));
        let providers = providers
            .into_iter()
            .map(|(name, inner)| NamedSub { name, inner })
            .collect();
        Self { providers, tracker }
    }

    pub fn tracker(&self) -> Arc<FreshnessTracker> {
        self.tracker.clone()
    }

    /// Pick active provider name after applying freshness rules.
    pub fn active_provider(&self) -> Option<String> {
        self.tracker.select_primary()
    }
}

#[async_trait]
impl GeyserSubscriber for FailoverMux {
    fn name(&self) -> &str {
        "failover-mux"
    }

    async fn subscribe(
        &self,
        filter: SubscribeFilter,
    ) -> Result<mpsc::Receiver<StreamEvent>, StreamError> {
        let primary = self
            .tracker
            .select_primary()
            .ok_or_else(|| StreamError::Subscribe("no providers registered".into()))?;

        let chosen = self
            .providers
            .iter()
            .find(|p| p.name == primary)
            .or_else(|| self.providers.first())
            .ok_or_else(|| StreamError::Subscribe("empty provider list".into()))?;

        info!(provider = %chosen.name, "failover mux selected provider");
        if self.tracker.is_stale(&chosen.name) {
            warn!(provider = %chosen.name, "selected provider marked stale; streaming anyway as last resort");
        }

        let mut rx = chosen.inner.subscribe(filter).await.map_err(|e| {
            self.tracker.mark_disconnected(&chosen.name);
            e
        })?;

        let (tx_out, rx_out) = mpsc::channel(256);
        let tracker = self.tracker.clone();
        let name = chosen.name.clone();
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                let slot = match &ev {
                    StreamEvent::Slot(s) => s.slot,
                    StreamEvent::Account(a) => a.slot,
                    StreamEvent::Price { slot, .. } => *slot,
                };
                tracker.record(&name, slot);
                if tx_out.send(ev).await.is_err() {
                    break;
                }
            }
            tracker.mark_disconnected(&name);
        });
        Ok(rx_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockGeyser;
    use liq_core::{PriceFx, Pubkey};

    #[test]
    fn failover_selects_fresh_over_stale() {
        let tracker = FreshnessTracker::new(
            vec!["primary".into(), "backup".into()],
            FreshnessPolicy {
                max_slot_lag: 8,
                max_silence_ms: 60_000,
            },
        );
        tracker.record("backup", 100);
        tracker.record("primary", 80); // lag 20 > 8
        assert!(tracker.is_stale("primary"));
        assert!(!tracker.is_stale("backup"));
        assert_eq!(tracker.select_primary().as_deref(), Some("backup"));
    }

    #[test]
    fn silence_marks_stale() {
        let tracker = FreshnessTracker::new(
            vec!["solo".into()],
            FreshnessPolicy {
                max_slot_lag: 1000,
                max_silence_ms: 1,
            },
        );
        tracker.record("solo", 1);
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(tracker.is_stale("solo"));
    }

    #[tokio::test]
    async fn mux_drains_selected_provider() {
        let asset = Pubkey::test(9, 9);
        let primary = Arc::new(MockGeyser::named(
            "primary",
            vec![StreamEvent::Price {
                asset,
                price_fx: PriceFx::from_f64(1.0).0,
                slot: 10,
                write_version: 1,
            }],
        )) as Arc<dyn GeyserSubscriber>;
        let backup = Arc::new(MockGeyser::named(
            "backup",
            vec![StreamEvent::Price {
                asset,
                price_fx: PriceFx::from_f64(2.0).0,
                slot: 50,
                write_version: 1,
            }],
        )) as Arc<dyn GeyserSubscriber>;

        let mux = FailoverMux::new(
            vec![
                ("primary".into(), primary),
                ("backup".into(), backup),
            ],
            FreshnessPolicy::default(),
        );
        // No records yet — first registered (primary) wins.
        let mut rx = mux.subscribe(SubscribeFilter::default()).await.unwrap();
        let ev = rx.recv().await.unwrap();
        match ev {
            StreamEvent::Price { price_fx, .. } => {
                assert_eq!(price_fx, PriceFx::from_f64(1.0).0);
            }
            _ => panic!("expected price"),
        }
    }

    #[tokio::test]
    async fn mux_fails_over_when_primary_stale() {
        let asset = Pubkey::test(9, 8);
        let primary = Arc::new(MockGeyser::named("primary", vec![])) as Arc<dyn GeyserSubscriber>;
        let backup = Arc::new(MockGeyser::named(
            "backup",
            vec![StreamEvent::Price {
                asset,
                price_fx: PriceFx::from_f64(9.0).0,
                slot: 200,
                write_version: 1,
            }],
        )) as Arc<dyn GeyserSubscriber>;

        let mux = FailoverMux::new(
            vec![
                ("primary".into(), primary),
                ("backup".into(), backup),
            ],
            FreshnessPolicy {
                max_slot_lag: 5,
                max_silence_ms: 60_000,
            },
        );
        mux.tracker().record("backup", 200);
        mux.tracker().record("primary", 10);
        assert_eq!(mux.active_provider().as_deref(), Some("backup"));

        let mut rx = mux.subscribe(SubscribeFilter::default()).await.unwrap();
        let ev = rx.recv().await.unwrap();
        match ev {
            StreamEvent::Price { price_fx, slot, .. } => {
                assert_eq!(price_fx, PriceFx::from_f64(9.0).0);
                assert_eq!(slot, 200);
            }
            _ => panic!("expected backup price"),
        }
    }
}
