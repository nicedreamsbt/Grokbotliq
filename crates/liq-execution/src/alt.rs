//! Address Lookup Table manager skeleton (no network).

use liq_core::Pubkey;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AltEntry {
    pub table: Pubkey,
    pub addresses: Vec<Pubkey>,
}

/// In-memory ALT registry for precomputing HOT account lookups.
#[derive(Default)]
pub struct AltManager {
    by_table: RwLock<HashMap<Pubkey, AltEntry>>,
}

impl AltManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, entry: AltEntry) {
        self.by_table.write().insert(entry.table, entry);
    }

    pub fn get(&self, table: &Pubkey) -> Option<AltEntry> {
        self.by_table.read().get(table).cloned()
    }

    pub fn resolve_index(&self, table: &Pubkey, index: u8) -> Option<Pubkey> {
        self.get(table)?
            .addresses
            .get(index as usize)
            .copied()
    }

    pub fn len(&self) -> usize {
        self.by_table.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_by_index() {
        let m = AltManager::new();
        let table = Pubkey::test(1, 1);
        m.upsert(AltEntry {
            table,
            addresses: vec![Pubkey::test(2, 1), Pubkey::test(2, 2)],
        });
        assert_eq!(m.resolve_index(&table, 1), Some(Pubkey::test(2, 2)));
        assert!(m.resolve_index(&table, 9).is_none());
    }
}
