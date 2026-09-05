//! Strategy → protocol tx builders. Accounts come from structured inputs
//! (fixtures/config), never hardcoded inside flash builders.

use liq_core::{
    programs, FundingStrategy, LabeledIx, Protocol, Pubkey,
};
use liq_kamino::{
    build_flash_tx, build_inventory_tx, FlashBorrowAccounts, FlashRepayAccounts,
    KaminoTxBuildParams, LiquidateV2Accounts,
};
use liq_routing::JupiterQuoteBlob;
use liq_save::{
    build_flash_atomic_plan, FlashBorrowAccounts as SaveFlashBorrow,
    FlashRepayAccounts as SaveFlashRepay, SaveFlashPlanAccounts, SaveLiquidateAccounts,
};

/// Structured account set for building liquidation txs in shadow/fixture mode.
/// Real mainnet keys should be loaded from config/RPC decode; fixture path uses
/// deterministic `Pubkey::test` **outside** the builders.
#[derive(Debug, Clone)]
pub struct PlanAccountSet {
    pub liquidator: Pubkey,
    pub obligation: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub repay_reserve: Pubkey,
    pub withdraw_reserve: Pubkey,
    pub user_liquidity: Pubkey,
    pub user_collateral: Pubkey,
}

impl PlanAccountSet {
    /// Fixture-shaped accounts derived from a borrower pubkey (tag/index stable).
    pub fn from_seed(seed: Pubkey) -> Self {
        let tag = seed.0[0];
        let base = u64::from_le_bytes(seed.0[24..32].try_into().unwrap_or([0; 8]));
        let pk = |i| Pubkey::test(tag.wrapping_add(20), base.wrapping_add(i));
        Self {
            liquidator: pk(1),
            obligation: seed,
            lending_market: pk(3),
            lending_market_authority: pk(4),
            repay_reserve: pk(5),
            withdraw_reserve: pk(6),
            user_liquidity: pk(7),
            user_collateral: pk(8),
        }
    }
}

fn kamino_liquidate_accounts(a: &PlanAccountSet) -> LiquidateV2Accounts {
    LiquidateV2Accounts {
        liquidator: a.liquidator,
        obligation: a.obligation,
        lending_market: a.lending_market,
        lending_market_authority: a.lending_market_authority,
        repay_reserve: a.repay_reserve,
        repay_reserve_liquidity_mint: Pubkey::test(a.repay_reserve.0[0], 100),
        repay_reserve_liquidity_supply: Pubkey::test(a.repay_reserve.0[0], 101),
        withdraw_reserve: a.withdraw_reserve,
        withdraw_reserve_liquidity_mint: Pubkey::test(a.withdraw_reserve.0[0], 102),
        withdraw_reserve_collateral_mint: Pubkey::test(a.withdraw_reserve.0[0], 103),
        withdraw_reserve_collateral_supply: Pubkey::test(a.withdraw_reserve.0[0], 104),
        withdraw_reserve_liquidity_supply: Pubkey::test(a.withdraw_reserve.0[0], 105),
        withdraw_reserve_liquidity_fee_receiver: Pubkey::test(a.withdraw_reserve.0[0], 106),
        user_source_liquidity: a.user_liquidity,
        user_destination_collateral: a.user_collateral,
        user_destination_liquidity: a.user_liquidity,
        collateral_token_program: programs::token(),
        repay_liquidity_token_program: programs::token(),
        withdraw_liquidity_token_program: programs::token(),
        instruction_sysvar_account: programs::sysvar_instructions(),
        collateral_obligation_farm_user_state: None,
        collateral_reserve_farm_state: None,
        debt_obligation_farm_user_state: None,
        debt_reserve_farm_state: None,
        farms_program: Pubkey::test(90, 1),
        deposit_reserves: vec![],
    }
}

fn kamino_flash_pair(a: &PlanAccountSet) -> (FlashBorrowAccounts, FlashRepayAccounts) {
    let borrow = FlashBorrowAccounts {
        user_transfer_authority: a.liquidator,
        lending_market_authority: a.lending_market_authority,
        lending_market: a.lending_market,
        reserve: a.repay_reserve,
        reserve_liquidity_mint: Pubkey::test(a.repay_reserve.0[0], 100),
        reserve_source_liquidity: Pubkey::test(a.repay_reserve.0[0], 101),
        user_destination_liquidity: a.user_liquidity,
        reserve_liquidity_fee_receiver: Pubkey::test(a.withdraw_reserve.0[0], 106),
        referrer_token_state: None, // → KLend program id readonly
        referrer_account: None,
    };
    let repay = FlashRepayAccounts {
        user_transfer_authority: a.liquidator,
        lending_market_authority: a.lending_market_authority,
        lending_market: a.lending_market,
        reserve: a.repay_reserve,
        reserve_liquidity_mint: Pubkey::test(a.repay_reserve.0[0], 100),
        reserve_destination_liquidity: Pubkey::test(a.repay_reserve.0[0], 101),
        user_source_liquidity: a.user_liquidity,
        reserve_liquidity_fee_receiver: Pubkey::test(a.withdraw_reserve.0[0], 106),
        referrer_token_state: None,
        referrer_account: None,
    };
    (borrow, repay)
}

fn save_flash_accounts(a: &PlanAccountSet) -> SaveFlashPlanAccounts {
    SaveFlashPlanAccounts {
        flash_borrow: SaveFlashBorrow {
            source_liquidity: Pubkey::test(a.repay_reserve.0[0], 101),
            destination_liquidity: a.user_liquidity,
            reserve: a.repay_reserve,
            lending_market: a.lending_market,
            lending_market_authority: a.lending_market_authority,
        },
        liquidate: SaveLiquidateAccounts {
            source_liquidity: a.user_liquidity,
            destination_collateral: a.user_collateral,
            destination_liquidity: a.user_liquidity,
            repay_reserve: a.repay_reserve,
            repay_reserve_liquidity_supply: Pubkey::test(a.repay_reserve.0[0], 101),
            withdraw_reserve: a.withdraw_reserve,
            withdraw_reserve_collateral_mint: Pubkey::test(a.withdraw_reserve.0[0], 103),
            withdraw_reserve_collateral_supply: Pubkey::test(a.withdraw_reserve.0[0], 104),
            withdraw_reserve_liquidity_supply: Pubkey::test(a.withdraw_reserve.0[0], 105),
            withdraw_reserve_fee_receiver: Pubkey::test(a.withdraw_reserve.0[0], 106),
            obligation: a.obligation,
            lending_market: a.lending_market,
            lending_market_authority: a.lending_market_authority,
            user_transfer_authority: a.liquidator,
        },
        flash_repay: SaveFlashRepay {
            source_liquidity: a.user_liquidity,
            destination_liquidity: Pubkey::test(a.repay_reserve.0[0], 101),
            fee_receiver: Pubkey::test(a.withdraw_reserve.0[0], 106),
            host_fee_receiver: Pubkey::test(a.withdraw_reserve.0[0], 107),
            reserve: a.repay_reserve,
            lending_market: a.lending_market,
            user_transfer_authority: a.liquidator,
        },
        refresh_reserves: vec![a.repay_reserve, a.withdraw_reserve],
        obligation: a.obligation,
    }
}

/// Result of planning wire ixs for a funding strategy.
#[derive(Debug, Clone)]
pub struct PlannedIxs {
    pub labeled: Vec<LabeledIx>,
    pub swap_incomplete: bool,
    pub used_flash_builder: bool,
}

/// Build protocol-exact instruction lists for the chosen funding strategy.
pub fn build_strategy_ixs(
    protocol: Protocol,
    strategy: FundingStrategy,
    accounts: &PlanAccountSet,
    liquidity_amount: u64,
    swap_blob: &JupiterQuoteBlob,
) -> PlannedIxs {
    let (swaps, swap_incomplete) = swap_blob.attach_or_omit();
    match (protocol, strategy) {
        (Protocol::Kamino, FundingStrategy::KaminoFlashBorrow) => {
            let (borrow, repay) = kamino_flash_pair(accounts);
            let params = KaminoTxBuildParams {
                obligation: accounts.obligation,
                deposit_reserves: vec![accounts.withdraw_reserve],
                borrow_reserves: vec![accounts.repay_reserve],
                liquidate: kamino_liquidate_accounts(accounts),
                liquidity_amount,
                min_acceptable_received: 0,
                max_allowed_ltv_override_percent: 0,
                cu_limit: 400_000,
                cu_price: 1_000,
                flash: Some((borrow, repay)),
            };
            let labeled = build_flash_tx(&params, &swaps).unwrap_or_default();
            PlannedIxs {
                labeled,
                swap_incomplete,
                used_flash_builder: true,
            }
        }
        (Protocol::Kamino, FundingStrategy::Inventory) => {
            let params = KaminoTxBuildParams {
                obligation: accounts.obligation,
                deposit_reserves: vec![accounts.withdraw_reserve],
                borrow_reserves: vec![accounts.repay_reserve],
                liquidate: kamino_liquidate_accounts(accounts),
                liquidity_amount,
                min_acceptable_received: 0,
                max_allowed_ltv_override_percent: 0,
                cu_limit: 400_000,
                cu_price: 1_000,
                flash: None,
            };
            PlannedIxs {
                labeled: build_inventory_tx(&params, &swaps),
                swap_incomplete,
                used_flash_builder: false,
            }
        }
        (Protocol::Save, FundingStrategy::SaveFlashLoan) => {
            let plan = build_flash_atomic_plan(
                &save_flash_accounts(accounts),
                liquidity_amount,
                &swaps,
                400_000,
                1_000,
            );
            PlannedIxs {
                labeled: plan.labeled,
                swap_incomplete,
                used_flash_builder: true,
            }
        }
        (Protocol::Project0, FundingStrategy::Project0Receivership) => {
            use liq_project0::*;
            let a = accounts;
            let params = ReceivershipBuildParams {
                start: StartLiquidationAccounts {
                    marginfi_account: a.obligation,
                    liquidation_record: Pubkey::test(a.obligation.0[0], 200),
                    group: a.lending_market,
                    liquidation_receiver: a.liquidator,
                    instruction_sysvar: programs::sysvar_instructions(),
                    remaining_writable: vec![a.repay_reserve],
                },
                withdraw: WithdrawAccounts {
                    group: a.lending_market,
                    marginfi_account: a.liquidator,
                    authority: a.liquidator,
                    bank: a.withdraw_reserve,
                    vault: Pubkey::test(a.withdraw_reserve.0[0], 201),
                    destination: a.user_liquidity,
                    bank_liquidity_vault_authority: a.lending_market_authority,
                    token_program: programs::token(),
                },
                repay: RepayAccounts {
                    group: a.lending_market,
                    marginfi_account: a.liquidator,
                    authority: a.liquidator,
                    bank: a.repay_reserve,
                    signer_token_account: a.user_liquidity,
                    vault: Pubkey::test(a.repay_reserve.0[0], 202),
                    token_program: programs::token(),
                },
                end: EndLiquidationAccounts {
                    marginfi_account: a.obligation,
                    liquidation_record: Pubkey::test(a.obligation.0[0], 200),
                    group: a.lending_market,
                    liquidation_receiver: a.liquidator,
                    fee_state: Pubkey::test(80, 1),
                    global_fee_wallet: Pubkey::test(80, 2),
                    system_program: programs::system(),
                    fee_payer: None,
                },
                withdraw_amount: liquidity_amount,
                repay_amount: liquidity_amount.saturating_mul(9) / 10,
                cu_limit: 500_000,
                cu_price: 1000,
            };
            PlannedIxs {
                labeled: build_receivership_tx(&params, &swaps),
                swap_incomplete,
                used_flash_builder: false,
            }
        }
        _ => PlannedIxs {
            labeled: vec![LabeledIx {
                label: "ComputeBudget:SetComputeUnitLimit".into(),
                ix: liq_core::compute_unit_limit(200_000),
            }],
            swap_incomplete,
            used_flash_builder: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_selects_kamino_flash_builder() {
        let accounts = PlanAccountSet::from_seed(Pubkey::test(1, 1));
        let planned = build_strategy_ixs(
            Protocol::Kamino,
            FundingStrategy::KaminoFlashBorrow,
            &accounts,
            1_000_000,
            &JupiterQuoteBlob::default(),
        );
        assert!(planned.used_flash_builder);
        assert!(planned.swap_incomplete);
        let labels: Vec<_> = planned.labeled.iter().map(|l| l.label.as_str()).collect();
        assert!(labels.iter().any(|l| l.contains("flash_borrow")));
        assert!(labels.iter().any(|l| l.contains("flash_repay")));
        let borrow = planned
            .labeled
            .iter()
            .find(|l| l.label.contains("flash_borrow"))
            .unwrap();
        // absent referrer → KLend program id readonly
        assert_eq!(borrow.ix.accounts[8].pubkey, programs::klend());
        assert!(!borrow.ix.accounts[8].is_writable);
    }

    #[test]
    fn planner_kamino_inventory_skips_flash() {
        let accounts = PlanAccountSet::from_seed(Pubkey::test(1, 2));
        let planned = build_strategy_ixs(
            Protocol::Kamino,
            FundingStrategy::Inventory,
            &accounts,
            1_000,
            &JupiterQuoteBlob::default(),
        );
        assert!(!planned.used_flash_builder);
        assert!(!planned
            .labeled
            .iter()
            .any(|l| l.label.contains("flash_borrow")));
    }

    #[test]
    fn planner_save_flash_uses_builder() {
        let accounts = PlanAccountSet::from_seed(Pubkey::test(7, 1));
        let planned = build_strategy_ixs(
            Protocol::Save,
            FundingStrategy::SaveFlashLoan,
            &accounts,
            1_000_000,
            &JupiterQuoteBlob::default(),
        );
        assert!(planned.used_flash_builder);
        assert!(planned
            .labeled
            .iter()
            .any(|l| l.label == "FlashBorrowReserveLiquidity"));
    }
}
