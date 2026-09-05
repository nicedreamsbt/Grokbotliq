//! Save/Solend flash loan composition.
//!
//! Primary path (preferred): FlashBorrowReserveLiquidity (tag 19) → liquidate →
//! (optional swap) → FlashRepayReserveLiquidity (tag 20).
//! Legacy path: FlashLoan tag 13 (deprecated CPI-receiver style) — kept for research.
//!
//! Layouts from vendored `idls/solend_sdk_0.1.0_instruction.rs`.
//! Writable/readonly flags reconciled against solend-sdk 0.1.0 helpers
//! (`flash_borrow_reserve_liquidity`, `flash_repay_reserve_liquidity`,
//! `liquidate_obligation_and_redeem_reserve_collateral`) — see meta flag tests.
//! Remaining uncertainty: Save mainnet tag 19/20 re-verify before live submit
//! (documented in PROTOCOL_RESEARCH.md).

use crate::{encode_liquidate_and_redeem, encode_refresh_obligation, encode_refresh_reserve, SaveIx};
use liq_core::{
    programs, AccountMeta, Instruction, LabeledIx, Pubkey, compute_unit_limit, compute_unit_price,
};
use serde::{Deserialize, Serialize};

/// FlashBorrowReserveLiquidity = 19 (solend-sdk pack).
pub const FLASH_BORROW_TAG: u8 = 19;
/// FlashRepayReserveLiquidity = 20.
pub const FLASH_REPAY_TAG: u8 = 20;
/// Legacy FlashLoan = 13 (deprecated).
pub const FLASH_LOAN_LEGACY_TAG: u8 = SaveIx::FlashLoan as u8;

/// Default flash fee assumption when reserve fees unknown (bps). Marked for live verify.
pub const DEFAULT_FLASH_FEE_BPS: u64 = 9;

pub fn encode_flash_borrow(liquidity_amount: u64) -> Vec<u8> {
    let mut d = vec![FLASH_BORROW_TAG];
    d.extend_from_slice(&liquidity_amount.to_le_bytes());
    d
}

pub fn encode_flash_repay(liquidity_amount: u64, borrow_instruction_index: u8) -> Vec<u8> {
    let mut d = vec![FLASH_REPAY_TAG];
    d.extend_from_slice(&liquidity_amount.to_le_bytes());
    d.push(borrow_instruction_index);
    d
}

pub fn encode_flash_loan_legacy(amount: u64) -> Vec<u8> {
    let mut d = vec![FLASH_LOAN_LEGACY_TAG];
    d.extend_from_slice(&amount.to_le_bytes());
    d
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashBorrowAccounts {
    pub source_liquidity: Pubkey,
    pub destination_liquidity: Pubkey,
    pub reserve: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
}

impl FlashBorrowAccounts {
    pub fn metas(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new(self.source_liquidity, false),
            AccountMeta::new(self.destination_liquidity, false),
            AccountMeta::new(self.reserve, false),
            AccountMeta::new_readonly(self.lending_market, false),
            AccountMeta::new_readonly(self.lending_market_authority, false),
            AccountMeta::new_readonly(programs::sysvar_instructions(), false),
            AccountMeta::new_readonly(programs::token(), false),
        ]
    }

    pub fn build_ix(&self, liquidity_amount: u64) -> Instruction {
        Instruction::new(
            programs::save(),
            self.metas(),
            encode_flash_borrow(liquidity_amount),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashRepayAccounts {
    pub source_liquidity: Pubkey,
    pub destination_liquidity: Pubkey,
    pub fee_receiver: Pubkey,
    pub host_fee_receiver: Pubkey,
    pub reserve: Pubkey,
    pub lending_market: Pubkey,
    pub user_transfer_authority: Pubkey,
}

impl FlashRepayAccounts {
    pub fn metas(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new(self.source_liquidity, false),
            AccountMeta::new(self.destination_liquidity, false),
            AccountMeta::new(self.fee_receiver, false),
            AccountMeta::new(self.host_fee_receiver, false),
            AccountMeta::new(self.reserve, false),
            AccountMeta::new_readonly(self.lending_market, false),
            AccountMeta::new_readonly(self.user_transfer_authority, true),
            AccountMeta::new_readonly(programs::sysvar_instructions(), false),
            AccountMeta::new_readonly(programs::token(), false),
        ]
    }

    pub fn build_ix(&self, liquidity_amount: u64, borrow_ix_index: u8) -> Instruction {
        Instruction::new(
            programs::save(),
            self.metas(),
            encode_flash_repay(liquidity_amount, borrow_ix_index),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveLiquidateAccounts {
    pub source_liquidity: Pubkey,
    pub destination_collateral: Pubkey,
    pub destination_liquidity: Pubkey,
    pub repay_reserve: Pubkey,
    pub repay_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve: Pubkey,
    pub withdraw_reserve_collateral_mint: Pubkey,
    pub withdraw_reserve_collateral_supply: Pubkey,
    pub withdraw_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve_fee_receiver: Pubkey,
    pub obligation: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub user_transfer_authority: Pubkey,
}

impl SaveLiquidateAccounts {
    pub fn metas(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new(self.source_liquidity, false),
            AccountMeta::new(self.destination_collateral, false),
            AccountMeta::new(self.destination_liquidity, false),
            AccountMeta::new(self.repay_reserve, false),
            AccountMeta::new(self.repay_reserve_liquidity_supply, false),
            AccountMeta::new(self.withdraw_reserve, false),
            AccountMeta::new(self.withdraw_reserve_collateral_mint, false),
            AccountMeta::new(self.withdraw_reserve_collateral_supply, false),
            AccountMeta::new(self.withdraw_reserve_liquidity_supply, false),
            AccountMeta::new(self.withdraw_reserve_fee_receiver, false),
            AccountMeta::new(self.obligation, false),
            AccountMeta::new_readonly(self.lending_market, false),
            AccountMeta::new_readonly(self.lending_market_authority, false),
            AccountMeta::new_readonly(self.user_transfer_authority, true),
            AccountMeta::new_readonly(programs::token(), false),
        ]
    }

    pub fn build_ix(&self, liquidity_amount: u64) -> Instruction {
        Instruction::new(
            programs::save(),
            self.metas(),
            encode_liquidate_and_redeem(liquidity_amount),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveFlashPlanAccounts {
    pub flash_borrow: FlashBorrowAccounts,
    pub liquidate: SaveLiquidateAccounts,
    pub flash_repay: FlashRepayAccounts,
    pub refresh_reserves: Vec<Pubkey>,
    pub obligation: Pubkey,
}

/// Atomic plan: CU → refresh* → flash_borrow → liquidate → [swaps] → flash_repay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveFlashAtomicPlan {
    pub labeled: Vec<LabeledIx>,
    /// Index of FlashBorrow within the final instruction list (for repay's borrow_instruction_index).
    pub flash_borrow_index: u8,
    pub liquidity_amount: u64,
}

pub fn build_flash_atomic_plan(
    accounts: &SaveFlashPlanAccounts,
    liquidity_amount: u64,
    swap_ixs: &[LabeledIx],
    cu_limit: u32,
    cu_price: u64,
) -> SaveFlashAtomicPlan {
    let mut labeled = Vec::new();
    labeled.push(LabeledIx {
        label: "ComputeBudget:SetComputeUnitLimit".into(),
        ix: compute_unit_limit(cu_limit),
    });
    labeled.push(LabeledIx {
        label: "ComputeBudget:SetComputeUnitPrice".into(),
        ix: compute_unit_price(cu_price),
    });
    for _r in &accounts.refresh_reserves {
        labeled.push(LabeledIx {
            label: "RefreshReserve".into(),
            ix: Instruction::new(
                programs::save(),
                vec![AccountMeta::new(*_r, false)],
                encode_refresh_reserve(),
            ),
        });
    }
    labeled.push(LabeledIx {
        label: "RefreshObligation".into(),
        ix: Instruction::new(
            programs::save(),
            vec![AccountMeta::new(accounts.obligation, false)],
            encode_refresh_obligation(),
        ),
    });

    let flash_borrow_index = labeled.len() as u8;
    labeled.push(LabeledIx {
        label: "FlashBorrowReserveLiquidity".into(),
        ix: accounts.flash_borrow.build_ix(liquidity_amount),
    });
    labeled.push(LabeledIx {
        label: "LiquidateObligationAndRedeemReserveCollateral".into(),
        ix: accounts.liquidate.build_ix(liquidity_amount),
    });
    for s in swap_ixs {
        labeled.push(s.clone());
    }
    labeled.push(LabeledIx {
        label: "FlashRepayReserveLiquidity".into(),
        ix: accounts.flash_repay.build_ix(liquidity_amount, flash_borrow_index),
    });

    SaveFlashAtomicPlan {
        labeled,
        flash_borrow_index,
        liquidity_amount,
    }
}

/// Inventory (non-flash) Save liquidation sequence.
pub fn build_inventory_liquidation_plan(
    obligation: Pubkey,
    reserves: &[Pubkey],
    liquidate: &SaveLiquidateAccounts,
    liquidity_amount: u64,
    swap_ixs: &[LabeledIx],
    cu_limit: u32,
    cu_price: u64,
) -> Vec<LabeledIx> {
    let mut labeled = vec![
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitLimit".into(),
            ix: compute_unit_limit(cu_limit),
        },
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitPrice".into(),
            ix: compute_unit_price(cu_price),
        },
    ];
    for r in reserves {
        labeled.push(LabeledIx {
            label: "RefreshReserve".into(),
            ix: Instruction::new(
                programs::save(),
                vec![AccountMeta::new(*r, false)],
                encode_refresh_reserve(),
            ),
        });
    }
    labeled.push(LabeledIx {
        label: "RefreshObligation".into(),
        ix: Instruction::new(
            programs::save(),
            vec![AccountMeta::new(obligation, false)],
            encode_refresh_obligation(),
        ),
    });
    labeled.push(LabeledIx {
        label: "LiquidateObligationAndRedeemReserveCollateral".into(),
        ix: liquidate.build_ix(liquidity_amount),
    });
    labeled.extend(swap_ixs.iter().cloned());
    labeled
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_accounts() -> SaveFlashPlanAccounts {
        let pk = |i| Pubkey::test(7, i);
        SaveFlashPlanAccounts {
            flash_borrow: FlashBorrowAccounts {
                source_liquidity: pk(1),
                destination_liquidity: pk(2),
                reserve: pk(3),
                lending_market: pk(4),
                lending_market_authority: pk(5),
            },
            liquidate: SaveLiquidateAccounts {
                source_liquidity: pk(2),
                destination_collateral: pk(10),
                destination_liquidity: pk(11),
                repay_reserve: pk(3),
                repay_reserve_liquidity_supply: pk(12),
                withdraw_reserve: pk(13),
                withdraw_reserve_collateral_mint: pk(14),
                withdraw_reserve_collateral_supply: pk(15),
                withdraw_reserve_liquidity_supply: pk(16),
                withdraw_reserve_fee_receiver: pk(17),
                obligation: pk(18),
                lending_market: pk(4),
                lending_market_authority: pk(5),
                user_transfer_authority: pk(19),
            },
            flash_repay: FlashRepayAccounts {
                source_liquidity: pk(2),
                destination_liquidity: pk(1),
                fee_receiver: pk(20),
                host_fee_receiver: pk(21),
                reserve: pk(3),
                lending_market: pk(4),
                user_transfer_authority: pk(19),
            },
            refresh_reserves: vec![pk(3), pk(13)],
            obligation: pk(18),
        }
    }

    #[test]
    fn flash_ix_ordering_borrow_before_liquidate_before_repay() {
        let plan = build_flash_atomic_plan(&sample_accounts(), 1_000_000, &[], 400_000, 1_000);
        let labels: Vec<_> = plan.labeled.iter().map(|l| l.label.as_str()).collect();
        let borrow_pos = labels
            .iter()
            .position(|l| *l == "FlashBorrowReserveLiquidity")
            .unwrap();
        let liq_pos = labels
            .iter()
            .position(|l| *l == "LiquidateObligationAndRedeemReserveCollateral")
            .unwrap();
        let repay_pos = labels
            .iter()
            .position(|l| *l == "FlashRepayReserveLiquidity")
            .unwrap();
        assert!(borrow_pos < liq_pos);
        assert!(liq_pos < repay_pos);
        assert_eq!(plan.flash_borrow_index as usize, borrow_pos);
        // repay data encodes borrow index
        let repay_data = &plan.labeled[repay_pos].ix.data;
        assert_eq!(repay_data[0], FLASH_REPAY_TAG);
        assert_eq!(*repay_data.last().unwrap(), plan.flash_borrow_index);
    }

    #[test]
    fn flash_data_bytes_and_account_metas() {
        let a = sample_accounts();
        let borrow = a.flash_borrow.build_ix(42);
        assert_eq!(borrow.data[0], FLASH_BORROW_TAG);
        assert_eq!(&borrow.data[1..9], &42u64.to_le_bytes());
        assert_eq!(borrow.accounts.len(), 7);
        assert!(!borrow.data.is_empty());
        let repay = a.flash_repay.build_ix(42, 4);
        assert_eq!(repay.data[0], FLASH_REPAY_TAG);
        assert_eq!(repay.accounts.len(), 9);
        assert!(repay.accounts.iter().any(|m| m.is_signer));
    }

    #[test]
    fn legacy_flash_loan_tag_13() {
        let d = encode_flash_loan_legacy(99);
        assert_eq!(d[0], 13);
    }

    #[test]
    fn flash_borrow_meta_writable_flags_match_solend_sdk() {
        let a = sample_accounts().flash_borrow;
        let m = a.metas();
        assert_eq!(m.len(), 7);
        // source, dest, reserve writable; market, authority, sysvar, token readonly
        assert!(m[0].is_writable && !m[0].is_signer);
        assert!(m[1].is_writable);
        assert!(m[2].is_writable);
        assert!(!m[3].is_writable); // lending_market
        assert!(!m[4].is_writable); // lending_market_authority
        assert!(!m[5].is_writable); // instructions
        assert!(!m[6].is_writable); // token
    }

    #[test]
    fn flash_repay_meta_writable_flags_match_solend_sdk() {
        let a = sample_accounts().flash_repay;
        let m = a.metas();
        assert_eq!(m.len(), 9);
        assert!(m[0].is_writable);
        assert!(m[1].is_writable);
        assert!(m[2].is_writable); // fee_receiver
        assert!(m[3].is_writable); // host_fee_receiver
        assert!(m[4].is_writable); // reserve
        assert!(!m[5].is_writable); // lending_market
        assert!(!m[6].is_writable && m[6].is_signer); // user authority
        assert!(!m[7].is_writable);
        assert!(!m[8].is_writable);
    }

    #[test]
    fn liquidate_meta_writable_flags_match_solend_sdk() {
        let a = sample_accounts().liquidate;
        let m = a.metas();
        assert_eq!(m.len(), 15);
        // first 11 writable per SDK; market, authority, user(signer), token readonly
        for i in 0..11 {
            assert!(m[i].is_writable, "idx {i} should be writable");
        }
        assert!(!m[11].is_writable); // lending_market
        assert!(!m[12].is_writable); // authority
        assert!(!m[13].is_writable && m[13].is_signer);
        assert!(!m[14].is_writable); // token
    }
}
