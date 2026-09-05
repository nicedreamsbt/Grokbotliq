//! Shadow mode: process fixtures / mock stream, evaluate opportunities, never sign/broadcast.
//! Asserts DRY_RUN is enabled.

use anyhow::{bail, ensure, Context};
use liq_core::{
    CandidateIndex, FundingStrategy, OracleTriggerPath, PriceFx, ProfitConfig, ProfitDecision,
    ProfitInput, ProfitabilityCalculator, Protocol, TriggerHit, UpdateSource,
};
use liq_execution::{
    build_strategy_ixs, BidProfile, ExecConfig, ExecutionEngine, PlanAccountSet, PreparedTx,
};
use liq_risk::{CircuitBreaker, RiskLimits};
use liq_routing::JupiterQuoteBlob;
use liq_streaming::{
    borrower_to_meta, borrower_triggers, drain_all, load_borrowers, load_oracle_ticks,
    resolve_fixtures_dir, rpc_url_configured, shadow_tx_base64, ticks_to_events, BorrowerFixture,
    FixtureBootstrap, HttpJsonRpcTransport, JsonRpcBootstrap, MockGeyser, RpcBootstrap,
    StreamEvent, YellowstoneConfig,
};
use liq_telemetry::Metrics;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Serialize)]
struct ShadowOpportunity {
    mode: &'static str,
    dry_run: bool,
    protocol: String,
    account: String,
    asset: String,
    side: String,
    trigger_price_usd: f64,
    new_price_usd: f64,
    health: f64,
    band: String,
    plan_ixs: Vec<String>,
    notional_usd_micro: u64,
    expected_profit_usd_micro: i64,
    profit_decision: String,
    would_submit: bool,
    slot: u64,
}

fn plan_ixs(protocol: Protocol) -> Vec<String> {
    match protocol {
        Protocol::Kamino => liq_kamino::liquidation_ix_order()
            .iter()
            .map(|s| s.to_string())
            .collect(),
        Protocol::Project0 => vec![
            "ComputeBudget".into(),
            "lending_account_liquidate".into(),
        ],
        Protocol::Save => liq_save::liquidation_ix_order()
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }
}

fn find_borrower<'a>(
    borrowers: &'a [BorrowerFixture],
    hit: &TriggerHit,
) -> Option<&'a BorrowerFixture> {
    borrowers.iter().find(|b| b.account.to_pubkey() == hit.account)
}

fn env_dry_run() -> bool {
    std::env::var("DRY_RUN")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let dry = env_dry_run();
    ensure!(
        dry,
        "shadow refuses to run with DRY_RUN=false — shadow never signs or broadcasts"
    );
    info!(dry_run = dry, "shadow mode starting (DRY_RUN asserted)");

    if let Some(cfg) = YellowstoneConfig::from_env() {
        warn!(
            endpoint = %cfg.endpoint,
            has_token = cfg.has_credentials(),
            "GEYSER configured but shadow uses fixtures/mock only (no live subscribe)"
        );
    }

    let mut args = std::env::args().skip(1);
    let fixtures_arg = args.next();
    let dir = resolve_fixtures_dir(fixtures_arg.as_deref());
    let ticks_path = PathBuf::from(&dir).join("oracle_ticks.json");
    let borrowers_path = PathBuf::from(&dir).join("borrowers.json");
    if !ticks_path.exists() {
        bail!("missing {} — pass fixtures dir or set LIQ_FIXTURES", ticks_path.display());
    }

    let ticks = load_oracle_ticks(&ticks_path).context("oracle ticks")?;
    let borrowers_file = load_borrowers(&borrowers_path).context("borrowers")?;

    let index = Arc::new(CandidateIndex::new());
    for b in &borrowers_file.borrowers {
        index.upsert_candidate(borrower_to_meta(b)?);
        index.set_triggers(b.account.to_pubkey(), borrower_triggers(b)?);
    }

    let metrics = Arc::new(Metrics::new());
    let path = OracleTriggerPath::new(index.clone(), metrics.clone());
    let profit = ProfitabilityCalculator::new(ProfitConfig::default());
    let risk = Arc::new(CircuitBreaker::new(RiskLimits::default(), metrics.clone()));
    let exec = ExecutionEngine::new(
        ExecConfig {
            dry_run: true, // hard-forced
            rpc_url: std::env::var("RPC_URL").unwrap_or_else(|_| "https://YOUR_PRIVATE_RPC".into()),
            jito_block_engine_url: None,
            bid_profile: BidProfile::Conservative,
        },
        risk,
        metrics.clone(),
    );
    ensure!(exec.config.dry_run, "internal: execution dry_run must be true");

    let mut seeded = std::collections::HashSet::new();
    for t in &ticks.ticks {
        let asset = t.asset.to_pubkey();
        if seeded.insert(asset) {
            path.prices.set(asset, PriceFx::from_f64(t.price_usd));
        }
    }

    let events = ticks_to_events(&ticks);
    let mut seen = std::collections::HashSet::new();
    let mut stream_events = Vec::new();
    for ev in events {
        if let StreamEvent::Price { asset, .. } = &ev {
            if !seen.insert(*asset) {
                stream_events.push(ev);
            }
        }
    }

    let mock = Arc::new(MockGeyser::named("fixture-shadow", stream_events));
    let mut count = 0usize;

    for ev in drain_all(mock).await? {
        if let StreamEvent::Price {
            asset,
            price_fx,
            slot,
            write_version,
        } = ev
        {
            let hits = path.apply_oracle_update(
                asset,
                PriceFx(price_fx),
                slot,
                write_version,
                UpdateSource::Mock,
            );
            for hit in hits {
                let Some(b) = find_borrower(&borrowers_file.borrowers, &hit) else {
                    continue;
                };
                let meta = borrower_to_meta(b)?;
                let plan = &b.plan;
                let input = ProfitInput {
                    gross_profit_usd_micro: (plan.gross_profit_usd * 1_000_000.0) as i64,
                    swap_cost_usd_micro: 100_000,
                    chain_cost_usd_micro: 50_000,
                    capital_used_usd_micro: (plan.capital_usd * 1_000_000.0) as u64,
                    notional_usd_micro: (plan.notional_usd * 1_000_000.0) as u64,
                };
                let decision = profit.evaluate(&input);
                let (expected, would, decision_s) = match &decision {
                    ProfitDecision::Accept {
                        net_profit_usd_micro,
                        roi_bps,
                    } => (
                        *net_profit_usd_micro,
                        true,
                        format!("Accept(net={net_profit_usd_micro},roi_bps={roi_bps})"),
                    ),
                    ProfitDecision::Reject {
                        reason,
                        net_profit_usd_micro,
                    } => (
                        *net_profit_usd_micro,
                        false,
                        format!("Reject({reason:?},net={net_profit_usd_micro})"),
                    ),
                };

                let rec = ShadowOpportunity {
                    mode: "shadow",
                    dry_run: true,
                    protocol: format!("{:?}", meta.protocol),
                    account: meta.account.to_string(),
                    asset: hit.asset.to_string(),
                    side: format!("{:?}", hit.side),
                    trigger_price_usd: hit.trigger_price.to_f64(),
                    new_price_usd: hit.new_price.to_f64(),
                    health: meta.health.to_f64(),
                    band: meta.band.as_str().into(),
                    plan_ixs: plan_ixs(meta.protocol),
                    notional_usd_micro: input.notional_usd_micro,
                    expected_profit_usd_micro: expected,
                    profit_decision: decision_s,
                    would_submit: would,
                    slot,
                };
                println!("{}", serde_json::to_string(&rec)?);
                count += 1;

                if would {
                    let strategy = match meta.protocol {
                        Protocol::Kamino => FundingStrategy::KaminoFlashBorrow,
                        Protocol::Save => FundingStrategy::SaveFlashLoan,
                        Protocol::Project0 => FundingStrategy::Project0Receivership,
                    };
                    let accounts = PlanAccountSet::from_seed(meta.account);
                    let planned = build_strategy_ixs(
                        meta.protocol,
                        strategy,
                        &accounts,
                        b.plan.repay_amount,
                        &JupiterQuoteBlob::from_env(),
                    );
                    let wire_ixs: Vec<_> = planned.labeled.iter().map(|l| l.ix.clone()).collect();
                    let envelope = shadow_tx_base64(&wire_ixs, "11111111111111111111111111111111");
                    let tx = PreparedTx {
                        label: "shadow-would-submit".into(),
                        protocol: rec.protocol.clone(),
                        account: rec.account.clone(),
                        notional_usd_micro: rec.notional_usd_micro,
                        expected_profit_usd_micro: rec.expected_profit_usd_micro,
                        wire: envelope.clone().into_bytes(),
                        instructions: wire_ixs,
                        funding_strategy: Some(strategy.as_str().to_string()),
                        ixs: planned.labeled.iter().map(|l| l.label.clone()).collect(),
                    };
                    let res = exec.execute(&tx, 0).await?;
                    ensure!(res.dry_run, "shadow must only dry-run");
                    ensure!(res.signature.is_none(), "shadow must not produce signatures");
                    info!(
                        account = %rec.account,
                        ixs = tx.instructions.len(),
                        flash = planned.used_flash_builder,
                        detail = %res.detail,
                        "shadow dry-run only (no broadcast)"
                    );
                    // Simulate when RPC configured; fixtures always simulate locally.
                    let rpc = std::env::var("RPC_URL").unwrap_or_else(|_| "https://YOUR_PRIVATE_RPC".into());
                    if rpc_url_configured(&rpc) {
                        if let Ok(transport) = HttpJsonRpcTransport::new(&rpc) {
                            let boot = JsonRpcBootstrap::new(transport);
                            match boot.simulate_transaction(&envelope, false).await {
                                Ok(sim) => info!(err=?sim.err, units=?sim.units_consumed, "shadow simulateTransaction"),
                                Err(e) => info!(error=%e, "shadow simulate skipped"),
                            }
                        }
                    } else {
                        let boot = FixtureBootstrap::demo_for_protocols();
                        let sim = boot.simulate_transaction(&envelope, false).await?;
                        info!(logs=?sim.logs, "shadow fixture simulate (sigVerify=false, no broadcast)");
                    }
                }
            }
        }
    }

    info!(
        opportunities = count,
        kamino = liq_kamino::KLEND_PROGRAM_ID_MAINNET,
        p0 = liq_project0::MARGINFI_PROGRAM_ID_MAINNET,
        save = liq_save::SAVE_PROGRAM_ID_MAINNET,
        "shadow complete (no broadcast)"
    );
    Ok(())
}
