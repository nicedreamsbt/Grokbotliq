//! Swap routing trait (Jupiter / local AMM). Stub quote for profitability gates.

use async_trait::async_trait;
use liq_core::{PriceFx, Pubkey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRequest {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub amount_in: u64,
    pub input_decimals: u8,
    pub output_decimals: u8,
    pub input_price: PriceFx,
    pub output_price: PriceFx,
    /// Assumed slippage bps for stub.
    pub slippage_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub amount_out: u64,
    pub cost_usd_micro: u64,
    pub route_label: String,
}

#[derive(Debug, Error)]
pub enum RouteError {
    #[error("no route")]
    NoRoute,
}

#[async_trait]
pub trait SwapRouter: Send + Sync {
    async fn quote(&self, req: &QuoteRequest) -> Result<Quote, RouteError>;
}

/// Naive mid-price router with flat slippage — for unit tests / dry-run.
pub struct StubRouter;

#[async_trait]
impl SwapRouter for StubRouter {
    async fn quote(&self, req: &QuoteRequest) -> Result<Quote, RouteError> {
        let in_usd = liq_core::amount_to_usd_micro(
            req.amount_in as u128,
            req.input_decimals,
            req.input_price,
        );
        let after_slip = in_usd * (10_000 - req.slippage_bps as u128) / 10_000;
        let cost = in_usd - after_slip;
        let out = after_slip
            .saturating_mul(10u128.pow(req.output_decimals as u32))
            .saturating_mul(PriceFx::SCALE)
            / req.output_price.0.saturating_mul(1_000_000).max(1);
        Ok(Quote {
            amount_out: out as u64,
            cost_usd_micro: cost as u64,
            route_label: "stub-mid".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_quote_applies_slippage() {
        let r = StubRouter;
        let q = r
            .quote(&QuoteRequest {
                input_mint: Pubkey::test(1, 1),
                output_mint: Pubkey::test(1, 2),
                amount_in: 1_000_000_000,
                input_decimals: 9,
                output_decimals: 6,
                input_price: PriceFx::from_f64(100.0),
                output_price: PriceFx::from_f64(1.0),
                slippage_bps: 30,
            })
            .await
            .unwrap();
        assert!(q.cost_usd_micro > 0);
        assert!(q.amount_out > 0);
    }
}
