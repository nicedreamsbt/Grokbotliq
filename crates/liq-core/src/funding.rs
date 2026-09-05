//! FundingStrategy paths and opportunity EV selection.

use crate::profitability::{ProfitConfig, ProfitDecision, ProfitInput, ProfitabilityCalculator};
use crate::types::Protocol;
use serde::{Deserialize, Serialize};

/// How the liquidator funds the repay leg of an atomic liquidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FundingStrategy {
    /// Use liquidator inventory (wallet balances / ATAs).
    Inventory,
    /// Save/Solend flash borrow → liquidate → (swap) → flash repay.
    SaveFlashLoan,
    /// Kamino klend flash_borrow → liquidate → (swap) → flash_repay (when available).
    KaminoFlashBorrow,
    /// Project 0 receivership: start → withdraw/swap/repay → end (avoids flash when applicable).
    Project0Receivership,
}

impl FundingStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inventory => "Inventory",
            Self::SaveFlashLoan => "SaveFlashLoan",
            Self::KaminoFlashBorrow => "KaminoFlashBorrow",
            Self::Project0Receivership => "Project0Receivership",
        }
    }

    /// Strategies that are structurally applicable for a given protocol.
    pub fn applicable_for(protocol: Protocol) -> &'static [FundingStrategy] {
        match protocol {
            Protocol::Kamino => &[
                FundingStrategy::Inventory,
                FundingStrategy::KaminoFlashBorrow,
            ],
            Protocol::Save => &[FundingStrategy::Inventory, FundingStrategy::SaveFlashLoan],
            Protocol::Project0 => &[
                FundingStrategy::Inventory,
                FundingStrategy::Project0Receivership,
            ],
        }
    }
}

/// Inputs for scoring one funding path (all USD microunits unless noted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingPathInput {
    pub strategy: FundingStrategy,
    pub protocol: Protocol,
    /// Gross liquidation bonus before costs.
    pub gross_profit_usd_micro: i64,
    pub swap_cost_usd_micro: u64,
    pub chain_cost_usd_micro: u64,
    /// Flash loan fee (0 for Inventory / P0 receivership without flash).
    pub flash_fee_usd_micro: u64,
    /// Jito tip modeled in micro-USD.
    pub tip_usd_micro: u64,
    pub capital_used_usd_micro: u64,
    pub notional_usd_micro: u64,
    /// Landing probability in bps (10_000 = 100%).
    pub landing_prob_bps: u64,
    /// Relative latency penalty (higher = slower). Dimensionless; used as divisor weight.
    pub latency_weight: u64,
    /// Whether this path can be built with current inventory / protocol support.
    pub feasible: bool,
}

/// Scored funding path with EV and profit gate result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredFundingPath {
    pub strategy: FundingStrategy,
    pub protocol: Protocol,
    pub net_profit_usd_micro: i64,
    /// EV score = net * landing_prob_bps / latency_weight (capital via ROI gate).
    pub expected_value_score: i64,
    pub decision: ProfitDecision,
    pub flash_fee_usd_micro: u64,
    pub tip_usd_micro: u64,
    pub capital_used_usd_micro: u64,
}

/// Opportunity after evaluating all feasible funding strategies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityPlan {
    pub protocol: Protocol,
    pub account: String,
    pub candidates: Vec<ScoredFundingPath>,
    pub best: Option<ScoredFundingPath>,
}

pub struct FundingPathEnumerator {
    pub profit: ProfitabilityCalculator,
}

impl FundingPathEnumerator {
    pub fn new(config: ProfitConfig) -> Self {
        Self {
            profit: ProfitabilityCalculator::new(config),
        }
    }

    pub fn score_one(&self, input: &FundingPathInput) -> Option<ScoredFundingPath> {
        if !input.feasible {
            return None;
        }
        let total_extra = input
            .flash_fee_usd_micro
            .saturating_add(input.tip_usd_micro);
        let profit_input = ProfitInput {
            gross_profit_usd_micro: input.gross_profit_usd_micro,
            swap_cost_usd_micro: input.swap_cost_usd_micro,
            chain_cost_usd_micro: input
                .chain_cost_usd_micro
                .saturating_add(total_extra),
            capital_used_usd_micro: input.capital_used_usd_micro.max(1),
            notional_usd_micro: input.notional_usd_micro,
        };
        let decision = self.profit.evaluate(&profit_input);
        let net = match &decision {
            ProfitDecision::Accept {
                net_profit_usd_micro,
                ..
            } => *net_profit_usd_micro,
            ProfitDecision::Reject {
                net_profit_usd_micro,
                ..
            } => *net_profit_usd_micro,
        };
        let latency = input.latency_weight.max(1);
        // Rank by net × landing_prob / latency. Capital efficiency is enforced via
        // ProfitabilityCalculator ROI gates (capital_used), not this divisor — otherwise
        // flash paths with ~$0 capital always dominate even when inventory is cheaper.
        let expected_value_score = if net <= 0 {
            net
        } else {
            ((net as i128) * (input.landing_prob_bps as i128) / (latency as i128)) as i64
        };
        Some(ScoredFundingPath {
            strategy: input.strategy,
            protocol: input.protocol,
            net_profit_usd_micro: net,
            expected_value_score,
            decision,
            flash_fee_usd_micro: input.flash_fee_usd_micro,
            tip_usd_micro: input.tip_usd_micro,
            capital_used_usd_micro: input.capital_used_usd_micro,
        })
    }

    /// Evaluate all paths; pick max expected_value_score among Accept decisions,
    /// else best Reject (still useful for logging).
    pub fn pick_best(&self, inputs: &[FundingPathInput]) -> OpportunityPlan {
        let protocol = inputs
            .first()
            .map(|i| i.protocol)
            .unwrap_or(Protocol::Kamino);
        let account = String::new();
        let mut candidates: Vec<ScoredFundingPath> = inputs
            .iter()
            .filter_map(|i| self.score_one(i))
            .collect();
        candidates.sort_by(|a, b| b.expected_value_score.cmp(&a.expected_value_score));
        let best = candidates
            .iter()
            .find(|c| matches!(c.decision, ProfitDecision::Accept { .. }))
            .cloned()
            .or_else(|| candidates.first().cloned());
        OpportunityPlan {
            protocol,
            account,
            candidates,
            best,
        }
    }

    /// Build default path inputs for a liquidation given protocol + economics.
    pub fn default_paths_for(
        protocol: Protocol,
        gross_profit_usd_micro: i64,
        swap_cost_usd_micro: u64,
        chain_cost_usd_micro: u64,
        tip_usd_micro: u64,
        notional_usd_micro: u64,
        inventory_available_usd_micro: u64,
        flash_fee_bps: u64,
    ) -> Vec<FundingPathInput> {
        let flash_fee = notional_usd_micro.saturating_mul(flash_fee_bps) / 10_000;
        FundingStrategy::applicable_for(protocol)
            .iter()
            .map(|&strategy| {
                let (capital, fee, latency, landing, feasible) = match strategy {
                    FundingStrategy::Inventory => (
                        inventory_available_usd_micro.min(notional_usd_micro).max(1),
                        0u64,
                        10u64,
                        8_500u64,
                        inventory_available_usd_micro >= notional_usd_micro / 2,
                    ),
                    FundingStrategy::SaveFlashLoan => {
                        (1, flash_fee, 14, 7_500, protocol == Protocol::Save)
                    }
                    FundingStrategy::KaminoFlashBorrow => {
                        (1, flash_fee, 14, 7_500, protocol == Protocol::Kamino)
                    }
                    FundingStrategy::Project0Receivership => {
                        // Receivership withdraws collateral first — low capital, no flash fee.
                        (
                            notional_usd_micro / 20 + 1,
                            0,
                            18,
                            7_000,
                            protocol == Protocol::Project0,
                        )
                    }
                };
                FundingPathInput {
                    strategy,
                    protocol,
                    gross_profit_usd_micro,
                    swap_cost_usd_micro,
                    chain_cost_usd_micro,
                    flash_fee_usd_micro: fee,
                    tip_usd_micro,
                    capital_used_usd_micro: capital,
                    notional_usd_micro,
                    landing_prob_bps: landing,
                    latency_weight: latency,
                    feasible,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_flash_when_inventory_short() {
        let enumr = FundingPathEnumerator::new(ProfitConfig {
            min_profit_usd_micro: 100_000,
            min_roi_bps: 1,
            max_cost_usd_micro: 50_000_000,
            min_notional_usd_micro: 1_000_000,
        });
        let paths = FundingPathEnumerator::default_paths_for(
            Protocol::Save,
            5_000_000,
            100_000,
            50_000,
            50_000,
            100_000_000, // $100 notional
            1_000_000,   // only $1 inventory
            9,           // 9 bps flash fee
        );
        let plan = enumr.pick_best(&paths);
        assert!(plan.best.is_some());
        let best = plan.best.unwrap();
        assert_eq!(best.strategy, FundingStrategy::SaveFlashLoan);
        assert!(matches!(best.decision, ProfitDecision::Accept { .. }));
    }

    #[test]
    fn inventory_preferred_when_funded_and_faster() {
        let enumr = FundingPathEnumerator::new(ProfitConfig {
            min_profit_usd_micro: 100_000,
            min_roi_bps: 1,
            max_cost_usd_micro: 50_000_000,
            min_notional_usd_micro: 1_000_000,
        });
        let paths = FundingPathEnumerator::default_paths_for(
            Protocol::Kamino,
            5_000_000,
            50_000,
            50_000,
            50_000,
            50_000_000,
            100_000_000, // plenty of inventory
            30,          // expensive flash
        );
        let plan = enumr.pick_best(&paths);
        let best = plan.best.expect("best");
        assert_eq!(best.strategy, FundingStrategy::Inventory);
    }

    #[test]
    fn p0_receivership_applicable() {
        assert!(FundingStrategy::applicable_for(Protocol::Project0)
            .contains(&FundingStrategy::Project0Receivership));
        let enumr = FundingPathEnumerator::new(ProfitConfig::default());
        let paths = FundingPathEnumerator::default_paths_for(
            Protocol::Project0,
            8_000_000,
            200_000,
            100_000,
            100_000,
            80_000_000,
            500_000,
            0,
        );
        let plan = enumr.pick_best(&paths);
        assert_eq!(
            plan.best.unwrap().strategy,
            FundingStrategy::Project0Receivership
        );
    }
}
