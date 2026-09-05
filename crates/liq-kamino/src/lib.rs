//! Kamino klend adapter: health math + instruction layout helpers.
//!
//! Discriminators: TODO load from published klend IDL at build time.
//! Layouts below match public docs / terminator patterns researched 2026-09-05.

use liq_core::{
    amount_to_usd_micro, health_factor_ratio, HealthFx, PriceFx, Pubkey,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const KLEND_PROGRAM_ID_MAINNET: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";
pub const KLEND_PROGRAM_ID_STAGING: &str = "SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh";
pub const SCOPE_PROGRAM_ID_MAINNET: &str = "HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ";

/// Scope chain sentinel (unused hop).
pub const SCOPE_CHAIN_SENTINEL: u16 = 65535;

#[derive(Debug, Error)]
pub enum KaminoError {
    #[error("missing price for asset")]
    MissingPrice,
    #[error("invalid amount")]
    InvalidAmount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaminoDeposit {
    pub reserve: Pubkey,
    pub mint: Pubkey,
    pub deposited_amount: u64,
    pub decimals: u8,
    /// liquidation_threshold in bps (e.g. 8000 = 80%).
    pub liq_threshold_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaminoBorrow {
    pub reserve: Pubkey,
    pub mint: Pubkey,
    pub borrowed_amount: u64,
    pub decimals: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaminoObligation {
    pub address: Pubkey,
    pub market: Pubkey,
    pub deposits: Vec<KaminoDeposit>,
    pub borrows: Vec<KaminoBorrow>,
}

#[derive(Debug, Clone)]
pub struct PriceMap {
    pub prices: Vec<(Pubkey, PriceFx)>,
}

impl PriceMap {
    pub fn get(&self, mint: &Pubkey) -> Option<PriceFx> {
        self.prices.iter().find(|(k, _)| k == mint).map(|(_, p)| *p)
    }
}

/// Compute HF and whether liquidatable using klend-style thresholds.
pub fn obligation_health(
    obl: &KaminoObligation,
    prices: &PriceMap,
) -> Result<(HealthFx, u128, u128), KaminoError> {
    let mut allowed: u128 = 0;
    for d in &obl.deposits {
        if d.deposited_amount == 0 {
            continue;
        }
        let px = prices.get(&d.mint).ok_or(KaminoError::MissingPrice)?;
        let usd = amount_to_usd_micro(d.deposited_amount as u128, d.decimals, px);
        allowed += usd * (d.liq_threshold_bps as u128) / 10_000;
    }
    let mut borrowed: u128 = 0;
    for b in &obl.borrows {
        if b.borrowed_amount == 0 {
            continue;
        }
        let px = prices.get(&b.mint).ok_or(KaminoError::MissingPrice)?;
        borrowed += amount_to_usd_micro(b.borrowed_amount as u128, b.decimals, px);
    }
    Ok((health_factor_ratio(allowed, borrowed), allowed, borrowed))
}

pub fn is_liquidatable(obl: &KaminoObligation, prices: &PriceMap) -> Result<bool, KaminoError> {
    let (hf, _, borrowed) = obligation_health(obl, prices)?;
    Ok(borrowed > 0 && hf.is_liquidatable())
}

/// Approximate max repay given close factor (bps of largest borrow).
pub fn max_repay_amount(borrowed_amount: u64, close_factor_bps: u16) -> u64 {
    (borrowed_amount as u128 * close_factor_bps as u128 / 10_000) as u64
}

/// Collateral received ~= repay_value * (1 + bonus_bps/10000) / withdraw_price.
pub fn collateral_out_amount(
    repay_amount: u64,
    repay_decimals: u8,
    repay_price: PriceFx,
    withdraw_decimals: u8,
    withdraw_price: PriceFx,
    bonus_bps: u16,
) -> u64 {
    let repay_usd = amount_to_usd_micro(repay_amount as u128, repay_decimals, repay_price);
    let with_bonus = repay_usd * (10_000 + bonus_bps as u128) / 10_000;
    // convert micro USD back to token amount
    // amount = usd_micro * 10^dec * 1e9 / (price * 1e6)
    let num = with_bonus
        .saturating_mul(10u128.pow(withdraw_decimals as u32))
        .saturating_mul(PriceFx::SCALE);
    let den = withdraw_price.0.saturating_mul(1_000_000);
    if den == 0 {
        return 0;
    }
    (num / den) as u64
}

/// Account meta description for liquidate v2 (names only; keys filled by runtime).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidateV2Accounts {
    pub liquidator: Pubkey,
    pub obligation: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub repay_reserve: Pubkey,
    pub withdraw_reserve: Pubkey,
    pub user_source_liquidity: Pubkey,
    pub user_destination_collateral: Pubkey,
    pub user_destination_liquidity: Pubkey,
    /// Remaining: deposit reserves for health.
    pub deposit_reserves: Vec<Pubkey>,
}

/// Instruction data for liquidate_obligation_and_redeem_reserve_collateral_v2.
/// TODO: replace DISCRIMINATOR with IDL sighash once klend.json is vendored.
pub const LIQUIDATE_V2_DISCRIMINATOR: [u8; 8] = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]; // placeholder

pub fn encode_liquidate_v2_data(
    liquidity_amount: u64,
    min_acceptable_received: u64,
    max_allowed_ltv_override_percent: u64,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(8 + 8 * 3);
    data.extend_from_slice(&LIQUIDATE_V2_DISCRIMINATOR);
    data.extend_from_slice(&liquidity_amount.to_le_bytes());
    data.extend_from_slice(&min_acceptable_received.to_le_bytes());
    data.extend_from_slice(&max_allowed_ltv_override_percent.to_le_bytes());
    data
}

/// Suggested ix ordering labels for a Kamino liquidation tx.
pub fn liquidation_ix_order() -> &'static [&'static str] {
    &[
        "ComputeBudget",
        "scope_refresh_optional",
        "refresh_reserve*",
        "refresh_obligation",
        "liquidate_obligation_and_redeem_reserve_collateral_v2",
        "swap_optional",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unhealthy_when_borrows_exceed_allowed() {
        let coll = Pubkey::test(1, 1);
        let debt = Pubkey::test(1, 2);
        let obl = KaminoObligation {
            address: Pubkey::test(2, 1),
            market: Pubkey::test(2, 2),
            deposits: vec![KaminoDeposit {
                reserve: Pubkey::test(3, 1),
                mint: coll,
                deposited_amount: 10_000_000_000, // 10 SOL
                decimals: 9,
                liq_threshold_bps: 8000,
            }],
            borrows: vec![KaminoBorrow {
                reserve: Pubkey::test(3, 2),
                mint: debt,
                borrowed_amount: 900_000_000, // 900 USDC
                decimals: 6,
            }],
        };
        let prices = PriceMap {
            prices: vec![
                (coll, PriceFx::from_f64(100.0)),
                (debt, PriceFx::from_f64(1.0)),
            ],
        };
        // allowed = 10*100*0.8 = 800; borrowed = 900 -> unhealthy
        assert!(is_liquidatable(&obl, &prices).unwrap());
        let (hf, _, _) = obligation_health(&obl, &prices).unwrap();
        assert!(hf.is_liquidatable());
    }

    #[test]
    fn encode_liquidate_data_len() {
        let d = encode_liquidate_v2_data(1_000_000, 0, 0);
        assert_eq!(d.len(), 32);
    }
}
