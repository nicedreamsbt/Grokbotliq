use serde::{Deserialize, Serialize};

/// Configurable profitability gates (USD microunits: 1e6 = $1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfitConfig {
    /// Minimum expected net profit in micro-USD.
    pub min_profit_usd_micro: u64,
    /// Minimum ROI = profit / capital_used (scaled by 1e6; 50_000 = 5%).
    pub min_roi_bps: u64,
    /// Max gas + tip budget in micro-USD.
    pub max_cost_usd_micro: u64,
    /// Minimum liquidation notional in micro-USD (dust filter).
    pub min_notional_usd_micro: u64,
}

impl Default for ProfitConfig {
    fn default() -> Self {
        Self {
            min_profit_usd_micro: 500_000,      // $0.50
            min_roi_bps: 10,                    // 0.10%
            max_cost_usd_micro: 2_000_000,      // $2
            min_notional_usd_micro: 10_000_000, // $10
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfitInput {
    /// Gross bonus value seized minus debt repaid, before costs (micro-USD).
    pub gross_profit_usd_micro: i64,
    /// Expected swap slippage / fees (micro-USD).
    pub swap_cost_usd_micro: u64,
    /// Priority fee + Jito tip + flat protocol fee (micro-USD).
    pub chain_cost_usd_micro: u64,
    /// Capital deployed (inventory or flashloan face) micro-USD.
    pub capital_used_usd_micro: u64,
    /// Liquidation notional micro-USD.
    pub notional_usd_micro: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfitDecision {
    Accept { net_profit_usd_micro: i64, roi_bps: u64 },
    Reject { reason: ProfitRejectReason, net_profit_usd_micro: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfitRejectReason {
    BelowMinProfit,
    BelowMinRoi,
    CostTooHigh,
    NotionalTooSmall,
    NegativeNet,
}

pub struct ProfitabilityCalculator {
    pub config: ProfitConfig,
}

impl ProfitabilityCalculator {
    pub fn new(config: ProfitConfig) -> Self {
        Self { config }
    }

    pub fn evaluate(&self, input: &ProfitInput) -> ProfitDecision {
        let total_cost = input.swap_cost_usd_micro.saturating_add(input.chain_cost_usd_micro);
        if total_cost > self.config.max_cost_usd_micro {
            return ProfitDecision::Reject {
                reason: ProfitRejectReason::CostTooHigh,
                net_profit_usd_micro: input.gross_profit_usd_micro - total_cost as i64,
            };
        }
        if input.notional_usd_micro < self.config.min_notional_usd_micro {
            return ProfitDecision::Reject {
                reason: ProfitRejectReason::NotionalTooSmall,
                net_profit_usd_micro: input.gross_profit_usd_micro - total_cost as i64,
            };
        }

        let net = input.gross_profit_usd_micro - total_cost as i64;
        if net <= 0 {
            return ProfitDecision::Reject {
                reason: ProfitRejectReason::NegativeNet,
                net_profit_usd_micro: net,
            };
        }
        if (net as u64) < self.config.min_profit_usd_micro {
            return ProfitDecision::Reject {
                reason: ProfitRejectReason::BelowMinProfit,
                net_profit_usd_micro: net,
            };
        }

        let roi_bps = if input.capital_used_usd_micro == 0 {
            u64::MAX
        } else {
            // roi_bps = net / capital * 10_000
            ((net as u128) * 10_000 / input.capital_used_usd_micro as u128) as u64
        };
        if roi_bps < self.config.min_roi_bps {
            return ProfitDecision::Reject {
                reason: ProfitRejectReason::BelowMinRoi,
                net_profit_usd_micro: net,
            };
        }

        ProfitDecision::Accept {
            net_profit_usd_micro: net,
            roi_bps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_profitable_liquidation() {
        let calc = ProfitabilityCalculator::new(ProfitConfig::default());
        let d = calc.evaluate(&ProfitInput {
            gross_profit_usd_micro: 5_000_000, // $5
            swap_cost_usd_micro: 200_000,
            chain_cost_usd_micro: 100_000,
            capital_used_usd_micro: 100_000_000, // $100
            notional_usd_micro: 100_000_000,
        });
        match d {
            ProfitDecision::Accept { net_profit_usd_micro, roi_bps } => {
                assert_eq!(net_profit_usd_micro, 4_700_000);
                assert!(roi_bps >= 10);
            }
            other => panic!("expected Accept, got {other:?}"),
        }
    }

    #[test]
    fn rejects_below_min_profit() {
        let calc = ProfitabilityCalculator::new(ProfitConfig::default());
        let d = calc.evaluate(&ProfitInput {
            gross_profit_usd_micro: 600_000,
            swap_cost_usd_micro: 200_000,
            chain_cost_usd_micro: 100_000,
            capital_used_usd_micro: 50_000_000,
            notional_usd_micro: 50_000_000,
        });
        // net = 300_000 < 500_000 min
        assert!(matches!(
            d,
            ProfitDecision::Reject {
                reason: ProfitRejectReason::BelowMinProfit,
                ..
            }
        ));
    }

    #[test]
    fn rejects_dust_notional() {
        let calc = ProfitabilityCalculator::new(ProfitConfig::default());
        let d = calc.evaluate(&ProfitInput {
            gross_profit_usd_micro: 5_000_000,
            swap_cost_usd_micro: 0,
            chain_cost_usd_micro: 0,
            capital_used_usd_micro: 1_000_000,
            notional_usd_micro: 1_000_000, // $1 < $10
        });
        assert!(matches!(
            d,
            ProfitDecision::Reject {
                reason: ProfitRejectReason::NotionalTooSmall,
                ..
            }
        ));
    }
}
