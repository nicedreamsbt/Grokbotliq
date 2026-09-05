//! Wire-ready Project 0 classic + receivership instruction sequences.

use crate::accounts::{
    ClassicLiquidateAccounts, EndLiquidationAccounts, MetaRole, NamedMeta, StartLiquidationAccounts,
};
use crate::{
    encode_classic_liquidate, encode_end_liquidation, encode_repay, encode_start_liquidation,
    encode_withdraw,
};
use liq_core::{
    compute_unit_limit, compute_unit_price, programs, AccountMeta, Instruction, LabeledIx, Pubkey,
};
use serde::{Deserialize, Serialize};

fn named_to_meta(n: NamedMeta) -> AccountMeta {
    match n.role {
        MetaRole::Writable => AccountMeta::new(n.key, false),
        MetaRole::Readonly => AccountMeta::new_readonly(n.key, false),
        MetaRole::Signer => AccountMeta::new_readonly(n.key, true),
        MetaRole::WritableSigner => AccountMeta::new(n.key, true),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawAccounts {
    pub group: Pubkey,
    pub marginfi_account: Pubkey,
    pub authority: Pubkey,
    pub bank: Pubkey,
    pub vault: Pubkey,
    pub destination: Pubkey,
    pub bank_liquidity_vault_authority: Pubkey,
    pub token_program: Pubkey,
}

impl WithdrawAccounts {
    pub fn metas(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new_readonly(self.group, false),
            AccountMeta::new(self.marginfi_account, false),
            AccountMeta::new_readonly(self.authority, true),
            AccountMeta::new(self.bank, false),
            AccountMeta::new(self.destination, false),
            AccountMeta::new_readonly(self.bank_liquidity_vault_authority, false),
            AccountMeta::new(self.vault, false),
            AccountMeta::new_readonly(self.token_program, false),
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepayAccounts {
    pub group: Pubkey,
    pub marginfi_account: Pubkey,
    pub authority: Pubkey,
    pub bank: Pubkey,
    pub signer_token_account: Pubkey,
    pub vault: Pubkey,
    pub token_program: Pubkey,
}

impl RepayAccounts {
    pub fn metas(&self) -> Vec<AccountMeta> {
        vec![
            AccountMeta::new_readonly(self.group, false),
            AccountMeta::new(self.marginfi_account, false),
            AccountMeta::new_readonly(self.authority, true),
            AccountMeta::new(self.bank, false),
            AccountMeta::new(self.signer_token_account, false),
            AccountMeta::new(self.vault, false),
            AccountMeta::new_readonly(self.token_program, false),
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivershipBuildParams {
    pub start: StartLiquidationAccounts,
    pub withdraw: WithdrawAccounts,
    pub repay: RepayAccounts,
    pub end: EndLiquidationAccounts,
    pub withdraw_amount: u64,
    pub repay_amount: u64,
    pub cu_limit: u32,
    pub cu_price: u64,
}

/// Full receivership sequence producing wire-ready Instruction lists.
/// Order: CU → start_liquidation → withdraw → [swaps] → repay → end_liquidation.
pub fn build_receivership_tx(
    params: &ReceivershipBuildParams,
    swaps: &[LabeledIx],
) -> Vec<LabeledIx> {
    let mut ixs = vec![
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitLimit".into(),
            ix: compute_unit_limit(params.cu_limit),
        },
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitPrice".into(),
            ix: compute_unit_price(params.cu_price),
        },
        LabeledIx {
            label: "start_liquidation".into(),
            ix: Instruction::new(
                programs::marginfi(),
                params.start.metas().into_iter().map(named_to_meta).collect(),
                encode_start_liquidation(),
            ),
        },
        LabeledIx {
            label: "lending_account_withdraw".into(),
            ix: Instruction::new(
                programs::marginfi(),
                params.withdraw.metas(),
                encode_withdraw(params.withdraw_amount, false),
            ),
        },
    ];
    ixs.extend(swaps.iter().cloned());
    ixs.push(LabeledIx {
        label: "lending_account_repay".into(),
        ix: Instruction::new(
            programs::marginfi(),
            params.repay.metas(),
            encode_repay(params.repay_amount, false),
        ),
    });
    ixs.push(LabeledIx {
        label: "end_liquidation".into(),
        ix: Instruction::new(
            programs::marginfi(),
            params.end.metas().into_iter().map(named_to_meta).collect(),
            encode_end_liquidation(),
        ),
    });
    ixs
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassicBuildParams {
    pub accounts: ClassicLiquidateAccounts,
    pub asset_amount: u64,
    pub cu_limit: u32,
    pub cu_price: u64,
}

pub fn build_classic_tx(params: &ClassicBuildParams) -> Vec<LabeledIx> {
    vec![
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitLimit".into(),
            ix: compute_unit_limit(params.cu_limit),
        },
        LabeledIx {
            label: "ComputeBudget:SetComputeUnitPrice".into(),
            ix: compute_unit_price(params.cu_price),
        },
        LabeledIx {
            label: "lending_account_liquidate".into(),
            ix: Instruction::new(
                programs::marginfi(),
                params
                    .accounts
                    .metas()
                    .into_iter()
                    .map(named_to_meta)
                    .collect(),
                encode_classic_liquidate(params.asset_amount),
            ),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc;

    fn sample_recv() -> ReceivershipBuildParams {
        let pk = |i| Pubkey::test(8, i);
        ReceivershipBuildParams {
            start: StartLiquidationAccounts {
                marginfi_account: pk(1),
                liquidation_record: pk(2),
                group: pk(3),
                liquidation_receiver: pk(4),
                instruction_sysvar: programs::sysvar_instructions(),
                remaining_writable: vec![pk(5), pk(6)],
            },
            withdraw: WithdrawAccounts {
                group: pk(3),
                marginfi_account: pk(4), // receiver account during receivership
                authority: pk(4),
                bank: pk(5),
                vault: pk(7),
                destination: pk(8),
                bank_liquidity_vault_authority: pk(9),
                token_program: programs::token(),
            },
            repay: RepayAccounts {
                group: pk(3),
                marginfi_account: pk(4),
                authority: pk(4),
                bank: pk(6),
                signer_token_account: pk(10),
                vault: pk(11),
                token_program: programs::token(),
            },
            end: EndLiquidationAccounts {
                marginfi_account: pk(1),
                liquidation_record: pk(2),
                group: pk(3),
                liquidation_receiver: pk(4),
                fee_state: pk(12),
                global_fee_wallet: pk(13),
                system_program: programs::system(),
                fee_payer: None,
            },
            withdraw_amount: 1_000,
            repay_amount: 900,
            cu_limit: 500_000,
            cu_price: 2000,
        }
    }

    #[test]
    fn receivership_sequence_order_and_wire_bytes() {
        let ixs = build_receivership_tx(&sample_recv(), &[]);
        let labels: Vec<_> = ixs.iter().map(|l| l.label.as_str()).collect();
        assert!(labels.contains(&"start_liquidation"));
        assert!(labels.contains(&"lending_account_withdraw"));
        assert!(labels.contains(&"lending_account_repay"));
        assert!(labels.contains(&"end_liquidation"));
        let start = labels.iter().position(|l| *l == "start_liquidation").unwrap();
        let end = labels.iter().position(|l| *l == "end_liquidation").unwrap();
        assert!(start < end);
        assert_eq!(&ixs[start].ix.data[..8], &disc::START_LIQUIDATION);
        assert_eq!(&ixs[end].ix.data[..8], &disc::END_LIQUIDATION);
        assert!(!ixs[start].ix.accounts.is_empty());
        for ix in &ixs {
            assert!(ix.ix.is_wire_ready() || ix.label.starts_with("ComputeBudget"));
            assert!(!ix.ix.data.is_empty());
        }
    }

    #[test]
    fn classic_emits_liquidate_data() {
        let params = ClassicBuildParams {
            accounts: ClassicLiquidateAccounts {
                group: Pubkey::test(1, 1),
                asset_bank: Pubkey::test(1, 2),
                liab_bank: Pubkey::test(1, 3),
                liquidator_marginfi_account: Pubkey::test(1, 4),
                authority: Pubkey::test(1, 5),
                liquidatee_marginfi_account: Pubkey::test(1, 6),
                bank_liquidity_vault_authority: Pubkey::test(1, 7),
                bank_liquidity_vault: Pubkey::test(1, 8),
                bank_insurance_vault: Pubkey::test(1, 9),
                token_program: programs::token(),
                remaining: vec![],
            },
            asset_amount: 55,
            cu_limit: 200_000,
            cu_price: 1,
        };
        let ixs = build_classic_tx(&params);
        let liq = &ixs[2];
        assert_eq!(&liq.ix.data[..8], &disc::LENDING_ACCOUNT_LIQUIDATE);
        assert_eq!(&liq.ix.data[8..16], &55u64.to_le_bytes());
        assert!(!liq.ix.accounts.is_empty());
    }
}
