//! Scope oracle slot freshness checks.

use crate::{KaminoError, SCOPE_MAX_AGE_SLOTS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeFreshness {
    pub price_slot: u64,
    pub current_slot: u64,
    pub max_age_slots: u64,
}

impl ScopeFreshness {
    pub fn new(price_slot: u64, current_slot: u64) -> Self {
        Self {
            price_slot,
            current_slot,
            max_age_slots: SCOPE_MAX_AGE_SLOTS,
        }
    }

    pub fn age_slots(self) -> u64 {
        self.current_slot.saturating_sub(self.price_slot)
    }

    pub fn is_fresh(self) -> bool {
        self.age_slots() <= self.max_age_slots
    }

    pub fn check(self) -> Result<(), KaminoError> {
        if self.is_fresh() {
            Ok(())
        } else {
            Err(KaminoError::ScopeStale)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_within_512() {
        let f = ScopeFreshness::new(1000, 1000 + 512);
        assert!(f.is_fresh());
        f.check().unwrap();
    }

    #[test]
    fn stale_beyond_512() {
        let f = ScopeFreshness::new(1000, 1000 + 513);
        assert!(!f.is_fresh());
        assert_eq!(f.check(), Err(KaminoError::ScopeStale));
    }
}
