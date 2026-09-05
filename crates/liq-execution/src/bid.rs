//! Priority fee + Jito tip adaptive bid profiles.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BidProfile {
    Conservative,
    Balanced,
    Aggressive,
    MaxCapture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BidQuote {
    /// Compute-budget priority fee in micro-lamports per CU.
    pub priority_fee_micro_lamports: u64,
    /// Jito tip in lamports.
    pub jito_tip_lamports: u64,
    /// Fraction of expected profit allocated to fees (bps).
    pub profit_share_bps: u16,
}

impl BidProfile {
    pub fn profit_share_bps(self) -> u16 {
        match self {
            Self::Conservative => 500,  // 5%
            Self::Balanced => 1500,     // 15%
            Self::Aggressive => 3500,   // 35%
            Self::MaxCapture => 6000,   // 60%
        }
    }

    pub fn base_priority_micro_lamports(self) -> u64 {
        match self {
            Self::Conservative => 1_000,
            Self::Balanced => 10_000,
            Self::Aggressive => 100_000,
            Self::MaxCapture => 1_000_000,
        }
    }

    pub fn base_tip_lamports(self) -> u64 {
        match self {
            Self::Conservative => 1_000,
            Self::Balanced => 10_000,
            Self::Aggressive => 100_000,
            Self::MaxCapture => 1_000_000,
        }
    }

    /// Adaptive bid from expected profit (USD micro ≈ treated as relative scale for tip).
    /// Tip scales with profit share; priority fee uses profile base with a soft profit bump.
    pub fn compute_bid(self, expected_profit_usd_micro: u64) -> BidQuote {
        let share = self.profit_share_bps();
        // Map micro-USD profit to lamports tip budget with a crude $1 ≈ 5e6 lamports proxy
        // (overridable by live SOL oracle later). Floor at profile base tip.
        let profit_tip = (expected_profit_usd_micro as u128)
            .saturating_mul(share as u128)
            .saturating_mul(5)
            / 10_000;
        let jito_tip_lamports = (profit_tip as u64).max(self.base_tip_lamports());

        let bump = (expected_profit_usd_micro / 1_000_000).min(10); // +0..10x soft bump
        let priority_fee_micro_lamports = self
            .base_priority_micro_lamports()
            .saturating_mul(1 + bump);

        BidQuote {
            priority_fee_micro_lamports,
            jito_tip_lamports,
            profit_share_bps: share,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_escalate() {
        let p = 2_000_000u64; // $2
        let c = BidProfile::Conservative.compute_bid(p);
        let a = BidProfile::Aggressive.compute_bid(p);
        let m = BidProfile::MaxCapture.compute_bid(p);
        assert!(c.jito_tip_lamports <= a.jito_tip_lamports);
        assert!(a.jito_tip_lamports <= m.jito_tip_lamports);
        assert!(c.priority_fee_micro_lamports < a.priority_fee_micro_lamports);
    }

    #[test]
    fn floor_at_base_tip() {
        let q = BidProfile::Balanced.compute_bid(0);
        assert_eq!(q.jito_tip_lamports, BidProfile::Balanced.base_tip_lamports());
    }
}
