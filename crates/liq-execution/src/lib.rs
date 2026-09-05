//! Transaction submission: dry-run, RPC placeholder, Jito placeholder.

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
    /// Serialized wire tx bytes (empty in dry foundations).
    pub wire: Vec<u8>,
    pub ixs: Vec<String>,
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
}

#[derive(Debug, Clone)]
pub struct ExecConfig {
    pub dry_run: bool,
    pub rpc_url: String,
    pub jito_block_engine_url: Option<String>,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            dry_run: true,
            rpc_url: "https://api.mainnet-beta.solana.com".into(),
            jito_block_engine_url: None,
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
}

impl ExecutionEngine {
    pub fn new(config: ExecConfig, risk: Arc<CircuitBreaker>, metrics: Arc<Metrics>) -> Self {
        Self {
            config,
            risk,
            metrics,
        }
    }

    pub async fn execute(&self, tx: &PreparedTx, oracle_staleness_slots: u64) -> Result<SubmitResult, ExecError> {
        self.risk
            .check_allow(tx.notional_usd_micro, oracle_staleness_slots)?;
        self.metrics.liquidations_attempted.inc();
        self.risk.begin(tx.notional_usd_micro);

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
                detail: "dry_run".into(),
            });
        }

        // Live path placeholders — require credentials.
        if self.config.jito_block_engine_url.is_some() {
            warn!("Jito submit not wired yet; falling back to stub");
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
    async fn dry_run_succeeds() {
        let metrics = Arc::new(Metrics::new());
        let risk = Arc::new(CircuitBreaker::new(RiskLimits::default(), metrics.clone()));
        let eng = ExecutionEngine::new(ExecConfig::default(), risk, metrics.clone());
        let tx = PreparedTx {
            label: "test".into(),
            protocol: "kamino".into(),
            account: "acct".into(),
            notional_usd_micro: 50_000_000,
            expected_profit_usd_micro: 1_000_000,
            wire: vec![],
            ixs: vec!["refresh".into(), "liquidate".into()],
        };
        let r = eng.execute(&tx, 0).await.unwrap();
        assert!(r.dry_run);
        assert_eq!(metrics.dry_run_skips.get(), 1);
    }
}
