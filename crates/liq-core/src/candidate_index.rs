use crate::types::{CandidateBand, HealthFx, PriceFx, Protocol, Pubkey};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerSide {
    /// Unhealthy when collateral price falls to <= trigger.
    CollateralDown,
    /// Unhealthy when debt price rises to >= trigger.
    DebtUp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateMeta {
    pub account: Pubkey,
    pub protocol: Protocol,
    pub band: CandidateBand,
    pub health: HealthFx,
    pub assets: Vec<Pubkey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceTrigger {
    pub account: Pubkey,
    pub asset: Pubkey,
    pub side: TriggerSide,
    pub trigger_price: PriceFx,
}

#[derive(Debug, Default)]
struct AssetIndex {
    collateral_down: BTreeMap<PriceFx, HashSet<Pubkey>>,
    debt_up: BTreeMap<PriceFx, HashSet<Pubkey>>,
}

#[derive(Default)]
pub struct CandidateIndex {
    by_account: RwLock<HashMap<Pubkey, CandidateMeta>>,
    by_band: RwLock<HashMap<CandidateBand, HashSet<Pubkey>>>,
    by_asset: RwLock<HashMap<Pubkey, AssetIndex>>,
    account_triggers: RwLock<HashMap<Pubkey, Vec<PriceTrigger>>>,
}

#[derive(Debug, Clone)]
pub struct TriggerHit {
    pub account: Pubkey,
    pub asset: Pubkey,
    pub side: TriggerSide,
    pub trigger_price: PriceFx,
    pub new_price: PriceFx,
}

impl CandidateIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_candidate(&self, meta: CandidateMeta) {
        let account = meta.account;
        let band = meta.band;
        let mut by_account = self.by_account.write();
        let mut by_band = self.by_band.write();
        if let Some(prev) = by_account.insert(account, meta) {
            if let Some(set) = by_band.get_mut(&prev.band) {
                set.remove(&account);
            }
        }
        by_band.entry(band).or_default().insert(account);
    }

    pub fn remove_candidate(&self, account: &Pubkey) {
        self.clear_triggers(account);
        let mut by_account = self.by_account.write();
        let mut by_band = self.by_band.write();
        if let Some(prev) = by_account.remove(account) {
            if let Some(set) = by_band.get_mut(&prev.band) {
                set.remove(account);
            }
        }
    }

    pub fn get(&self, account: &Pubkey) -> Option<CandidateMeta> {
        self.by_account.read().get(account).cloned()
    }

    pub fn accounts_in_band(&self, band: CandidateBand) -> Vec<Pubkey> {
        self.by_band
            .read()
            .get(&band)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }

    pub fn clear_triggers(&self, account: &Pubkey) {
        let mut account_triggers = self.account_triggers.write();
        let Some(old) = account_triggers.remove(account) else {
            return;
        };
        let mut by_asset = self.by_asset.write();
        for t in old {
            if let Some(idx) = by_asset.get_mut(&t.asset) {
                let map = match t.side {
                    TriggerSide::CollateralDown => &mut idx.collateral_down,
                    TriggerSide::DebtUp => &mut idx.debt_up,
                };
                if let Some(set) = map.get_mut(&t.trigger_price) {
                    set.remove(account);
                    if set.is_empty() {
                        map.remove(&t.trigger_price);
                    }
                }
            }
        }
    }

    pub fn set_triggers(&self, account: Pubkey, triggers: Vec<PriceTrigger>) {
        self.clear_triggers(&account);
        let mut by_asset = self.by_asset.write();
        let mut account_triggers = self.account_triggers.write();
        for t in &triggers {
            let idx = by_asset.entry(t.asset).or_default();
            let map = match t.side {
                TriggerSide::CollateralDown => &mut idx.collateral_down,
                TriggerSide::DebtUp => &mut idx.debt_up,
            };
            map.entry(t.trigger_price).or_default().insert(account);
        }
        account_triggers.insert(account, triggers);
    }

    /// Find accounts whose liquidation trigger was newly crossed by a price move.
    pub fn on_price_update(
        &self,
        asset: Pubkey,
        prev_price: PriceFx,
        new_price: PriceFx,
    ) -> Vec<TriggerHit> {
        let by_asset = self.by_asset.read();
        let Some(idx) = by_asset.get(&asset) else {
            return Vec::new();
        };
        let mut hits = Vec::new();

        if new_price < prev_price {
            // Collateral price fell: fire triggers where prev > trig >= new
            // (was healthy above trigger, now at or below).
            for (trig_px, accounts) in idx.collateral_down.range(new_price..=prev_price) {
                if prev_price > *trig_px && new_price <= *trig_px {
                    for account in accounts {
                        hits.push(TriggerHit {
                            account: *account,
                            asset,
                            side: TriggerSide::CollateralDown,
                            trigger_price: *trig_px,
                            new_price,
                        });
                    }
                }
            }
        } else if new_price > prev_price {
            // Debt price rose: fire where prev < trig <= new
            for (trig_px, accounts) in idx.debt_up.range(prev_price..=new_price) {
                if prev_price < *trig_px && *trig_px <= new_price {
                    for account in accounts {
                        hits.push(TriggerHit {
                            account: *account,
                            asset,
                            side: TriggerSide::DebtUp,
                            trigger_price: *trig_px,
                            new_price,
                        });
                    }
                }
            }
        }

        hits
    }

    pub fn trigger_count(&self) -> usize {
        self.account_triggers.read().values().map(|v| v.len()).sum()
    }

    pub fn candidate_count(&self) -> usize {
        self.by_account.read().len()
    }
}

pub fn classify_band(health: HealthFx, distance_pct: f64) -> CandidateBand {
    if health.is_liquidatable() {
        return CandidateBand::Critical;
    }
    if distance_pct <= 0.02 {
        CandidateBand::Hot
    } else if distance_pct <= 0.08 {
        CandidateBand::Warm
    } else {
        CandidateBand::Cold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collateral_down_trigger_crossing() {
        let idx = CandidateIndex::new();
        let acct = Pubkey::test(7, 1);
        let sol = Pubkey::test(9, 1);
        idx.upsert_candidate(CandidateMeta {
            account: acct,
            protocol: Protocol::Kamino,
            band: CandidateBand::Hot,
            health: HealthFx::from_f64(1.05),
            assets: vec![sol],
        });
        idx.set_triggers(
            acct,
            vec![PriceTrigger {
                account: acct,
                asset: sol,
                side: TriggerSide::CollateralDown,
                trigger_price: PriceFx::from_f64(100.0),
            }],
        );

        let hits = idx.on_price_update(sol, PriceFx::from_f64(110.0), PriceFx::from_f64(99.0));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].account, acct);

        let hits2 = idx.on_price_update(sol, PriceFx::from_f64(99.0), PriceFx::from_f64(90.0));
        assert!(hits2.is_empty());

        let hits3 = idx.on_price_update(sol, PriceFx::from_f64(110.0), PriceFx::from_f64(105.0));
        assert!(hits3.is_empty());
    }

    #[test]
    fn debt_up_trigger_crossing() {
        let idx = CandidateIndex::new();
        let acct = Pubkey::test(7, 2);
        let usdc = Pubkey::test(9, 2);
        idx.set_triggers(
            acct,
            vec![PriceTrigger {
                account: acct,
                asset: usdc,
                side: TriggerSide::DebtUp,
                trigger_price: PriceFx::from_f64(1.02),
            }],
        );
        let hits = idx.on_price_update(usdc, PriceFx::from_f64(1.00), PriceFx::from_f64(1.03));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].side, TriggerSide::DebtUp);

        let hits2 = idx.on_price_update(usdc, PriceFx::from_f64(1.03), PriceFx::from_f64(1.05));
        assert!(hits2.is_empty());
    }

    #[test]
    fn band_classification() {
        assert_eq!(
            classify_band(HealthFx::from_f64(0.95), 0.0),
            CandidateBand::Critical
        );
        assert_eq!(
            classify_band(HealthFx::from_f64(1.01), 0.01),
            CandidateBand::Hot
        );
        assert_eq!(
            classify_band(HealthFx::from_f64(1.05), 0.05),
            CandidateBand::Warm
        );
        assert_eq!(
            classify_band(HealthFx::from_f64(1.20), 0.20),
            CandidateBand::Cold
        );
    }
}
