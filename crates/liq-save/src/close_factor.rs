//! Close-factor sizing for Save/Solend liquidations.

use crate::{SaveError, DEFAULT_CLOSE_FACTOR_BPS};

pub fn max_repay(borrowed_amount: u64, close_factor_bps: u16) -> u64 {
    (borrowed_amount as u128 * close_factor_bps as u128 / 10_000) as u64
}

/// Choose repay amount: min(desired, close-factor cap, borrowed).
pub fn size_repay(
    borrowed_amount: u64,
    desired_repay: u64,
    close_factor_bps: u16,
) -> Result<u64, SaveError> {
    let cap = max_repay(borrowed_amount, close_factor_bps);
    let amt = desired_repay.min(cap).min(borrowed_amount);
    if amt == 0 {
        return Err(SaveError::EmptyRepay);
    }
    Ok(amt)
}

pub fn default_close_factor_bps() -> u16 {
    DEFAULT_CLOSE_FACTOR_BPS
}

/// Approximate collateral seized given liquidation bonus (bps).
pub fn seize_collateral_amount(
    repay_usd_micro: u128,
    coll_price_usd_micro_per_token: u128,
    coll_decimals: u8,
    bonus_bps: u16,
) -> u64 {
    if coll_price_usd_micro_per_token == 0 {
        return 0;
    }
    let with_bonus = repay_usd_micro * (10_000 + bonus_bps as u128) / 10_000;
    let tokens = with_bonus
        .saturating_mul(10u128.pow(coll_decimals as u32))
        / coll_price_usd_micro_per_token;
    tokens as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_factor_half() {
        assert_eq!(max_repay(1_000_000, 5_000), 500_000);
        assert_eq!(size_repay(1_000_000, 800_000, 5_000).unwrap(), 500_000);
        assert_eq!(size_repay(1_000_000, 100_000, 5_000).unwrap(), 100_000);
        assert!(size_repay(0, 1, 5_000).is_err());
    }

    #[test]
    fn seize_with_5pct_bonus() {
        // repay $100 (1e8 micro), coll $50/token with 0 decimals => 2 tokens; +5% => 2.1 -> 2
        let seized = seize_collateral_amount(100_000_000, 50_000_000, 0, 500);
        assert_eq!(seized, 2);
    }
}
