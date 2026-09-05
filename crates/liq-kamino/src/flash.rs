//! Kamino klend flash borrow / repay.
//!
//! Discriminators: Anchor `sha256("global:<snake_name>")[0..8]` — matches pinned refresh/liq
//! pattern used elsewhere in this crate. Account metas from vendored `idls/klend.json`
//! (`flashBorrowReserveLiquidity` / `flashRepayReserveLiquidity`).
//!
//! Verification TODO: re-check discriminators against `@kamino-finance/klend-sdk` codegen
//! JS files before mainnet submit (IDL JSON in this pin lacks discriminator arrays).

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
    /// Optional referrer; pass lending_market as placeholder when unused.
    pub referrer_token_state: Pubkey,
    pub referrer_account: Pubkey,
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
            AccountMeta::new(self.referrer_token_state, false),
            AccountMeta::new(self.referrer_account, false),
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
    pub referrer_token_state: Pubkey,
    pub referrer_account: Pubkey,
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
            AccountMeta::new(self.referrer_token_state, false),
            AccountMeta::new(self.referrer_account, false),
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
}
