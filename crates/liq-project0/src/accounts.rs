//! Account meta name lists for classic + receivership builders.
//! Keys are filled by the runtime; this module only describes roles/order.

use liq_core::Pubkey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetaRole {
    Writable,
    Readonly,
    Signer,
    WritableSigner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedMeta {
    pub name: &'static str,
    pub key: Pubkey,
    pub role: MetaRole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassicLiquidateAccounts {
    pub group: Pubkey,
    pub asset_bank: Pubkey,
    pub liab_bank: Pubkey,
    pub liquidator_marginfi_account: Pubkey,
    pub authority: Pubkey,
    pub liquidatee_marginfi_account: Pubkey,
    pub bank_liquidity_vault_authority: Pubkey,
    pub bank_liquidity_vault: Pubkey,
    pub bank_insurance_vault: Pubkey,
    pub token_program: Pubkey,
    /// Remaining: oracles + observation accounts.
    pub remaining: Vec<Pubkey>,
}

impl ClassicLiquidateAccounts {
    pub fn metas(&self) -> Vec<NamedMeta> {
        let mut v = vec![
            NamedMeta { name: "group", key: self.group, role: MetaRole::Readonly },
            NamedMeta { name: "asset_bank", key: self.asset_bank, role: MetaRole::Writable },
            NamedMeta { name: "liab_bank", key: self.liab_bank, role: MetaRole::Writable },
            NamedMeta {
                name: "liquidator_marginfi_account",
                key: self.liquidator_marginfi_account,
                role: MetaRole::Writable,
            },
            NamedMeta { name: "authority", key: self.authority, role: MetaRole::Signer },
            NamedMeta {
                name: "liquidatee_marginfi_account",
                key: self.liquidatee_marginfi_account,
                role: MetaRole::Writable,
            },
            NamedMeta {
                name: "bank_liquidity_vault_authority",
                key: self.bank_liquidity_vault_authority,
                role: MetaRole::Readonly,
            },
            NamedMeta {
                name: "bank_liquidity_vault",
                key: self.bank_liquidity_vault,
                role: MetaRole::Writable,
            },
            NamedMeta {
                name: "bank_insurance_vault",
                key: self.bank_insurance_vault,
                role: MetaRole::Writable,
            },
            NamedMeta {
                name: "token_program",
                key: self.token_program,
                role: MetaRole::Readonly,
            },
        ];
        for (i, k) in self.remaining.iter().enumerate() {
            v.push(NamedMeta {
                name: if i == 0 { "remaining" } else { "remaining" },
                key: *k,
                role: MetaRole::Readonly,
            });
        }
        v
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartLiquidationAccounts {
    pub marginfi_account: Pubkey,
    pub liquidation_record: Pubkey,
    pub group: Pubkey,
    pub liquidation_receiver: Pubkey,
    pub instruction_sysvar: Pubkey,
    /// Banks/oracles must be writable remaining accounts.
    pub remaining_writable: Vec<Pubkey>,
}

impl StartLiquidationAccounts {
    pub fn metas(&self) -> Vec<NamedMeta> {
        let mut v = vec![
            NamedMeta {
                name: "marginfi_account",
                key: self.marginfi_account,
                role: MetaRole::Writable,
            },
            NamedMeta {
                name: "liquidation_record",
                key: self.liquidation_record,
                role: MetaRole::Writable,
            },
            NamedMeta { name: "group", key: self.group, role: MetaRole::Readonly },
            NamedMeta {
                name: "liquidation_receiver",
                key: self.liquidation_receiver,
                role: MetaRole::Readonly,
            },
            NamedMeta {
                name: "instruction_sysvar",
                key: self.instruction_sysvar,
                role: MetaRole::Readonly,
            },
        ];
        for k in &self.remaining_writable {
            v.push(NamedMeta {
                name: "bank_or_oracle",
                key: *k,
                role: MetaRole::Writable,
            });
        }
        v
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndLiquidationAccounts {
    pub marginfi_account: Pubkey,
    pub liquidation_record: Pubkey,
    pub group: Pubkey,
    pub liquidation_receiver: Pubkey,
    pub fee_state: Pubkey,
    pub global_fee_wallet: Pubkey,
    pub system_program: Pubkey,
    pub fee_payer: Option<Pubkey>,
}

impl EndLiquidationAccounts {
    pub fn metas(&self) -> Vec<NamedMeta> {
        let mut v = vec![
            NamedMeta {
                name: "marginfi_account",
                key: self.marginfi_account,
                role: MetaRole::Writable,
            },
            NamedMeta {
                name: "liquidation_record",
                key: self.liquidation_record,
                role: MetaRole::Writable,
            },
            NamedMeta { name: "group", key: self.group, role: MetaRole::Readonly },
            NamedMeta {
                name: "liquidation_receiver",
                key: self.liquidation_receiver,
                role: MetaRole::WritableSigner,
            },
            NamedMeta { name: "fee_state", key: self.fee_state, role: MetaRole::Readonly },
            NamedMeta {
                name: "global_fee_wallet",
                key: self.global_fee_wallet,
                role: MetaRole::Writable,
            },
            NamedMeta {
                name: "system_program",
                key: self.system_program,
                role: MetaRole::Readonly,
            },
        ];
        if let Some(fp) = self.fee_payer {
            v.push(NamedMeta {
                name: "fee_payer",
                key: fp,
                role: MetaRole::WritableSigner,
            });
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_meta_count() {
        let a = ClassicLiquidateAccounts {
            group: Pubkey::test(1, 1),
            asset_bank: Pubkey::test(1, 2),
            liab_bank: Pubkey::test(1, 3),
            liquidator_marginfi_account: Pubkey::test(1, 4),
            authority: Pubkey::test(1, 5),
            liquidatee_marginfi_account: Pubkey::test(1, 6),
            bank_liquidity_vault_authority: Pubkey::test(1, 7),
            bank_liquidity_vault: Pubkey::test(1, 8),
            bank_insurance_vault: Pubkey::test(1, 9),
            token_program: Pubkey::test(1, 10),
            remaining: vec![Pubkey::test(1, 11)],
        };
        assert_eq!(a.metas().len(), 11);
    }

    #[test]
    fn start_end_meta_roles() {
        let s = StartLiquidationAccounts {
            marginfi_account: Pubkey::test(2, 1),
            liquidation_record: Pubkey::test(2, 2),
            group: Pubkey::test(2, 3),
            liquidation_receiver: Pubkey::test(2, 4),
            instruction_sysvar: Pubkey::test(2, 5),
            remaining_writable: vec![],
        };
        assert_eq!(s.metas().len(), 5);
        let e = EndLiquidationAccounts {
            marginfi_account: Pubkey::test(2, 1),
            liquidation_record: Pubkey::test(2, 2),
            group: Pubkey::test(2, 3),
            liquidation_receiver: Pubkey::test(2, 4),
            fee_state: Pubkey::test(2, 6),
            global_fee_wallet: Pubkey::test(2, 7),
            system_program: Pubkey::test(2, 8),
            fee_payer: None,
        };
        assert_eq!(e.metas().len(), 7);
    }
}
