//! FundingStrategy plan attachment to PreparedTx / opportunity JSON.

use liq_core::{
    FundingPathEnumerator, FundingStrategy, OpportunityPlan, Protocol, ScoredFundingPath,
};
use serde::{Deserialize, Serialize};

/// Opportunity log line emitted by liquidator in DRY_RUN (and live when gated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityJson {
    pub dry_run: bool,
    pub protocol: String,
    pub account: String,
    pub funding_strategy: String,
    pub net_profit_usd_micro: i64,
    pub expected_value_score: i64,
    pub flash_fee_usd_micro: u64,
    pub tip_usd_micro: u64,
    pub capital_used_usd_micro: u64,
    pub ix_labels: Vec<String>,
    pub accepted: bool,
    pub slot: u64,
}

pub fn opportunity_from_best(
    dry_run: bool,
    account: &str,
    plan: &OpportunityPlan,
    best: &ScoredFundingPath,
    ix_labels: Vec<String>,
    slot: u64,
) -> OpportunityJson {
    let _ = plan;
    OpportunityJson {
        dry_run,
        protocol: format!("{:?}", best.protocol),
        account: account.to_string(),
        funding_strategy: best.strategy.as_str().to_string(),
        net_profit_usd_micro: best.net_profit_usd_micro,
        expected_value_score: best.expected_value_score,
        flash_fee_usd_micro: best.flash_fee_usd_micro,
        tip_usd_micro: best.tip_usd_micro,
        capital_used_usd_micro: best.capital_used_usd_micro,
        ix_labels,
        accepted: matches!(
            best.decision,
            liq_core::ProfitDecision::Accept { .. }
        ),
        slot,
    }
}

/// Evaluate funding paths for a candidate and return the opportunity plan.
pub fn evaluate_funding(
    enumr: &FundingPathEnumerator,
    protocol: Protocol,
    gross_profit_usd_micro: i64,
    swap_cost_usd_micro: u64,
    chain_cost_usd_micro: u64,
    tip_usd_micro: u64,
    notional_usd_micro: u64,
    inventory_usd_micro: u64,
    flash_fee_bps: u64,
) -> OpportunityPlan {
    let paths = FundingPathEnumerator::default_paths_for(
        protocol,
        gross_profit_usd_micro,
        swap_cost_usd_micro,
        chain_cost_usd_micro,
        tip_usd_micro,
        notional_usd_micro,
        inventory_usd_micro,
        flash_fee_bps,
    );
    enumr.pick_best(&paths)
}

pub fn strategy_ix_labels(strategy: FundingStrategy, protocol: Protocol) -> Vec<String> {
    match (strategy, protocol) {
        (FundingStrategy::SaveFlashLoan, _) => vec![
            "ComputeBudget".into(),
            "RefreshReserve*".into(),
            "RefreshObligation".into(),
            "FlashBorrowReserveLiquidity".into(),
            "LiquidateObligationAndRedeemReserveCollateral".into(),
            "SwapOptional".into(),
            "FlashRepayReserveLiquidity".into(),
        ],
        (FundingStrategy::KaminoFlashBorrow, _) => vec![
            "ComputeBudget".into(),
            "refresh_reserve*".into(),
            "refresh_obligation".into(),
            "CreateIdempotentAssociatedTokenAccount*".into(),
            "flash_borrow_reserve_liquidity".into(),
            "liquidate_v2".into(),
            "swap_optional".into(),
            "flash_repay_reserve_liquidity".into(),
        ],
        (FundingStrategy::Project0Receivership, _) => vec![
            "ComputeBudget".into(),
            "start_liquidation".into(),
            "withdraw".into(),
            "swap_optional".into(),
            "repay".into(),
            "end_liquidation".into(),
        ],
        (FundingStrategy::Inventory, Protocol::Kamino) => vec![
            "ComputeBudget".into(),
            "refresh_reserve*".into(),
            "refresh_obligation".into(),
            "CreateIdempotentAssociatedTokenAccount*".into(),
            "liquidate_v2".into(),
            "swap_optional".into(),
        ],
        (FundingStrategy::Inventory, Protocol::Save) => vec![
            "ComputeBudget".into(),
            "RefreshReserve*".into(),
            "RefreshObligation".into(),
            "Liquidate".into(),
            "SwapOptional".into(),
        ],
        (FundingStrategy::Inventory, Protocol::Project0) => vec![
            "ComputeBudget".into(),
            "lending_account_liquidate".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use liq_core::ProfitConfig;

    #[test]
    fn evaluate_returns_best_for_save() {
        let enumr = FundingPathEnumerator::new(ProfitConfig {
            min_profit_usd_micro: 100_000,
            min_roi_bps: 1,
            max_cost_usd_micro: 50_000_000,
            min_notional_usd_micro: 1,
        });
        let plan = evaluate_funding(
            &enumr,
            Protocol::Save,
            4_000_000,
            100_000,
            50_000,
            50_000,
            50_000_000,
            0,
            9,
        );
        assert_eq!(
            plan.best.unwrap().strategy,
            FundingStrategy::SaveFlashLoan
        );
    }
}
