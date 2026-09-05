//! Save Finance (Solend successor) adapter.
//!
//! Program ID verified from docs.save.finance/architecture/addresses.md.
//! Instruction tags follow classic SPL token-lending / Solend enum ordering.
//! Confirm against live Save binary before mainnet submit.

use liq_core::{
    amount_to_usd_micro, health_factor_ratio, HealthFx, PriceFx, Pubkey,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SAVE_PROGRAM_ID_MAINNET: &str = "So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo";

/// Classic Solend close factor (50%). Confirm on-chain.
pub const DEFAULT_CLOSE_FACTOR_BPS: u16 = 5_000;

/// LendingInstruction tag indices from solendprotocol/solana-program-library
/// (may drift if Save upgraded — TODO verify).
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum SaveIx {
    InitLendingMarket = 0,
    SetLendingMarketOwner = 1,
    InitReserve = 2,
    RefreshReserve = 3,
    DepositReserveLiquidity = 4,
    RedeemReserveCollateral = 5,
    InitObligation = 6,
    RefreshObligation = 7,
    DepositObligationCollateral = 8,
    WithdrawObligationCollateral = 9,
    BorrowObligationLiquidity = 10,
    RepayObligationLiquidity = 11,
    LiquidateObligation = 12,
    // later variants: LiquidateObligationAndRedeemReserveCollateral etc.
}

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("missing price")]
    MissingPrice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveDeposit {
    pub reserve: Pubkey,
    pub mint: Pubkey,
    pub deposited_amount: u64,
    pub decimals: u8,
    pub liq_threshold_bps: u16,
    pub liq_bonus_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveBorrow {
    pub reserve: Pubkey,
    pub mint: Pubkey,
    pub borrowed_amount: u64,
    pub decimals: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveObligation {
    pub address: Pubkey,
    pub lending_market: Pubkey,
    pub deposits: Vec<SaveDeposit>,
    pub borrows: Vec<SaveBorrow>,
}

#[derive(Debug, Clone)]
pub struct SavePrices {
    pub prices: Vec<(Pubkey, PriceFx)>,
}

impl SavePrices {
    pub fn get(&self, mint: &Pubkey) -> Option<PriceFx> {
        self.prices.iter().find(|(m, _)| m == mint).map(|(_, p)| *p)
    }
}

pub fn obligation_health(
    obl: &SaveObligation,
    prices: &SavePrices,
) -> Result<(HealthFx, u128, u128), SaveError> {
    let mut allowed = 0u128;
    for d in &obl.deposits {
        if d.deposited_amount == 0 {
            continue;
        }
        let px = prices.get(&d.mint).ok_or(SaveError::MissingPrice)?;
        let usd = amount_to_usd_micro(d.deposited_amount as u128, d.decimals, px);
        allowed += usd * d.liq_threshold_bps as u128 / 10_000;
    }
    let mut borrowed = 0u128;
    for b in &obl.borrows {
        if b.borrowed_amount == 0 {
            continue;
        }
        let px = prices.get(&b.mint).ok_or(SaveError::MissingPrice)?;
        borrowed += amount_to_usd_micro(b.borrowed_amount as u128, b.decimals, px);
    }
    Ok((health_factor_ratio(allowed, borrowed), allowed, borrowed))
}

pub fn is_liquidatable(obl: &SaveObligation, prices: &SavePrices) -> Result<bool, SaveError> {
    let (hf, _, borrowed) = obligation_health(obl, prices)?;
    Ok(borrowed > 0 && hf.is_liquidatable())
}

pub fn max_repay(borrowed_amount: u64, close_factor_bps: u16) -> u64 {
    (borrowed_amount as u128 * close_factor_bps as u128 / 10_000) as u64
}

/// Encode LiquidateObligation { liquidity_amount: u64 }.
pub fn encode_liquidate_obligation(liquidity_amount: u64) -> Vec<u8> {
    let mut data = vec![SaveIx::LiquidateObligation as u8];
    data.extend_from_slice(&liquidity_amount.to_le_bytes());
    data
}

pub fn encode_refresh_reserve() -> Vec<u8> {
    vec![SaveIx::RefreshReserve as u8]
}

pub fn encode_refresh_obligation() -> Vec<u8> {
    vec![SaveIx::RefreshObligation as u8]
}

pub fn liquidation_ix_order() -> &'static [&'static str] {
    &[
        "ComputeBudget",
        "RefreshReserve*",
        "RefreshObligation",
        "LiquidateObligation",
        "RedeemOptional",
        "SwapOptional",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liquidatable_and_close_factor() {
        let coll = Pubkey::test(1, 1);
        let debt = Pubkey::test(1, 2);
        let obl = SaveObligation {
            address: Pubkey::test(2, 1),
            lending_market: Pubkey::test(2, 2),
            deposits: vec![SaveDeposit {
                reserve: Pubkey::test(3, 1),
                mint: coll,
                deposited_amount: 1_000_000_000,
                decimals: 9,
                liq_threshold_bps: 8500,
                liq_bonus_bps: 500,
            }],
            borrows: vec![SaveBorrow {
                reserve: Pubkey::test(3, 2),
                mint: debt,
                borrowed_amount: 200_000_000,
                decimals: 6,
            }],
        };
        let prices = SavePrices {
            prices: vec![
                (coll, PriceFx::from_f64(50.0)),
                (debt, PriceFx::from_f64(1.0)),
            ],
        };
        // allowed = 50*0.85 = 42.5; borrowed = 200 -> liquidatable
        assert!(is_liquidatable(&obl, &prices).unwrap());
        assert_eq!(max_repay(200_000_000, DEFAULT_CLOSE_FACTOR_BPS), 100_000_000);
        let data = encode_liquidate_obligation(100);
        assert_eq!(data[0], 12);
        assert_eq!(&data[1..9], &100u64.to_le_bytes());
    }
}
