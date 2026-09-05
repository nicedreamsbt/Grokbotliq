//! Project 0 / marginfi-v2 adapter: classic + receivership liquidation.
//!
//! Verified program IDs from docs.0.xyz (2026-09-05).
//! Discriminators: TODO from marginfi IDL.

use liq_core::{amount_to_usd_micro, PriceFx, Pubkey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MARGINFI_PROGRAM_ID_MAINNET: &str = "MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA";
pub const MARGINFI_PROGRAM_ID_STAGING: &str = "stag8sTKds2h4KzjUw3zKTsxbqvT4XKHdaR9X9E6Rct";

/// Classic liquidator premium (approx, from docs).
pub const CLASSIC_LIQUIDATOR_PREMIUM_BPS: u16 = 250;
pub const CLASSIC_INSURANCE_BPS: u16 = 250;

/// Default receivership max fee until FeeState is fetched live.
pub const DEFAULT_RECEIVERSHIP_MAX_FEE_BPS: u16 = 1000;

#[derive(Debug, Error)]
pub enum P0Error {
    #[error("missing bank price")]
    MissingPrice,
    #[error("account healthy")]
    Healthy,
    #[error("receivership profit cap exceeded")]
    ProfitCap,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankMeta {
    pub bank: Pubkey,
    pub mint: Pubkey,
    pub maint_asset_weight_fx: u64, // 1e6 = 1.0
    pub maint_liab_weight_fx: u64,
    pub decimals: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BalanceKind {
    Asset,
    Liability,
    Empty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub bank: Pubkey,
    pub kind: BalanceKind,
    /// Share-adjusted token amount (already converted off-chain for health).
    pub amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarginfiAccountView {
    pub address: Pubkey,
    pub group: Pubkey,
    pub balances: Vec<Balance>,
}

#[derive(Debug, Clone)]
pub struct BankBook {
    pub banks: Vec<BankMeta>,
    pub prices: Vec<(Pubkey, PriceFx)>, // mint -> price
    pub confidence: Vec<(Pubkey, u64)>, // mint -> c in 1e6 (5% = 50_000)
}

impl BankBook {
    pub fn bank(&self, key: &Pubkey) -> Option<&BankMeta> {
        self.banks.iter().find(|b| &b.bank == key)
    }
    pub fn price(&self, mint: &Pubkey) -> Option<PriceFx> {
        self.prices.iter().find(|(m, _)| m == mint).map(|(_, p)| *p)
    }
    pub fn conf(&self, mint: &Pubkey) -> u64 {
        self.confidence
            .iter()
            .find(|(m, _)| m == mint)
            .map(|(_, c)| (*c).min(50_000))
            .unwrap_or(0)
    }
}

/// Maintenance health (can be negative). Units: micro-USD weighted.
pub fn maintenance_health(account: &MarginfiAccountView, book: &BankBook) -> Result<i128, P0Error> {
    let mut assets: i128 = 0;
    let mut liabs: i128 = 0;
    for bal in &account.balances {
        if bal.kind == BalanceKind::Empty || bal.amount == 0 {
            continue;
        }
        let meta = book.bank(&bal.bank).ok_or(P0Error::MissingPrice)?;
        let px = book.price(&meta.mint).ok_or(P0Error::MissingPrice)?;
        let c = book.conf(&meta.mint);
        let adj = match bal.kind {
            BalanceKind::Asset => {
                // P * (1 - c)
                PriceFx(px.0.saturating_mul(1_000_000 - c as u128) / 1_000_000)
            }
            BalanceKind::Liability => {
                PriceFx(px.0.saturating_mul(1_000_000 + c as u128) / 1_000_000)
            }
            BalanceKind::Empty => continue,
        };
        let usd = amount_to_usd_micro(bal.amount as u128, meta.decimals, adj) as i128;
        match bal.kind {
            BalanceKind::Asset => {
                assets += usd * meta.maint_asset_weight_fx as i128 / 1_000_000;
            }
            BalanceKind::Liability => {
                liabs += usd * meta.maint_liab_weight_fx as i128 / 1_000_000;
            }
            BalanceKind::Empty => {}
        }
    }
    Ok(assets - liabs)
}

pub fn is_liquidatable(account: &MarginfiAccountView, book: &BankBook) -> Result<bool, P0Error> {
    Ok(maintenance_health(account, book)? < 0)
}

/// Classic liquidation sizing: seize asset equity A, assume (1 - 0.025)*A liability.
/// Cannot raise health above zero — caller should size conservatively (70-80%).
pub fn classic_assumed_liability(seized_equity_usd_micro: u128) -> u128 {
    seized_equity_usd_micro * (10_000 - CLASSIC_LIQUIDATOR_PREMIUM_BPS as u128) / 10_000
}

pub fn classic_borrower_debt_relief(seized_equity_usd_micro: u128) -> u128 {
    let haircut = CLASSIC_LIQUIDATOR_PREMIUM_BPS as u128 + CLASSIC_INSURANCE_BPS as u128;
    seized_equity_usd_micro * (10_000 - haircut) / 10_000
}

/// Receivership profit check: Seized <= Repaid * (1 + max_fee).
pub fn receivership_profit_ok(
    seized_equity_usd_micro: u128,
    repaid_equity_usd_micro: u128,
    max_fee_bps: u16,
) -> bool {
    let cap = repaid_equity_usd_micro * (10_000 + max_fee_bps as u128) / 10_000;
    seized_equity_usd_micro <= cap
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiquidationMode {
    Classic,
    Receivership,
}

/// Placeholder discriminators — replace from IDL.
pub const IX_LENDING_ACCOUNT_LIQUIDATE: [u8; 8] = [0xee, 0xee, 0xee, 0xee, 0xee, 0xee, 0xee, 0x01];
pub const IX_START_LIQUIDATION: [u8; 8] = [0xee, 0xee, 0xee, 0xee, 0xee, 0xee, 0xee, 0x02];
pub const IX_END_LIQUIDATION: [u8; 8] = [0xee, 0xee, 0xee, 0xee, 0xee, 0xee, 0xee, 0x03];

pub fn encode_start_liquidation() -> Vec<u8> {
    IX_START_LIQUIDATION.to_vec()
}

pub fn encode_end_liquidation() -> Vec<u8> {
    IX_END_LIQUIDATION.to_vec()
}

pub fn encode_classic_liquidate(asset_amount: u64) -> Vec<u8> {
    let mut d = IX_LENDING_ACCOUNT_LIQUIDATE.to_vec();
    d.extend_from_slice(&asset_amount.to_le_bytes());
    d
}

pub fn receivership_ix_order() -> &'static [&'static str] {
    &[
        "ComputeBudget",
        "kamino_refresh_optional",
        "start_liquidation",
        "withdraw",
        "swap_optional",
        "repay",
        "end_liquidation",
    ]
}

pub fn classic_ix_order() -> &'static [&'static str] {
    &["switchboard_crank_optional", "lending_account_liquidate", "rebalance_optional"]
}

/// FeeState fields we care about (populated from chain later).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeStateView {
    pub liquidation_max_fee_bps: u16,
    pub liquidation_flat_sol_fee_lamports: u64,
    pub global_fee_wallet: Pubkey,
    pub paused: bool,
}

impl Default for FeeStateView {
    fn default() -> Self {
        Self {
            liquidation_max_fee_bps: DEFAULT_RECEIVERSHIP_MAX_FEE_BPS,
            liquidation_flat_sol_fee_lamports: 0,
            global_fee_wallet: Pubkey::default(),
            paused: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (MarginfiAccountView, BankBook) {
        let bank_a = Pubkey::test(10, 1);
        let bank_b = Pubkey::test(10, 2);
        let mint_a = Pubkey::test(11, 1);
        let mint_b = Pubkey::test(11, 2);
        let account = MarginfiAccountView {
            address: Pubkey::test(12, 1),
            group: Pubkey::test(12, 2),
            balances: vec![
                Balance {
                    bank: bank_a,
                    kind: BalanceKind::Asset,
                    amount: 10_000_000_000, // 10 SOL
                },
                Balance {
                    bank: bank_b,
                    kind: BalanceKind::Liability,
                    amount: 800_000_000, // 800 USDC
                },
            ],
        };
        let book = BankBook {
            banks: vec![
                BankMeta {
                    bank: bank_a,
                    mint: mint_a,
                    maint_asset_weight_fx: 900_000, // 0.9
                    maint_liab_weight_fx: 1_000_000,
                    decimals: 9,
                },
                BankMeta {
                    bank: bank_b,
                    mint: mint_b,
                    maint_asset_weight_fx: 1_000_000,
                    maint_liab_weight_fx: 1_100_000, // 1.1
                    decimals: 6,
                },
            ],
            prices: vec![
                (mint_a, PriceFx::from_f64(100.0)),
                (mint_b, PriceFx::from_f64(1.0)),
            ],
            confidence: vec![],
        };
        (account, book)
    }

    #[test]
    fn detects_negative_maint_health() {
        let (account, book) = sample();
        // assets = 10*100*0.9 = 900; liabs = 800*1.1 = 880; health +20 -> healthy
        assert!(!is_liquidatable(&account, &book).unwrap());

        // crash SOL price
        let mut book2 = book;
        book2.prices[0].1 = PriceFx::from_f64(50.0);
        // assets = 10*50*0.9 = 450; liabs = 880; health negative
        assert!(is_liquidatable(&account, &book2).unwrap());
    }

    #[test]
    fn receivership_profit_cap() {
        assert!(receivership_profit_ok(110, 100, 1000)); // 10%
        assert!(!receivership_profit_ok(111, 100, 1000));
    }
}
