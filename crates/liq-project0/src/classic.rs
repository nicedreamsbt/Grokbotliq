//! Classic liquidation math from marginfi liquidate.rs docs:
//! `q_ll = q_a * p_a * (1 - f_l) / p_l`
//! `q_lf = q_a * p_a * (1 - (f_l + f_i)) / p_l`

use crate::{CLASSIC_INSURANCE_BPS, CLASSIC_LIQUIDATOR_PREMIUM_BPS};

/// Assumed liability the liquidator pays for seized equity `A`: `(1 - f_l) * A`.
pub fn classic_assumed_liability(seized_equity_usd_micro: u128) -> u128 {
    seized_equity_usd_micro * (10_000 - CLASSIC_LIQUIDATOR_PREMIUM_BPS as u128) / 10_000
}

/// Debt relief credited to the borrower: `(1 - (f_l + f_i)) * A`.
pub fn classic_borrower_debt_relief(seized_equity_usd_micro: u128) -> u128 {
    let haircut = CLASSIC_LIQUIDATOR_PREMIUM_BPS as u128 + CLASSIC_INSURANCE_BPS as u128;
    seized_equity_usd_micro * (10_000 - haircut) / 10_000
}

/// Insurance fund receipt: `q_ll - q_lf` in equity space.
pub fn classic_insurance_take(seized_equity_usd_micro: u128) -> u128 {
    classic_assumed_liability(seized_equity_usd_micro)
        .saturating_sub(classic_borrower_debt_relief(seized_equity_usd_micro))
}

/// Conservative size so post-liq health stays <= 0 (docs recommend ~70-80% of max).
pub fn size_classic_seize(max_seize_usd_micro: u128, fill_bps: u16) -> u128 {
    max_seize_usd_micro * (fill_bps as u128).min(10_000) / 10_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insurance_is_difference() {
        let seized = 10_000u128;
        assert_eq!(
            classic_insurance_take(seized),
            classic_assumed_liability(seized) - classic_borrower_debt_relief(seized)
        );
    }

    #[test]
    fn sizing_caps_at_100pct() {
        assert_eq!(size_classic_seize(1000, 8000), 800);
        assert_eq!(size_classic_seize(1000, 12_000), 1000);
    }
}
