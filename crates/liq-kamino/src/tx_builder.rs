//! Protocol-exact Kamino liquidation tx builders → Instruction lists.

use crate::accounts::LiquidateV2Accounts;
use crate::flash::{FlashBorrowAccounts, FlashRepayAccounts};
use crate::{encode_liquidate_v2_data, encode_refresh_obligation, encode_refresh_reserve};
use liq_core::{
    compute_unit_limit, compute_unit_price, programs, AccountMeta, Instruction, LabeledIx, Pubkey,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaminoTxBuildParams {
    pub obligation: Pubkey,
    pub deposit_reserves: Vec<Pubkey>,
    pub borrow_reserves: Vec<Pubkey>,
    pub liquidate: LiquidateV2Accounts,
    pub liquidity_amount: u64,
    pub min_acceptable_received: u64,
    pub max_allowed_ltv_override_percent: u64,
    pub cu_limit: u32,
    pub cu_price: u64,
    pub flash: Option<(FlashBorrowAccounts, FlashRepayAccounts)>,
}

fn refresh_ixs(obligation: Pubkey, reserves: &[Pubkey]) -> Vec<LabeledIx> {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for r in reserves {
        if seen.contains(r) {
            continue;
        }
        seen.push(*r);
        out.push(LabeledIx {
            label: "refresh_reserve".into(),
            ix: Instruction::new(
                programs::klend(),
                vec![AccountMeta::new(*r, false)],
                encode_refresh_reserve(),
            ),
        });
    }
    out.push(LabeledIx {
        label: "refresh_obligation".into(),
        ix: Instruction::new(
            programs::klend(),
            vec![AccountMeta::new(obligation, false)],
            encode_refresh_obligation(),
        ),
    });
    out
}

fn liquidate_ix(params: &KaminoTxBuildParams) -> LabeledIx {
    let metas: Vec<AccountMeta> = params
        .liquidate
        .metas()
        .into_iter()
        .map(|m| match m.role {
            crate::accounts::MetaRole::Signer => AccountMeta::new_readonly(m.key, true),
            crate::accounts::MetaRole::Writable => AccountMeta::new(m.key, false),
            crate::accounts::MetaRole::Readonly => AccountMeta::new_readonly(m.key, false),
        })
        .collect();
    LabeledIx {
        label: "liquidate_obligation_and_redeem_reserve_collateral_v2".into(),
        ix: Instruction::new(
            programs::klend(),
            metas,
            encode_liquidate_v2_data(
                params.liquidity_amount,
                params.min_acceptable_received,
                params.max_allowed_ltv_override_percent,
            ),
        ),
    }
}

/// Inventory path: CU → refresh → liquidate_v2 → optional swaps.
pub fn build_inventory_tx(params: &KaminoTxBuildParams, swaps: &[LabeledIx]) -> Vec<LabeledIx> {
    let mut ixs = vec![
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitLimit".into(),
            ix: compute_unit_limit(params.cu_limit),
        },
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitPrice".into(),
            ix: compute_unit_price(params.cu_price),
        },
    ];
    let mut reserves = params.deposit_reserves.clone();
    reserves.extend(params.borrow_reserves.iter().copied());
    ixs.extend(refresh_ixs(params.obligation, &reserves));
    ixs.push(liquidate_ix(params));
    ixs.extend(swaps.iter().cloned());
    ixs
}

/// Flash path: CU → refresh → flash_borrow → liquidate_v2 → swaps → flash_repay.
pub fn build_flash_tx(params: &KaminoTxBuildParams, swaps: &[LabeledIx]) -> Option<Vec<LabeledIx>> {
    let (borrow_acc, repay_acc) = params.flash.as_ref()?;
    let mut ixs = vec![
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitLimit".into(),
            ix: compute_unit_limit(params.cu_limit),
        },
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitPrice".into(),
            ix: compute_unit_price(params.cu_price),
        },
    ];
    let mut reserves = params.deposit_reserves.clone();
    reserves.extend(params.borrow_reserves.iter().copied());
    ixs.extend(refresh_ixs(params.obligation, &reserves));
    let borrow_idx = ixs.len() as u8;
    ixs.push(LabeledIx {
        label: "flash_borrow_reserve_liquidity".into(),
        ix: borrow_acc.build_ix(params.liquidity_amount),
    });
    ixs.push(liquidate_ix(params));
    ixs.extend(swaps.iter().cloned());
    ixs.push(LabeledIx {
        label: "flash_repay_reserve_liquidity".into(),
        ix: repay_acc.build_ix(params.liquidity_amount, borrow_idx),
    });
    Some(ixs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::LiquidateV2Accounts;
    use crate::disc;

    fn sample_liq() -> LiquidateV2Accounts {
        LiquidateV2Accounts {
            liquidator: Pubkey::test(1, 1),
            obligation: Pubkey::test(1, 2),
            lending_market: Pubkey::test(1, 3),
            lending_market_authority: Pubkey::test(1, 4),
            repay_reserve: Pubkey::test(1, 5),
            repay_reserve_liquidity_mint: Pubkey::test(1, 6),
            repay_reserve_liquidity_supply: Pubkey::test(1, 7),
            withdraw_reserve: Pubkey::test(1, 8),
            withdraw_reserve_liquidity_mint: Pubkey::test(1, 9),
            withdraw_reserve_collateral_mint: Pubkey::test(1, 10),
            withdraw_reserve_collateral_supply: Pubkey::test(1, 11),
            withdraw_reserve_liquidity_supply: Pubkey::test(1, 12),
            withdraw_reserve_liquidity_fee_receiver: Pubkey::test(1, 13),
            user_source_liquidity: Pubkey::test(1, 14),
            user_destination_collateral: Pubkey::test(1, 15),
            user_destination_liquidity: Pubkey::test(1, 16),
            collateral_token_program: programs::token(),
            repay_liquidity_token_program: programs::token(),
            withdraw_liquidity_token_program: programs::token(),
            instruction_sysvar_account: programs::sysvar_instructions(),
            collateral_obligation_farm_user_state: None,
            collateral_reserve_farm_state: None,
            debt_obligation_farm_user_state: None,
            debt_reserve_farm_state: None,
            farms_program: Pubkey::test(1, 21),
            deposit_reserves: vec![],
        }
    }

    #[test]
    fn inventory_builder_emits_nonempty_data_and_metas() {
        let params = KaminoTxBuildParams {
            obligation: Pubkey::test(1, 2),
            deposit_reserves: vec![Pubkey::test(1, 8)],
            borrow_reserves: vec![Pubkey::test(1, 5)],
            liquidate: sample_liq(),
            liquidity_amount: 1_000,
            min_acceptable_received: 0,
            max_allowed_ltv_override_percent: 0,
            cu_limit: 400_000,
            cu_price: 1000,
            flash: None,
        };
        let ixs = build_inventory_tx(&params, &[]);
        assert!(ixs.len() >= 5);
        let liq = ixs
            .iter()
            .find(|l| l.label.contains("liquidate"))
            .unwrap();
        assert!(liq.ix.data.len() >= 8);
        assert_eq!(&liq.ix.data[..8], &disc::LIQUIDATE_V2);
        assert!(!liq.ix.accounts.is_empty());
    }
}

#[cfg(test)]
mod flash_builder_tests {
    use super::*;
    use crate::accounts::LiquidateV2Accounts;
    use crate::flash::{FlashBorrowAccounts, FlashRepayAccounts};

    fn sample_liq() -> LiquidateV2Accounts {
        LiquidateV2Accounts {
            liquidator: Pubkey::test(1, 1),
            obligation: Pubkey::test(1, 2),
            lending_market: Pubkey::test(1, 3),
            lending_market_authority: Pubkey::test(1, 4),
            repay_reserve: Pubkey::test(1, 5),
            repay_reserve_liquidity_mint: Pubkey::test(1, 6),
            repay_reserve_liquidity_supply: Pubkey::test(1, 7),
            withdraw_reserve: Pubkey::test(1, 8),
            withdraw_reserve_liquidity_mint: Pubkey::test(1, 9),
            withdraw_reserve_collateral_mint: Pubkey::test(1, 10),
            withdraw_reserve_collateral_supply: Pubkey::test(1, 11),
            withdraw_reserve_liquidity_supply: Pubkey::test(1, 12),
            withdraw_reserve_liquidity_fee_receiver: Pubkey::test(1, 13),
            user_source_liquidity: Pubkey::test(1, 14),
            user_destination_collateral: Pubkey::test(1, 15),
            user_destination_liquidity: Pubkey::test(1, 16),
            collateral_token_program: programs::token(),
            repay_liquidity_token_program: programs::token(),
            withdraw_liquidity_token_program: programs::token(),
            instruction_sysvar_account: programs::sysvar_instructions(),
            collateral_obligation_farm_user_state: None,
            collateral_reserve_farm_state: None,
            debt_obligation_farm_user_state: None,
            debt_reserve_farm_state: None,
            farms_program: Pubkey::test(1, 21),
            deposit_reserves: vec![],
        }
    }

    fn sample_flash() -> (FlashBorrowAccounts, FlashRepayAccounts) {
        let borrow = FlashBorrowAccounts {
            user_transfer_authority: Pubkey::test(3, 1),
            lending_market_authority: Pubkey::test(1, 4),
            lending_market: Pubkey::test(1, 3),
            reserve: Pubkey::test(1, 5),
            reserve_liquidity_mint: Pubkey::test(1, 6),
            reserve_source_liquidity: Pubkey::test(1, 7),
            user_destination_liquidity: Pubkey::test(1, 14),
            reserve_liquidity_fee_receiver: Pubkey::test(1, 13),
            referrer_token_state: None,
            referrer_account: None,
        };
        let repay = FlashRepayAccounts {
            user_transfer_authority: Pubkey::test(3, 1),
            lending_market_authority: Pubkey::test(1, 4),
            lending_market: Pubkey::test(1, 3),
            reserve: Pubkey::test(1, 5),
            reserve_liquidity_mint: Pubkey::test(1, 6),
            reserve_destination_liquidity: Pubkey::test(1, 7),
            user_source_liquidity: Pubkey::test(1, 14),
            reserve_liquidity_fee_receiver: Pubkey::test(1, 13),
            referrer_token_state: None,
            referrer_account: None,
        };
        (borrow, repay)
    }

    #[test]
    fn flash_tx_emits_borrow_and_repay_with_absent_referrer() {
        let (b, r) = sample_flash();
        let params = KaminoTxBuildParams {
            obligation: Pubkey::test(1, 2),
            deposit_reserves: vec![Pubkey::test(1, 8)],
            borrow_reserves: vec![Pubkey::test(1, 5)],
            liquidate: sample_liq(),
            liquidity_amount: 1_000,
            min_acceptable_received: 0,
            max_allowed_ltv_override_percent: 0,
            cu_limit: 400_000,
            cu_price: 1000,
            flash: Some((b, r)),
        };
        let ixs = build_flash_tx(&params, &[]).expect("flash");
        let labels: Vec<_> = ixs.iter().map(|l| l.label.as_str()).collect();
        assert!(labels.contains(&"flash_borrow_reserve_liquidity"));
        assert!(labels.contains(&"flash_repay_reserve_liquidity"));
        let borrow = ixs.iter().find(|l| l.label.contains("flash_borrow")).unwrap();
        let ref_meta = &borrow.ix.accounts[8];
        assert_eq!(ref_meta.pubkey, programs::klend());
        assert!(!ref_meta.is_writable);
    }
}
