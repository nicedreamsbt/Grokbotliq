use anyhow::Context;
use liq_core::{
    CandidateIndex, OracleTriggerPath, ProfitConfig, ProfitabilityCalculator, PriceFx, Pubkey,
    UpdateSource,
};
use liq_execution::{ExecConfig, ExecutionEngine, PreparedTx};
use liq_risk::{CircuitBreaker, RiskLimits};
use liq_telemetry::Metrics;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Deserialize)]
struct AppConfig {
    dry_run: bool,
    rpc_url: String,
    #[serde(default)]
    jito_block_engine_url: Option<String>,
    #[serde(default)]
    geyser_endpoint: Option<String>,
    #[serde(default)]
    min_profit_usd: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            dry_run: true,
            rpc_url: "https://YOUR_PRIVATE_RPC".into(),
            jito_block_engine_url: None,
            geyser_endpoint: None,
            min_profit_usd: 0.5,
        }
    }
}

fn load_config() -> AppConfig {
    let path = std::env::var("LIQ_CONFIG").unwrap_or_else(|_| "config/example.toml".into());
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = load_config();
    let dry = std::env::var("DRY_RUN")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(cfg.dry_run);

    info!(
        dry_run = dry,
        rpc = %cfg.rpc_url,
        geyser = ?cfg.geyser_endpoint,
        jito = ?cfg.jito_block_engine_url,
        "starting liquidator"
    );
    info!(
        kamino = liq_kamino::KLEND_PROGRAM_ID_MAINNET,
        p0 = liq_project0::MARGINFI_PROGRAM_ID_MAINNET,
        save = liq_save::SAVE_PROGRAM_ID_MAINNET,
        "protocol program ids"
    );

    let metrics = Arc::new(Metrics::new());
    let index = Arc::new(CandidateIndex::new());
    let oracle_path = OracleTriggerPath::new(index.clone(), metrics.clone());
    let risk = Arc::new(CircuitBreaker::new(RiskLimits::default(), metrics.clone()));
    let mut profit_cfg = ProfitConfig::default();
    profit_cfg.min_profit_usd_micro = (cfg.min_profit_usd * 1_000_000.0) as u64;
    let _profit = ProfitabilityCalculator::new(profit_cfg);

    let exec = ExecutionEngine::new(
        ExecConfig {
            dry_run: dry,
            rpc_url: cfg.rpc_url.clone(),
            jito_block_engine_url: cfg.jito_block_engine_url.clone(),
        },
        risk,
        metrics.clone(),
    );

    // Demo wake path with mock oracle tick (foundations smoke).
    let asset = Pubkey::test(9, 1);
    oracle_path.prices.set(asset, PriceFx::from_f64(110.0));
    let _hits = oracle_path.apply_oracle_update(
        asset,
        PriceFx::from_f64(110.0),
        1,
        1,
        UpdateSource::Mock,
    );

    if dry {
        let demo = PreparedTx {
            label: "boot-smoke".into(),
            protocol: "none".into(),
            account: "demo".into(),
            notional_usd_micro: 50_000_000,
            expected_profit_usd_micro: 1_000_000,
            wire: vec![],
            ixs: liq_kamino::liquidation_ix_order()
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        let res = exec.execute(&demo, 0).await.context("dry execute")?;
        info!(?res, "boot smoke complete");
    }

    if cfg.geyser_endpoint.is_none() {
        info!("no Geyser endpoint configured — idle after smoke (set geyser_endpoint / GEYSER_ENDPOINT)");
    }

    // Keep process semantics simple for scaffold: exit after smoke in dry mode.
    let _ = PathBuf::from("config");
    Ok(())
}
