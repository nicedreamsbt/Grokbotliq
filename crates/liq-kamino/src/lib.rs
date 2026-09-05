//! Kamino klend adapter: health math + liquidate v2 builders.
//!
//! Discriminators pinned from `@kamino-finance/klend-sdk@11.0.1` codegen.
//! Full IDL vendored at `idls/klend.json` (sha256 in PROTOCOL_RESEARCH.md).

mod accounts;
mod decode;
mod flash;
mod refresh;
mod scope;
mod tx_builder;

pub use accounts::*;
pub use decode::*;
pub use flash::*;
pub use refresh::*;
pub use scope::*;
pub use tx_builder::*;

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

/// Scope prices older than this many slots are treated as stale (kamino docs / research).
pub const SCOPE_MAX_AGE_SLOTS: u64 = 512;

pub mod disc {
    /// From klend-sdk codegen `refreshReserve.js`.
    pub const REFRESH_RESERVE: [u8; 8] = [2, 218, 138, 235, 79, 201, 25, 102];
    /// From klend-sdk codegen `refreshObligation.js`.
    pub const REFRESH_OBLIGATION: [u8; 8] = [33, 132, 147, 228, 151, 192, 72, 89];
    /// From klend-sdk codegen `liquidateObligationAndRedeemReserveCollateralV2.js`.
    pub const LIQUIDATE_V2: [u8; 8] = [162, 161, 35, 143, 30, 187, 185, 103];
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KaminoError {
    #[error("missing price for asset")]
    MissingPrice,
    #[error("invalid amount")]
    InvalidAmount,
    #[error("scope oracle stale")]
    ScopeStale,
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
/// Liquidatable when borrowed > sum(deposit * liq_threshold).
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

/// Approximate max repay given close factor (bps of borrowed amount).
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
    let num = with_bonus
        .saturating_mul(10u128.pow(withdraw_decimals as u32))
        .saturating_mul(PriceFx::SCALE);
    let den = withdraw_price.0.saturating_mul(1_000_000);
    if den == 0 {
        return 0;
    }
    (num / den) as u64
}

/// Instruction data for liquidate_obligation_and_redeem_reserve_collateral_v2.
pub fn encode_liquidate_v2_data(
    liquidity_amount: u64,
    min_acceptable_received: u64,
    max_allowed_ltv_override_percent: u64,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(8 + 8 * 3);
    data.extend_from_slice(&disc::LIQUIDATE_V2);
    data.extend_from_slice(&liquidity_amount.to_le_bytes());
    data.extend_from_slice(&min_acceptable_received.to_le_bytes());
    data.extend_from_slice(&max_allowed_ltv_override_percent.to_le_bytes());
    data
}

pub fn encode_refresh_reserve() -> Vec<u8> {
    disc::REFRESH_RESERVE.to_vec()
}

pub fn encode_refresh_obligation() -> Vec<u8> {
    disc::REFRESH_OBLIGATION.to_vec()
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
                deposited_amount: 10_000_000_000,
                decimals: 9,
                liq_threshold_bps: 8000,
            }],
            borrows: vec![KaminoBorrow {
                reserve: Pubkey::test(3, 2),
                mint: debt,
                borrowed_amount: 900_000_000,
                decimals: 6,
            }],
        };
        let prices = PriceMap {
            prices: vec![
                (coll, PriceFx::from_f64(100.0)),
                (debt, PriceFx::from_f64(1.0)),
            ],
        };
        assert!(is_liquidatable(&obl, &prices).unwrap());
        let (hf, _, _) = obligation_health(&obl, &prices).unwrap();
        assert!(hf.is_liquidatable());
    }

    #[test]
    fn encode_liquidate_data_matches_sdk_disc() {
        let d = encode_liquidate_v2_data(1_000_000, 0, 0);
        assert_eq!(d.len(), 32);
        assert_eq!(&d[..8], &disc::LIQUIDATE_V2);
        assert_eq!(disc::LIQUIDATE_V2, [162, 161, 35, 143, 30, 187, 185, 103]);
    }
}
