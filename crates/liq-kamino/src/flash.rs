//! Kamino klend flash borrow / repay.
//!
//! Discriminators verified against `@kamino-finance/klend-sdk` codegen (user-confirmed 2026-09-05):
//! borrow `[135,231,52,167,7,52,212,193]`, repay `[185,117,0,203,96,245,180,186]`.
//! Account metas from vendored `idls/klend.json`
//! (`flashBorrowReserveLiquidity` / `flashRepayReserveLiquidity`).
//!
//! Optional referrer accounts: when absent, official Kamino codegen passes the
//! **KLend program ID as readonly** (not lending_market as a writable placeholder).

use liq_core::{programs, AccountMeta, Instruction, Pubkey};
use serde::{Deserialize, Serialize};

pub mod disc {
    /// `global:flash_borrow_reserve_liquidity`
    pub const FLASH_BORROW: [u8; 8] = [135, 231, 52, 167, 7, 52, 212, 193];
    /// `global:flash_repay_reserve_liquidity`
    pub const FLASH_REPAY: [u8; 8] = [185, 117, 0, 203, 96, 245, 180, 186];
}

/// Klend supports flash borrow/repay on reserves (present in IDL).
pub const KAMINO_FLASH_SUPPORTED: bool = true;

pub fn encode_flash_borrow(liquidity_amount: u64) -> Vec<u8> {
    let mut d = disc::FLASH_BORROW.to_vec();
    d.extend_from_slice(&liquidity_amount.to_le_bytes());
    d
}

pub fn encode_flash_repay(liquidity_amount: u64, borrow_instruction_index: u8) -> Vec<u8> {
    let mut d = disc::FLASH_REPAY.to_vec();
    d.extend_from_slice(&liquidity_amount.to_le_bytes());
    d.push(borrow_instruction_index);
    d
}

/// Readonly KLend program id placeholder used when optional referrer accounts are absent.
pub fn absent_referrer_meta() -> AccountMeta {
    AccountMeta::new_readonly(programs::klend(), false)
}

fn referrer_meta(key: Option<Pubkey>) -> AccountMeta {
    match key {
        Some(k) => AccountMeta::new(k, false),
        None => absent_referrer_meta(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashBorrowAccounts {
    pub user_transfer_authority: Pubkey,
    pub lending_market_authority: Pubkey,
    pub lending_market: Pubkey,
    pub reserve: Pubkey,
    pub reserve_liquidity_mint: Pubkey,
    pub reserve_source_liquidity: Pubkey,
    pub user_destination_liquidity: Pubkey,
    pub reserve_liquidity_fee_receiver: Pubkey,
    /// Optional referrer; `None` → KLend program id readonly (codegen convention).
    pub referrer_token_state: Option<Pubkey>,
    pub referrer_account: Option<Pubkey>,
}

impl FlashBorrowAccounts {
    pub fn metas(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new_readonly(self.user_transfer_authority, true),
            AccountMeta::new_readonly(self.lending_market_authority, false),
            AccountMeta::new_readonly(self.lending_market, false),
            AccountMeta::new(self.reserve, false),
            AccountMeta::new_readonly(self.reserve_liquidity_mint, false),
            AccountMeta::new(self.reserve_source_liquidity, false),
            AccountMeta::new(self.user_destination_liquidity, false),
            AccountMeta::new(self.reserve_liquidity_fee_receiver, false),
            referrer_meta(self.referrer_token_state),
            referrer_meta(self.referrer_account),
            AccountMeta::new_readonly(programs::sysvar_instructions(), false),
            AccountMeta::new_readonly(programs::token(), false),
        ]
    }

    pub fn build_ix(&self, amount: u64) -> Instruction {
        Instruction::new(programs::klend(), self.metas(), encode_flash_borrow(amount))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashRepayAccounts {
    pub user_transfer_authority: Pubkey,
    pub lending_market_authority: Pubkey,
    pub lending_market: Pubkey,
    pub reserve: Pubkey,
    pub reserve_liquidity_mint: Pubkey,
    pub reserve_destination_liquidity: Pubkey,
    pub user_source_liquidity: Pubkey,
    pub reserve_liquidity_fee_receiver: Pubkey,
    pub referrer_token_state: Option<Pubkey>,
    pub referrer_account: Option<Pubkey>,
}

impl FlashRepayAccounts {
    pub fn metas(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new_readonly(self.user_transfer_authority, true),
            AccountMeta::new_readonly(self.lending_market_authority, false),
            AccountMeta::new_readonly(self.lending_market, false),
            AccountMeta::new(self.reserve, false),
            AccountMeta::new_readonly(self.reserve_liquidity_mint, false),
            AccountMeta::new(self.reserve_destination_liquidity, false),
            AccountMeta::new(self.user_source_liquidity, false),
            AccountMeta::new(self.reserve_liquidity_fee_receiver, false),
            referrer_meta(self.referrer_token_state),
            referrer_meta(self.referrer_account),
            AccountMeta::new_readonly(programs::sysvar_instructions(), false),
            AccountMeta::new_readonly(programs::token(), false),
        ]
    }

    pub fn build_ix(&self, amount: u64, borrow_ix_index: u8) -> Instruction {
        Instruction::new(
            programs::klend(),
            self.metas(),
            encode_flash_repay(amount, borrow_ix_index),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_borrow(with_referrer: bool) -> FlashBorrowAccounts {
        FlashBorrowAccounts {
            user_transfer_authority: Pubkey::test(2, 1),
            lending_market_authority: Pubkey::test(2, 2),
            lending_market: Pubkey::test(2, 3),
            reserve: Pubkey::test(2, 4),
            reserve_liquidity_mint: Pubkey::test(2, 5),
            reserve_source_liquidity: Pubkey::test(2, 6),
            user_destination_liquidity: Pubkey::test(2, 7),
            reserve_liquidity_fee_receiver: Pubkey::test(2, 8),
            referrer_token_state: with_referrer.then_some(Pubkey::test(2, 9)),
            referrer_account: with_referrer.then_some(Pubkey::test(2, 10)),
        }
    }

    #[test]
    fn flash_supported_and_data_layout() {
        assert!(KAMINO_FLASH_SUPPORTED);
        let d = encode_flash_borrow(100);
        assert_eq!(&d[..8], &disc::FLASH_BORROW);
        assert_eq!(&d[8..], &100u64.to_le_bytes());
        let r = encode_flash_repay(100, 3);
        assert_eq!(&r[..8], &disc::FLASH_REPAY);
        assert_eq!(r[16], 3);
    }

    #[test]
    fn absent_referrer_uses_klend_program_id_readonly() {
        let metas = sample_borrow(false).metas();
        assert_eq!(metas.len(), 12);
        // indices 8 and 9 are referrerTokenState / referrerAccount
        let rts = &metas[8];
        let ra = &metas[9];
        assert_eq!(rts.pubkey, programs::klend());
        assert_eq!(ra.pubkey, programs::klend());
        assert!(!rts.is_writable);
        assert!(!ra.is_writable);
        assert!(!rts.is_signer);
        assert!(!ra.is_signer);
        // must NOT be lending_market writable placeholder
        assert_ne!(rts.pubkey, Pubkey::test(2, 3));
        assert!(!metas[8].is_writable);
    }

    #[test]
    fn present_referrer_is_writable() {
        let metas = sample_borrow(true).metas();
        assert_eq!(metas[8].pubkey, Pubkey::test(2, 9));
        assert!(metas[8].is_writable);
        assert_eq!(metas[9].pubkey, Pubkey::test(2, 10));
        assert!(metas[9].is_writable);
    }
}
