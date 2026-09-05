//! Swap routing: Jupiter instruction placeholder + DirectDex stub + route cache.

use async_trait::async_trait;
use liq_core::{
    programs, AccountMeta, Instruction, LabeledIx, PriceFx, Pubkey,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

/// SwapRouter builds quotes and can attach swap instructions into an atomic plan.
#[async_trait]
pub trait SwapRouter: Send + Sync {
    async fn quote(&self, req: &QuoteRequest) -> Result<Quote, RouteError>;

    /// Build swap instruction(s) to append after liquidation / before flash repay.
    fn build_swap_ixs(&self, req: &QuoteRequest, _quote: &Quote) -> Vec<LabeledIx>;
}

/// Naive mid-price router with flat slippage — for unit tests / dry-run.
pub struct StubRouter;

#[async_trait]
impl SwapRouter for StubRouter {
    async fn quote(&self, req: &QuoteRequest) -> Result<Quote, RouteError> {
        mid_quote(req, "stub-mid")
    }

    fn build_swap_ixs(&self, _req: &QuoteRequest, quote: &Quote) -> Vec<LabeledIx> {
        vec![LabeledIx {
            label: format!("swap:{}", quote.route_label),
            ix: Instruction::new(
                Pubkey::test(99, 1),
                vec![AccountMeta::new(Pubkey::test(99, 2), false)],
                b"STUB_SWAP".to_vec(),
            ),
        }]
    }
}

/// Jupiter v6 swap instruction placeholder (does not call Jupiter API).
/// Produces a labeled ix with non-empty data so atomic plans can attach a swap slot.
pub struct JupiterPlaceholder {
    pub program_id: Pubkey,
}

impl Default for JupiterPlaceholder {
    fn default() -> Self {
        Self {
            // Placeholder program id (not a real Jupiter program) — replace with live pin.
            program_id: Pubkey::test(74, 1),
        }
    }
}

#[async_trait]
impl SwapRouter for JupiterPlaceholder {
    async fn quote(&self, req: &QuoteRequest) -> Result<Quote, RouteError> {
        mid_quote(req, "jupiter-placeholder")
    }

    fn build_swap_ixs(&self, req: &QuoteRequest, quote: &Quote) -> Vec<LabeledIx> {
        // Placeholder data: tag + amount_in + amount_out (not real Jupiter layout).
        let mut data = b"JUP6PLACE".to_vec();
        data.extend_from_slice(&req.amount_in.to_le_bytes());
        data.extend_from_slice(&quote.amount_out.to_le_bytes());
        vec![LabeledIx {
            label: "jupiter_swap_placeholder".into(),
            ix: Instruction::new(
                self.program_id,
                vec![
                    AccountMeta::new(req.input_mint, false),
                    AccountMeta::new(req.output_mint, false),
                    AccountMeta::new_readonly(programs::token(), false),
                ],
                data,
            ),
        }]
    }
}

/// Direct DEX stub (single-hop) for HOT asset pairs.
pub struct DirectDexStub {
    pub label: String,
}

impl Default for DirectDexStub {
    fn default() -> Self {
        Self {
            label: "direct-dex".into(),
        }
    }
}

#[async_trait]
impl SwapRouter for DirectDexStub {
    async fn quote(&self, req: &QuoteRequest) -> Result<Quote, RouteError> {
        mid_quote(req, &self.label)
    }

    fn build_swap_ixs(&self, req: &QuoteRequest, _quote: &Quote) -> Vec<LabeledIx> {
        let mut data = b"DEX1".to_vec();
        data.extend_from_slice(&req.amount_in.to_le_bytes());
        vec![LabeledIx {
            label: format!("direct_dex:{}", self.label),
            ix: Instruction::new(
                Pubkey::test(88, 1),
                vec![
                    AccountMeta::new(req.input_mint, false),
                    AccountMeta::new(req.output_mint, false),
                ],
                data,
            ),
        }]
    }
}

fn mid_quote(req: &QuoteRequest, label: &str) -> Result<Quote, RouteError> {
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
        route_label: label.into(),
    })
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct RouteKey {
    pub input: Pubkey,
    pub output: Pubkey,
}

/// Prefetch route cache for HOT assets.
#[derive(Default)]
pub struct RouteCache {
    inner: RwLock<HashMap<RouteKey, Quote>>,
}

impl RouteCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, key: RouteKey, quote: Quote) {
        self.inner.write().insert(key, quote);
    }

    pub fn get(&self, key: &RouteKey) -> Option<Quote> {
        self.inner.read().get(key).cloned()
    }

    pub fn prefetch_hot(
        &self,
        pairs: &[(Pubkey, Pubkey)],
        quotes: &[Quote],
    ) {
        for ((a, b), q) in pairs.iter().zip(quotes.iter()) {
            self.insert(
                RouteKey {
                    input: *a,
                    output: *b,
                },
                q.clone(),
            );
        }
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
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

    #[tokio::test]
    async fn jupiter_placeholder_builds_ix() {
        let j = JupiterPlaceholder::default();
        let req = QuoteRequest {
            input_mint: Pubkey::test(1, 1),
            output_mint: Pubkey::test(1, 2),
            amount_in: 1000,
            input_decimals: 6,
            output_decimals: 6,
            input_price: PriceFx::from_f64(1.0),
            output_price: PriceFx::from_f64(1.0),
            slippage_bps: 10,
        };
        let q = j.quote(&req).await.unwrap();
        let ixs = j.build_swap_ixs(&req, &q);
        assert_eq!(ixs.len(), 1);
        assert!(!ixs[0].ix.data.is_empty());
        assert!(!ixs[0].ix.accounts.is_empty());
    }

    #[test]
    fn route_cache_prefetch() {
        let cache = RouteCache::new();
        let a = Pubkey::test(1, 1);
        let b = Pubkey::test(1, 2);
        cache.prefetch_hot(
            &[(a, b)],
            &[Quote {
                amount_out: 1,
                cost_usd_micro: 1,
                route_label: "hot".into(),
            }],
        );
        assert_eq!(cache.len(), 1);
        assert!(cache
            .get(&RouteKey {
                input: a,
                output: b
            })
            .is_some());
    }
}
