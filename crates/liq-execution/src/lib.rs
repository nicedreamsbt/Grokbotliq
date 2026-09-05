//! Transaction submission: dry-run, bid profiles, Jito/RPC traits + mocks,
//! blockhash cache, ALT manager skeleton, tx template cache.

mod alt;
mod bid;
mod blockhash;
mod funding;
mod jito;
mod rpc;
mod plan;
mod template;

pub use alt::*;
pub use bid::*;
pub use blockhash::*;
pub use funding::*;
pub use jito::*;
pub use rpc::*;
pub use plan::*;
pub use template::*;

use async_trait::async_trait;
use liq_risk::{CircuitBreaker, RiskReject};
use liq_telemetry::Metrics;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedTx {
    pub label: String,
    pub protocol: String,
    pub account: String,
    pub notional_usd_micro: u64,
    pub expected_profit_usd_micro: i64,
    /// Serialized wire tx bytes (empty until signing).
    pub wire: Vec<u8>,
    pub ixs: Vec<String>,
    /// Wire-ready instruction list (program_id + metas + data); signing still behind traits.
    #[serde(default)]
    pub instructions: Vec<liq_core::Instruction>,
    #[serde(default)]
    pub funding_strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitResult {
    pub signature: Option<String>,
    pub dry_run: bool,
    pub accepted: bool,
    pub detail: String,
}

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("risk: {0}")]
    Risk(#[from] RiskReject),
    #[error("submit failed: {0}")]
    Submit(String),
    #[error("stale blockhash")]
    StaleBlockhash,
}

#[derive(Debug, Clone)]
pub struct ExecConfig {
    pub dry_run: bool,
    pub rpc_url: String,
    pub jito_block_engine_url: Option<String>,
    pub bid_profile: BidProfile,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            dry_run: true,
            rpc_url: "https://api.mainnet-beta.solana.com".into(),
            jito_block_engine_url: None,
            bid_profile: BidProfile::Balanced,
        }
    }
}

#[async_trait]
pub trait TxSubmitter: Send + Sync {
    async fn submit(&self, tx: &PreparedTx) -> Result<SubmitResult, ExecError>;
}

pub struct ExecutionEngine {
    pub config: ExecConfig,
    pub risk: Arc<CircuitBreaker>,
    pub metrics: Arc<Metrics>,
    pub blockhashes: Arc<BlockhashCache>,
    pub templates: Arc<TxTemplateCache>,
}

impl ExecutionEngine {
    pub fn new(config: ExecConfig, risk: Arc<CircuitBreaker>, metrics: Arc<Metrics>) -> Self {
        Self {
            config,
            risk,
            metrics,
            blockhashes: Arc::new(BlockhashCache::new(60)),
            templates: Arc::new(TxTemplateCache::new()),
        }
    }

    pub async fn execute(&self, tx: &PreparedTx, oracle_staleness_slots: u64) -> Result<SubmitResult, ExecError> {
        self.risk
            .check_allow(tx.notional_usd_micro, oracle_staleness_slots)?;
        self.metrics.liquidations_attempted.inc();
        self.risk.begin(tx.notional_usd_micro);

        let bid = self.config.bid_profile.compute_bid(tx.expected_profit_usd_micro.max(0) as u64);
        info!(
            target: "liq_execution",
            profile = ?self.config.bid_profile,
            priority_micro_lamports = bid.priority_fee_micro_lamports,
            jito_tip = bid.jito_tip_lamports,
            "computed bid"
        );

        if self.config.dry_run {
            self.metrics.dry_run_skips.inc();
            self.risk.end_success();
            info!(
                target: "liq_execution",
                account = %tx.account,
                protocol = %tx.protocol,
                profit = tx.expected_profit_usd_micro,
                "DRY_RUN: would submit liquidation"
            );
            return Ok(SubmitResult {
                signature: None,
                dry_run: true,
                accepted: true,
                detail: format!(
                    "dry_run tip={} prio={}",
                    bid.jito_tip_lamports, bid.priority_fee_micro_lamports
                ),
            });
        }

        if self.config.jito_block_engine_url.is_some() {
            warn!("Jito live submit not wired; credentials required");
        }
        self.risk.end_failure();
        self.metrics.liquidations_failed.inc();
        Err(ExecError::Submit(
            "live submit requires RPC/Jito credentials — see config".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use liq_risk::RiskLimits;

    #[tokio::test]
    async fn dry_run_succeeds_with_bid() {
        let metrics = Arc::new(Metrics::new());
        let risk = Arc::new(CircuitBreaker::new(RiskLimits::default(), metrics.clone()));
        let mut cfg = ExecConfig::default();
        cfg.bid_profile = BidProfile::Aggressive;
        let eng = ExecutionEngine::new(cfg, risk, metrics.clone());
        let tx = PreparedTx {
            label: "test".into(),
            protocol: "kamino".into(),
            account: "acct".into(),
            notional_usd_micro: 50_000_000,
            expected_profit_usd_micro: 1_000_000,
            wire: vec![],
            instructions: vec![],
            funding_strategy: None,
            ixs: vec!["refresh".into(), "liquidate".into()],
        };
        let r = eng.execute(&tx, 0).await.unwrap();
        assert!(r.dry_run);
        assert!(r.detail.contains("tip="));
        assert_eq!(metrics.dry_run_skips.get(), 1);
    }
}
