use crate::types::{Pubkey, UpdateSource};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Stored account/oracle entry with slot + write_version concurrency control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAccount<T> {
    pub slot: u64,
    pub write_version: u64,
    pub pubkey: Pubkey,
    pub decoded: T,
    pub source: UpdateSource,
    /// Receive timestamp in microseconds since UNIX epoch.
    pub recv_ts_us: u64,
}

impl<T> StoredAccount<T> {
    pub fn now_ts_us() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }

    pub fn new(
        slot: u64,
        write_version: u64,
        pubkey: Pubkey,
        decoded: T,
        source: UpdateSource,
    ) -> Self {
        Self {
            slot,
            write_version,
            pubkey,
            decoded,
            source,
            recv_ts_us: Self::now_ts_us(),
        }
    }

    /// True if `other` is strictly newer by (slot, write_version).
    pub fn is_stale_vs(&self, other_slot: u64, other_wv: u64) -> bool {
        (other_slot, other_wv) > (self.slot, self.write_version)
    }
}

/// In-memory state store keyed by pubkey.
#[derive(Default)]
pub struct StateStore<T: Clone> {
    inner: RwLock<HashMap<Pubkey, StoredAccount<T>>>,
}

impl<T: Clone> StateStore<T> {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &Pubkey) -> Option<StoredAccount<T>> {
        self.inner.read().get(key).cloned()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }

    /// Insert if missing or if incoming (slot, write_version) is newer.
    /// Returns true if the store was updated.
    pub fn upsert(&self, entry: StoredAccount<T>) -> bool {
        let mut map = self.inner.write();
        match map.get(&entry.pubkey) {
            Some(existing)
                if !existing.is_stale_vs(entry.slot, entry.write_version) =>
            {
                false
            }
            _ => {
                map.insert(entry.pubkey, entry);
                true
            }
        }
    }

    pub fn remove(&self, key: &Pubkey) -> Option<StoredAccount<T>> {
        self.inner.write().remove(key)
    }

    pub fn keys(&self) -> Vec<Pubkey> {
        self.inner.read().keys().copied().collect()
    }

    pub fn retain<F: FnMut(&Pubkey, &StoredAccount<T>) -> bool>(&self, mut f: F) {
        self.inner.write().retain(|k, v| f(k, v));
    }
}

pub type SharedStateStore<T> = Arc<StateStore<T>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_respects_slot_write_version() {
        let store = StateStore::new();
        let pk = Pubkey::test(1, 1);
        assert!(store.upsert(StoredAccount::new(10, 1, pk, 100u32, UpdateSource::Mock)));
        assert!(!store.upsert(StoredAccount::new(10, 1, pk, 200u32, UpdateSource::Mock)));
        assert!(!store.upsert(StoredAccount::new(9, 99, pk, 300u32, UpdateSource::Mock)));
        assert!(store.upsert(StoredAccount::new(10, 2, pk, 400u32, UpdateSource::Mock)));
        assert_eq!(store.get(&pk).unwrap().decoded, 400);
        assert!(store.upsert(StoredAccount::new(11, 0, pk, 500u32, UpdateSource::Geyser)));
        assert_eq!(store.get(&pk).unwrap().decoded, 500);
        assert_eq!(store.get(&pk).unwrap().source, UpdateSource::Geyser);
    }
}
