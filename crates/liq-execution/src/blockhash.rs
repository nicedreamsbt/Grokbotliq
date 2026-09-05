//! Recent blockhash cache (no network in tests).

use parking_lot::RwLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CachedBlockhash {
    pub hash: [u8; 32],
    pub slot: u64,
    pub fetched_at: Instant,
}

pub struct BlockhashCache {
    inner: RwLock<Option<CachedBlockhash>>,
    max_age: Duration,
}

impl BlockhashCache {
    pub fn new(max_age_secs: u64) -> Self {
        Self {
            inner: RwLock::new(None),
            max_age: Duration::from_secs(max_age_secs),
        }
    }

    pub fn put(&self, hash: [u8; 32], slot: u64) {
        *self.inner.write() = Some(CachedBlockhash {
            hash,
            slot,
            fetched_at: Instant::now(),
        });
    }

    pub fn get_fresh(&self) -> Option<CachedBlockhash> {
        let g = self.inner.read();
        let c = g.as_ref()?;
        if c.fetched_at.elapsed() > self.max_age {
            return None;
        }
        Some(c.clone())
    }

    pub fn is_fresh(&self) -> bool {
        self.get_fresh().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get() {
        let c = BlockhashCache::new(60);
        assert!(!c.is_fresh());
        c.put([7u8; 32], 42);
        let got = c.get_fresh().unwrap();
        assert_eq!(got.slot, 42);
        assert_eq!(got.hash[0], 7);
    }
}
