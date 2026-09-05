//! Project 0 / marginfi-v2 adapter: classic + receivership liquidation.
//!
//! Discriminators and FeeState layout pinned from public upstream:
//! `0dotxyz/marginfi-v2` `type-crate/src/constants.rs` and `type-crate/src/types/fee_state.rs`
//! (fetched 2026-09-05). See `idls/marginfi_liquidation_subset.json`.

mod accounts;
mod classic;
mod fee_state;
mod receivership;
mod tx_builder;

pub use accounts::*;
pub use classic::*;
pub use fee_state::*;
pub use receivership::*;
pub use tx_builder::*;

use liq_core::{amount_to_usd_micro, PriceFx, Pubkey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MARGINFI_PROGRAM_ID_MAINNET: &str = "MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA";
pub const MARGINFI_PROGRAM_ID_STAGING: &str = "stag8sTKds2h4KzjUw3zKTsxbqvT4XKHdaR9X9E6Rct";

/// Classic liquidator premium (DEFAULT_LIQUIDATION_FEE = 0.025 from type-crate).
pub const CLASSIC_LIQUIDATOR_PREMIUM_BPS: u16 = 250;
/// Insurance fee typically matches liquidator fee in docs (~2.5%).
pub const CLASSIC_INSURANCE_BPS: u16 = 250;

/// Default receivership max fee until FeeState is fetched live (~10%).
pub const DEFAULT_RECEIVERSHIP_MAX_FEE_BPS: u16 = 1000;

/// FeeState PDA seed (`FEE_STATE_SEED`).
pub const FEE_STATE_SEED: &str = "feestate";
/// Liquidation record PDA seed.
pub const LIQUIDATION_RECORD_SEED: &str = "liq_record";

/// Instruction discriminators from `type-crate::constants::ix_discriminators`
/// and Anchor `sha256("global:<name>")[0..8]` for classic liquidate.
pub mod disc {
    /// Custom (not plain Anchor sighash) — from type-crate.
    pub const INIT_LIQUIDATION_RECORD: [u8; 8] = [236, 213, 238, 126, 147, 251, 164, 8];
    pub const START_LIQUIDATION: [u8; 8] = [244, 93, 90, 214, 192, 166, 191, 21];
    pub const END_LIQUIDATION: [u8; 8] = [110, 11, 244, 54, 229, 181, 22, 184];
    pub const LENDING_ACCOUNT_WITHDRAW: [u8; 8] = [36, 72, 74, 19, 210, 210, 192, 192];
    pub const LENDING_ACCOUNT_REPAY: [u8; 8] = [79, 209, 172, 177, 222, 51, 173, 151];
    /// Anchor sighash `global:lending_account_liquidate`.
    pub const LENDING_ACCOUNT_LIQUIDATE: [u8; 8] = [214, 169, 151, 213, 251, 167, 86, 219];
    pub const START_FLASHLOAN: [u8; 8] = [14, 131, 33, 220, 81, 186, 180, 107];
    pub const END_FLASHLOAN: [u8; 8] = [105, 124, 201, 106, 153, 2, 8, 156];
}

/// Account discriminators from `type-crate::constants::discriminators`.
pub mod account_disc {
    pub const FEE_STATE: [u8; 8] = [63, 224, 16, 85, 193, 36, 235, 220];
    pub const LIQUIDATION_RECORD: [u8; 8] = [95, 116, 23, 132, 89, 210, 245, 162];
    pub const ACCOUNT: [u8; 8] = [67, 178, 130, 109, 126, 114, 28, 42];
    pub const BANK: [u8; 8] = [142, 49, 166, 242, 50, 66, 97, 188];
    pub const GROUP: [u8; 8] = [182, 23, 173, 240, 151, 206, 182, 67];
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum P0Error {
    #[error("missing bank price")]
    MissingPrice,
    #[error("account healthy")]
    Healthy,
    #[error("receivership profit cap exceeded")]
    ProfitCap,
    #[error("fee state parse error: {0}")]
    FeeState(&'static str),
    #[error("protocol paused")]
    Paused,
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
    pub confidence: Vec<(Pubkey, u64)>, // mint -> c in 1e6 (capped at 5% = 50_000)
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
///
/// Matches docs: assets use P*(1-c) with maint asset weight; liabilities use P*(1+c)
/// with maint liability weight. Liquidatable when health < 0.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiquidationMode {
    Classic,
    Receivership,
}

/// Encode helpers (data only — account metas via `accounts` module).
pub fn encode_start_liquidation() -> Vec<u8> {
    disc::START_LIQUIDATION.to_vec()
}

pub fn encode_end_liquidation() -> Vec<u8> {
    disc::END_LIQUIDATION.to_vec()
}

pub fn encode_classic_liquidate(asset_amount: u64) -> Vec<u8> {
    let mut d = disc::LENDING_ACCOUNT_LIQUIDATE.to_vec();
    d.extend_from_slice(&asset_amount.to_le_bytes());
    d
}

pub fn encode_withdraw(amount: u64, withdraw_all: bool) -> Vec<u8> {
    let mut d = disc::LENDING_ACCOUNT_WITHDRAW.to_vec();
    d.extend_from_slice(&amount.to_le_bytes());
    d.push(u8::from(withdraw_all));
    d
}

pub fn encode_repay(amount: u64, repay_all: bool) -> Vec<u8> {
    let mut d = disc::LENDING_ACCOUNT_REPAY.to_vec();
    d.extend_from_slice(&amount.to_le_bytes());
    d.push(u8::from(repay_all));
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

/// Plan a liquidation sequence (no signing / network).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidationPlan {
    pub mode: LiquidationMode,
    pub ixs: Vec<String>,
    pub datas: Vec<Vec<u8>>,
}

pub fn plan_classic(asset_amount: u64) -> LiquidationPlan {
    LiquidationPlan {
        mode: LiquidationMode::Classic,
        ixs: classic_ix_order().iter().map(|s| (*s).to_string()).collect(),
        datas: vec![encode_classic_liquidate(asset_amount)],
    }
}

pub fn plan_receivership(withdraw_amount: u64, repay_amount: u64) -> LiquidationPlan {
    LiquidationPlan {
        mode: LiquidationMode::Receivership,
        ixs: receivership_ix_order().iter().map(|s| (*s).to_string()).collect(),
        datas: vec![
            encode_start_liquidation(),
            encode_withdraw(withdraw_amount, false),
            encode_repay(repay_amount, false),
            encode_end_liquidation(),
        ],
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
                    amount: 10_000_000_000,
                },
                Balance {
                    bank: bank_b,
                    kind: BalanceKind::Liability,
                    amount: 800_000_000,
                },
            ],
        };
        let book = BankBook {
            banks: vec![
                BankMeta {
                    bank: bank_a,
                    mint: mint_a,
                    maint_asset_weight_fx: 900_000,
                    maint_liab_weight_fx: 1_000_000,
                    decimals: 9,
                },
                BankMeta {
                    bank: bank_b,
                    mint: mint_b,
                    maint_asset_weight_fx: 1_000_000,
                    maint_liab_weight_fx: 1_100_000,
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
        assert!(!is_liquidatable(&account, &book).unwrap());
        let mut book2 = book;
        book2.prices[0].1 = PriceFx::from_f64(50.0);
        assert!(is_liquidatable(&account, &book2).unwrap());
    }

    #[test]
    fn classic_math_matches_docs() {
        // q_ll = q_a * (1 - 0.025); q_lf = q_a * (1 - 0.05) in equity space
        let seized = 1_000_000u128;
        assert_eq!(classic_assumed_liability(seized), 975_000);
        assert_eq!(classic_borrower_debt_relief(seized), 950_000);
    }

    #[test]
    fn discriminators_match_upstream_pins() {
        assert_eq!(disc::START_LIQUIDATION[0], 244);
        assert_eq!(disc::END_LIQUIDATION[0], 110);
        assert_eq!(disc::LENDING_ACCOUNT_LIQUIDATE, [214, 169, 151, 213, 251, 167, 86, 219]);
        let data = encode_classic_liquidate(42);
        assert_eq!(&data[..8], &disc::LENDING_ACCOUNT_LIQUIDATE);
        assert_eq!(&data[8..16], &42u64.to_le_bytes());
    }

    #[test]
    fn receivership_plan_sequences_start_end() {
        let plan = plan_receivership(100, 90);
        assert_eq!(plan.mode, LiquidationMode::Receivership);
        assert_eq!(plan.datas[0], disc::START_LIQUIDATION);
        assert_eq!(plan.datas[3], disc::END_LIQUIDATION);
        assert!(receivership_profit_ok(110, 100, 1000));
        assert!(!receivership_profit_ok(111, 100, 1000));
    }
}
