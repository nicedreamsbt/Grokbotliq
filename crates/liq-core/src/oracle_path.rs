//! Oracle update -> trigger crossing -> candidate wake path.

use crate::candidate_index::{CandidateIndex, TriggerHit};
use crate::state_store::{StateStore, StoredAccount};
use crate::types::{PriceFx, Pubkey, UpdateSource};
use liq_telemetry::metrics::Metrics;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct OraclePriceBook {
    prices: Arc<RwLock<HashMap<Pubkey, PriceFx>>>,
}

impl OraclePriceBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, asset: &Pubkey) -> Option<PriceFx> {
        self.prices.read().get(asset).copied()
    }

    pub fn set(&self, asset: Pubkey, price: PriceFx) {
        self.prices.write().insert(asset, price);
    }
}

pub struct OracleTriggerPath {
    pub prices: OraclePriceBook,
    pub index: Arc<CandidateIndex>,
    pub oracle_store: Arc<StateStore<PriceFx>>,
    pub metrics: Arc<Metrics>,
}

impl OracleTriggerPath {
    pub fn new(index: Arc<CandidateIndex>, metrics: Arc<Metrics>) -> Self {
        Self {
            prices: OraclePriceBook::new(),
            index,
            oracle_store: Arc::new(StateStore::new()),
            metrics,
        }
    }

    /// Apply an oracle price update; returns newly crossed trigger hits.
    pub fn apply_oracle_update(
        &self,
        asset: Pubkey,
        new_price: PriceFx,
        slot: u64,
        write_version: u64,
        source: UpdateSource,
    ) -> Vec<TriggerHit> {
        let prev = self.prices.get(&asset).unwrap_or(new_price);
        let updated = self.oracle_store.upsert(StoredAccount::new(
            slot,
            write_version,
            asset,
            new_price,
            source,
        ));
        if !updated && self.prices.get(&asset) == Some(new_price) {
            return Vec::new();
        }
        self.prices.set(asset, new_price);
        self.metrics.oracle_updates.inc();
        let hits = self.index.on_price_update(asset, prev, new_price);
        self.metrics.trigger_hits.add(hits.len() as u64);
        for _ in &hits {
            self.metrics.candidates_woken.inc();
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate_index::{PriceTrigger, TriggerSide};
    use crate::types::Protocol;
    use crate::candidate_index::CandidateMeta;
    use crate::types::{CandidateBand, HealthFx};

    #[test]
    fn oracle_update_wakes_crossed_candidates() {
        let index = Arc::new(CandidateIndex::new());
        let metrics = Arc::new(Metrics::new());
        let path = OracleTriggerPath::new(index.clone(), metrics);

        let acct = Pubkey::test(1, 42);
        let sol = Pubkey::test(2, 1);
        path.prices.set(sol, PriceFx::from_f64(120.0));
        index.upsert_candidate(CandidateMeta {
            account: acct,
            protocol: Protocol::Save,
            band: CandidateBand::Hot,
            health: HealthFx::from_f64(1.03),
            assets: vec![sol],
        });
        index.set_triggers(
            acct,
            vec![PriceTrigger {
                account: acct,
                asset: sol,
                side: TriggerSide::CollateralDown,
                trigger_price: PriceFx::from_f64(100.0),
            }],
        );

        let hits = path.apply_oracle_update(
            sol,
            PriceFx::from_f64(95.0),
            100,
            1,
            UpdateSource::Mock,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(path.oracle_store.get(&sol).unwrap().decoded, PriceFx::from_f64(95.0));
    }
}
