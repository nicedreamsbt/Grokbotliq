//! Receivership sequencing: start → withdraw/swap/repay → end.
//! Profit constraint from end_liquidation: `seized <= repaid * (1 + max_fee)`.

use crate::DEFAULT_RECEIVERSHIP_MAX_FEE_BPS;

/// Receivership profit check: Seized <= Repaid * (1 + max_fee).
pub fn receivership_profit_ok(
    seized_equity_usd_micro: u128,
    repaid_equity_usd_micro: u128,
    max_fee_bps: u16,
) -> bool {
    let cap = repaid_equity_usd_micro * (10_000 + max_fee_bps as u128) / 10_000;
    seized_equity_usd_micro <= cap
}

/// Max seize allowed for a given repay and fee.
pub fn max_seize_for_repay(repaid_equity_usd_micro: u128, max_fee_bps: u16) -> u128 {
    repaid_equity_usd_micro * (10_000 + max_fee_bps as u128) / 10_000
}

/// Default fee when FeeState not yet loaded.
pub fn default_max_fee_bps() -> u16 {
    DEFAULT_RECEIVERSHIP_MAX_FEE_BPS
}

/// Health must improve vs start snapshot; typically cannot raise maint health above 0
/// unless equity is below closeout threshold (handled on-chain).
pub fn health_improved(pre_maint: i128, post_maint: i128) -> bool {
    post_maint > pre_maint
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profit_cap_at_10pct() {
        assert!(receivership_profit_ok(110, 100, 1000));
        assert!(!receivership_profit_ok(111, 100, 1000));
        assert_eq!(max_seize_for_repay(100, 1000), 110);
    }

    #[test]
    fn health_must_rise() {
        assert!(health_improved(-100, -50));
        assert!(!health_improved(-50, -50));
        assert!(!health_improved(-40, -50));
    }
}
