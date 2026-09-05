use crate::types::{HealthFx, PriceFx, Pubkey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionLeg {
    pub asset: Pubkey,
    pub amount: u128,
    /// Decimals for amount scaling (e.g. 6 or 9).
    pub decimals: u8,
    pub is_collateral: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightParams {
    pub asset: Pubkey,
    /// Maintenance collateral factor / asset weight (1e6 = 100%).
    pub maint_weight_fx: u64,
    /// Liquidation threshold for SPL-style (1e6 = 100%); used by Kamino/Save.
    pub liq_threshold_fx: u64,
}

/// Generic HF = collateral_power / borrows (scaled).
pub fn health_factor_ratio(collateral_usd_micro: u128, borrow_usd_micro: u128) -> HealthFx {
    if borrow_usd_micro == 0 {
        return HealthFx::from_f64(1000.0);
    }
    // HF = coll / borrow, scale 1e6
    let hf = (collateral_usd_micro.saturating_mul(HealthFx::SCALE as u128)) / borrow_usd_micro;
    HealthFx(hf as i128)
}

pub fn amount_to_usd_micro(amount: u128, decimals: u8, price: PriceFx) -> u128 {
    // amount * price / 10^decimals * 1e6 / 1e9  => micro USD
    // = amount * price * 1e6 / (10^decimals * 1e9)
    let den = 10u128.pow(decimals as u32).saturating_mul(PriceFx::SCALE);
    amount.saturating_mul(price.0).saturating_mul(1_000_000) / den.max(1)
}

/// Solve collateral trigger price for single-asset sensitivity (others fixed).
/// Returns price where HF == 1 given fixed_borrow_usd_micro and other_coll_usd_micro.
pub fn collateral_trigger_price(
    coll_amount: u128,
    coll_decimals: u8,
    liq_threshold_fx: u64,
    other_coll_power_usd_micro: u128,
    borrow_usd_micro: u128,
) -> Option<PriceFx> {
    if coll_amount == 0 || borrow_usd_micro <= other_coll_power_usd_micro {
        return None;
    }
    let need = borrow_usd_micro - other_coll_power_usd_micro;
    // need = amount * price / 10^dec * threshold * 1e6/1e6  in micro USD
    // price = need * 10^dec * PriceFx::SCALE / (amount * threshold_fx / 1e6) / 1e6
    //       = need * 10^dec * 1e9 * 1e6 / (amount * threshold_fx * 1e6)
    //       = need * 10^dec * 1e9 / (amount * threshold_fx)
    let num = need
        .saturating_mul(10u128.pow(coll_decimals as u32))
        .saturating_mul(PriceFx::SCALE);
    let den = coll_amount.saturating_mul(liq_threshold_fx as u128);
    if den == 0 {
        return None;
    }
    Some(PriceFx(num / den))
}
