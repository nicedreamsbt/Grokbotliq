//! Transaction template / precompute cache for HOT accounts.

use liq_core::Pubkey;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxTemplate {
    pub account: Pubkey,
    pub protocol: String,
    pub ix_labels: Vec<String>,
    /// Pre-encoded instruction datas (refresh + liquidate skeleton).
    pub ix_datas: Vec<Vec<u8>>,
    pub account_metas: Vec<Pubkey>,
    pub updated_slot: u64,
}

#[derive(Default)]
pub struct TxTemplateCache {
    by_account: RwLock<HashMap<Pubkey, TxTemplate>>,
}

impl TxTemplateCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, tmpl: TxTemplate) {
        self.by_account.write().insert(tmpl.account, tmpl);
    }

    pub fn get(&self, account: &Pubkey) -> Option<TxTemplate> {
        self.by_account.read().get(account).cloned()
    }

    pub fn invalidate(&self, account: &Pubkey) {
        self.by_account.write().remove(account);
    }

    pub fn len(&self) -> usize {
        self.by_account.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip() {
        let c = TxTemplateCache::new();
        let acct = Pubkey::test(3, 1);
        c.put(TxTemplate {
            account: acct,
            protocol: "kamino".into(),
            ix_labels: vec!["refresh".into(), "liq".into()],
            ix_datas: vec![vec![1], vec![2]],
            account_metas: vec![Pubkey::test(3, 2)],
            updated_slot: 99,
        });
        assert_eq!(c.get(&acct).unwrap().updated_slot, 99);
        c.invalidate(&acct);
        assert!(c.get(&acct).is_none());
    }
}
