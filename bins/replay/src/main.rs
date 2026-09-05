//! Replay fixtures through oracle → candidate → plan, printing opportunity records.

use anyhow::{bail, Context};
use liq_core::{
    CandidateIndex, OracleTriggerPath, PriceFx, ProfitConfig, ProfitDecision, ProfitInput,
    ProfitabilityCalculator, Protocol, TriggerHit, UpdateSource,
};
use liq_execution::{BidProfile, ExecConfig, ExecutionEngine, PreparedTx};
use liq_risk::{CircuitBreaker, RiskLimits};
use liq_streaming::{
    borrower_to_meta, borrower_triggers, drain_all, load_borrowers, load_oracle_ticks,
    resolve_fixtures_dir, ticks_to_events, BorrowerFixture, MockGeyser, StreamEvent,
};
use liq_telemetry::Metrics;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Serialize)]
struct OpportunityRecord {
    source: &'static str,
    protocol: String,
    account: String,
    asset: String,
    side: String,
    trigger_price_usd: f64,
    new_price_usd: f64,
    health: f64,
    band: String,
    plan_ixs: Vec<String>,
    ix_data_preview_hex: String,
    notional_usd_micro: u64,
    expected_profit_usd_micro: i64,
    profit_decision: String,
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

fn encode_preview(protocol: Protocol, repay: u64) -> String {
    let bytes = match protocol {
        Protocol::Kamino => liq_kamino::encode_liquidate_v2_data(repay, 0, 0),
        Protocol::Project0 => liq_project0::encode_classic_liquidate(repay),
        Protocol::Save => liq_save::encode_liquidate_and_redeem(repay),
    };
    bytes.iter().take(16).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join("")
}

fn find_borrower<'a>(
    borrowers: &'a [BorrowerFixture],
    hit: &TriggerHit,
) -> Option<&'a BorrowerFixture> {
    borrowers.iter().find(|b| b.account.to_pubkey() == hit.account)
}

fn build_opportunity(
    hit: &TriggerHit,
    borrower: &BorrowerFixture,
    slot: u64,
    profit: &ProfitabilityCalculator,
) -> OpportunityRecord {
    let meta = borrower_to_meta(borrower).expect("borrower meta");
    let plan = &borrower.plan;
    let input = ProfitInput {
        gross_profit_usd_micro: (plan.gross_profit_usd * 1_000_000.0) as i64,
        swap_cost_usd_micro: 100_000,
        chain_cost_usd_micro: 50_000,
        capital_used_usd_micro: (plan.capital_usd * 1_000_000.0) as u64,
        notional_usd_micro: (plan.notional_usd * 1_000_000.0) as u64,
    };
    let decision = profit.evaluate(&input);
    let decision_s = match &decision {
        ProfitDecision::Accept { net_profit_usd_micro, roi_bps } => {
            format!("Accept(net={net_profit_usd_micro},roi_bps={roi_bps})")
        }
        ProfitDecision::Reject {
            reason,
            net_profit_usd_micro,
        } => format!("Reject({reason:?},net={net_profit_usd_micro})"),
    };
    let expected = match decision {
        ProfitDecision::Accept {
            net_profit_usd_micro,
            ..
        } => net_profit_usd_micro,
        ProfitDecision::Reject {
            net_profit_usd_micro,
            ..
        } => net_profit_usd_micro,
    };

    OpportunityRecord {
        source: "replay",
        protocol: format!("{:?}", meta.protocol),
        account: meta.account.to_string(),
        asset: hit.asset.to_string(),
        side: format!("{:?}", hit.side),
        trigger_price_usd: hit.trigger_price.to_f64(),
        new_price_usd: hit.new_price.to_f64(),
        health: meta.health.to_f64(),
        band: meta.band.as_str().into(),
        plan_ixs: plan_ixs(meta.protocol),
        ix_data_preview_hex: encode_preview(meta.protocol, plan.repay_amount),
        notional_usd_micro: input.notional_usd_micro,
        expected_profit_usd_micro: expected,
        profit_decision: decision_s,
        slot,
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
    info!(
        ticks = ticks.ticks.len(),
        borrowers = borrowers_file.borrowers.len(),
        dir = %dir.display(),
        "loaded fixtures"
    );

    let index = Arc::new(CandidateIndex::new());
    for b in &borrowers_file.borrowers {
        index.upsert_candidate(borrower_to_meta(b)?);
        index.set_triggers(b.account.to_pubkey(), borrower_triggers(b)?);
    }

    let metrics = Arc::new(Metrics::new());
    let path = OracleTriggerPath::new(index.clone(), metrics.clone());
    let profit = ProfitabilityCalculator::new(ProfitConfig::default());

    // Seed prices from first tick per asset so crossings are detected.
    let mut seeded = std::collections::HashSet::new();
    for t in &ticks.ticks {
        let asset = t.asset.to_pubkey();
        if seeded.insert(asset) {
            path.prices.set(asset, PriceFx::from_f64(t.price_usd));
            path.apply_oracle_update(
                asset,
                PriceFx::from_f64(t.price_usd),
                t.slot,
                t.write_version,
                UpdateSource::Replay,
            );
        }
    }

    let events = ticks_to_events(&ticks);
    // Skip already-seeded first occurrence per asset when replaying stream.
    let mut seen = std::collections::HashSet::new();
    let mut stream_events = Vec::new();
    for ev in events {
        if let StreamEvent::Price { asset, .. } = &ev {
            if !seen.insert(*asset) {
                stream_events.push(ev);
            }
        }
    }

    let mock = Arc::new(MockGeyser::named("fixture-replay", stream_events));
    let mut opportunities = Vec::new();

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
                UpdateSource::Replay,
            );
            for hit in hits {
                if let Some(b) = find_borrower(&borrowers_file.borrowers, &hit) {
                    let rec = build_opportunity(&hit, b, slot, &profit);
                    println!("{}", serde_json::to_string(&rec)?);
                    opportunities.push(rec);
                } else {
                    info!(account = %hit.account, "trigger hit without borrower fixture");
                }
            }
        }
    }

    // Dry-run execute first accepted opportunity (smoke the execution path).
    if let Some(opp) = opportunities
        .iter()
        .find(|o| o.profit_decision.starts_with("Accept"))
    {
        let risk = Arc::new(CircuitBreaker::new(RiskLimits::default(), metrics.clone()));
        let exec = ExecutionEngine::new(
            ExecConfig {
                dry_run: true,
                rpc_url: "http://127.0.0.1:8899".into(),
                jito_block_engine_url: None,
                bid_profile: BidProfile::Balanced,
            },
            risk,
            metrics.clone(),
        );
        let tx = PreparedTx {
            label: "replay-opportunity".into(),
            protocol: opp.protocol.clone(),
            account: opp.account.clone(),
            notional_usd_micro: opp.notional_usd_micro,
            expected_profit_usd_micro: opp.expected_profit_usd_micro,
            wire: vec![],
            instructions: vec![],
            funding_strategy: None,
            ixs: opp.plan_ixs.clone(),
        };
        let res = exec.execute(&tx, 0).await?;
        info!(?res, "replay dry-run submit");
    }

    info!(
        opportunities = opportunities.len(),
        candidates = index.candidate_count(),
        triggers = index.trigger_count(),
        "replay complete"
    );
    Ok(())
}
