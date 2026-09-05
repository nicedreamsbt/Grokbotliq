//! Liquidate v2 account metas from klend IDL (`liquidateObligationAndRedeemReserveCollateralV2`).

use liq_core::Pubkey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetaRole {
    Writable,
    Readonly,
    Signer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountMetaDesc {
    pub name: &'static str,
    pub key: Pubkey,
    pub role: MetaRole,
}

/// Flat account list matching IDL liquidationAccounts + farms + farmsProgram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidateV2Accounts {
    pub liquidator: Pubkey,
    pub obligation: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub repay_reserve: Pubkey,
    pub repay_reserve_liquidity_mint: Pubkey,
    pub repay_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve: Pubkey,
    pub withdraw_reserve_liquidity_mint: Pubkey,
    pub withdraw_reserve_collateral_mint: Pubkey,
    pub withdraw_reserve_collateral_supply: Pubkey,
    pub withdraw_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve_liquidity_fee_receiver: Pubkey,
    pub user_source_liquidity: Pubkey,
    pub user_destination_collateral: Pubkey,
    pub user_destination_liquidity: Pubkey,
    pub collateral_token_program: Pubkey,
    pub repay_liquidity_token_program: Pubkey,
    pub withdraw_liquidity_token_program: Pubkey,
    pub instruction_sysvar_account: Pubkey,
    /// Optional farm accounts; when absent pass program id as readonly placeholder.
    pub collateral_obligation_farm_user_state: Option<Pubkey>,
    pub collateral_reserve_farm_state: Option<Pubkey>,
    pub debt_obligation_farm_user_state: Option<Pubkey>,
    pub debt_reserve_farm_state: Option<Pubkey>,
    pub farms_program: Pubkey,
    /// Remaining deposit reserves for health checks.
    pub deposit_reserves: Vec<Pubkey>,
}

impl LiquidateV2Accounts {
    pub fn metas(&self) -> Vec<AccountMetaDesc> {
        let placeholder = self.farms_program;
        let mut v = vec![
            AccountMetaDesc { name: "liquidator", key: self.liquidator, role: MetaRole::Signer },
            AccountMetaDesc { name: "obligation", key: self.obligation, role: MetaRole::Writable },
            AccountMetaDesc { name: "lending_market", key: self.lending_market, role: MetaRole::Readonly },
            AccountMetaDesc {
                name: "lending_market_authority",
                key: self.lending_market_authority,
                role: MetaRole::Readonly,
            },
            AccountMetaDesc { name: "repay_reserve", key: self.repay_reserve, role: MetaRole::Writable },
            AccountMetaDesc {
                name: "repay_reserve_liquidity_mint",
                key: self.repay_reserve_liquidity_mint,
                role: MetaRole::Readonly,
            },
            AccountMetaDesc {
                name: "repay_reserve_liquidity_supply",
                key: self.repay_reserve_liquidity_supply,
                role: MetaRole::Writable,
            },
            AccountMetaDesc {
                name: "withdraw_reserve",
                key: self.withdraw_reserve,
                role: MetaRole::Writable,
            },
            AccountMetaDesc {
                name: "withdraw_reserve_liquidity_mint",
                key: self.withdraw_reserve_liquidity_mint,
                role: MetaRole::Readonly,
            },
            AccountMetaDesc {
                name: "withdraw_reserve_collateral_mint",
                key: self.withdraw_reserve_collateral_mint,
                role: MetaRole::Writable,
            },
            AccountMetaDesc {
                name: "withdraw_reserve_collateral_supply",
                key: self.withdraw_reserve_collateral_supply,
                role: MetaRole::Writable,
            },
            AccountMetaDesc {
                name: "withdraw_reserve_liquidity_supply",
                key: self.withdraw_reserve_liquidity_supply,
                role: MetaRole::Writable,
            },
            AccountMetaDesc {
                name: "withdraw_reserve_liquidity_fee_receiver",
                key: self.withdraw_reserve_liquidity_fee_receiver,
                role: MetaRole::Writable,
            },
            AccountMetaDesc {
                name: "user_source_liquidity",
                key: self.user_source_liquidity,
                role: MetaRole::Writable,
            },
            AccountMetaDesc {
                name: "user_destination_collateral",
                key: self.user_destination_collateral,
                role: MetaRole::Writable,
            },
            AccountMetaDesc {
                name: "user_destination_liquidity",
                key: self.user_destination_liquidity,
                role: MetaRole::Writable,
            },
            AccountMetaDesc {
                name: "collateral_token_program",
                key: self.collateral_token_program,
                role: MetaRole::Readonly,
            },
            AccountMetaDesc {
                name: "repay_liquidity_token_program",
                key: self.repay_liquidity_token_program,
                role: MetaRole::Readonly,
            },
            AccountMetaDesc {
                name: "withdraw_liquidity_token_program",
                key: self.withdraw_liquidity_token_program,
                role: MetaRole::Readonly,
            },
            AccountMetaDesc {
                name: "instruction_sysvar_account",
                key: self.instruction_sysvar_account,
                role: MetaRole::Readonly,
            },
            opt_farm("collateral_obligation_farm_user_state", self.collateral_obligation_farm_user_state, placeholder),
            opt_farm("collateral_reserve_farm_state", self.collateral_reserve_farm_state, placeholder),
            opt_farm("debt_obligation_farm_user_state", self.debt_obligation_farm_user_state, placeholder),
            opt_farm("debt_reserve_farm_state", self.debt_reserve_farm_state, placeholder),
            AccountMetaDesc { name: "farms_program", key: self.farms_program, role: MetaRole::Readonly },
        ];
        for r in &self.deposit_reserves {
            v.push(AccountMetaDesc {
                name: "deposit_reserve",
                key: *r,
                role: MetaRole::Readonly,
            });
        }
        v
    }
}

fn opt_farm(name: &'static str, key: Option<Pubkey>, placeholder: Pubkey) -> AccountMetaDesc {
    match key {
        Some(k) => AccountMetaDesc { name, key: k, role: MetaRole::Writable },
        None => AccountMetaDesc { name, key: placeholder, role: MetaRole::Readonly },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LiquidateV2Accounts {
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
            collateral_token_program: Pubkey::test(1, 17),
            repay_liquidity_token_program: Pubkey::test(1, 18),
            withdraw_liquidity_token_program: Pubkey::test(1, 19),
            instruction_sysvar_account: Pubkey::test(1, 20),
            collateral_obligation_farm_user_state: None,
            collateral_reserve_farm_state: None,
            debt_obligation_farm_user_state: None,
            debt_reserve_farm_state: None,
            farms_program: Pubkey::test(1, 21),
            deposit_reserves: vec![Pubkey::test(1, 22)],
        }
    }

    #[test]
    fn v2_meta_count_without_farms() {
        // 20 liquidation + 4 farm placeholders + farmsProgram + 1 deposit = 26
        assert_eq!(sample().metas().len(), 26);
    }
}
