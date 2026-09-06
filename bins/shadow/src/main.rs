//! Shadow mode: fixtures or `--mainnet` / `LIQ_MAINNET_SHADOW=1`.
//! Asserts DRY_RUN; never sendTransaction / Jito.

use anyhow::{bail, ensure, Context};
use liq_core::{
    CandidateIndex, FundingStrategy, OracleTriggerPath, PriceFx, ProfitConfig,
    ProfitDecision, ProfitInput, ProfitabilityCalculator, Protocol, TriggerHit, UpdateSource,
};
use liq_execution::{
    build_strategy_ixs, BidProfile, ExecConfig, ExecutionEngine, PlanAccountSet, PreparedTx,
};
use liq_risk::{CircuitBreaker, RiskLimits};
use liq_routing::JupiterQuoteBlob;
use liq_streaming::{
    borrower_to_meta, borrower_triggers, discover_mainnet, drain_all, load_borrowers,
    load_local_env_files, load_oracle_ticks, pool_from_env, resolve_fixtures_dir,
    rpc_url_configured, rpc_urls_from_env, shadow_tx_base64, ticks_to_events, BorrowerFixture,
    FixtureBootstrap, HttpJsonRpcTransport, JsonRpcBootstrap, MockGeyser,
    known, minimal_cu_limit_tx_base64_with_payer, RpcBootstrap, StreamEvent,
    YellowstoneConfig,
};
use liq_telemetry::Metrics;
use serde::Serialize;
use serde_json::json;
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

fn wants_mainnet(args: &[String]) -> bool {
    args.iter().any(|a| a == "--mainnet")
        || std::env::var("LIQ_MAINNET_SHADOW")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
}

async fn run_mainnet_shadow() -> anyhow::Result<()> {
    let loaded = load_local_env_files(None);
    for (path, keys) in &loaded {
        info!(path = %path.display(), keys = ?keys, "loaded local env (values not logged)");
    }
    ensure!(env_dry_run(), "shadow refuses DRY_RUN=false");

    let urls = rpc_urls_from_env();
    ensure!(
        !urls.is_empty() && urls.iter().any(|u| rpc_url_configured(u)),
        "mainnet shadow needs RPC_URLS or RPC_URL in config/local.env"
    );
    let pool = pool_from_env().context("rpc pool")?;
    info!(
        hosts = ?pool.stats_snapshot().iter().map(|s| &s.host).collect::<Vec<_>>(),
        "rotating RPC pool ready"
    );

    let discovery = discover_mainnet(&pool).await.context("discover_mainnet")?;
    info!(
        slot = discovery.slot,
        host = %discovery.endpoint_host,
        scanned = discovery.accounts_scanned,
        candidates = discovery.candidates.len(),
        "discovery complete"
    );

    let metrics = Arc::new(Metrics::new());
    let risk = Arc::new(CircuitBreaker::new(RiskLimits::default(), metrics.clone()));
    let exec = ExecutionEngine::new(
        ExecConfig {
            dry_run: true,
            rpc_url: pool.current_host(), // host-only placeholder; never used for broadcast in dry_run
            jito_block_engine_url: None,
            bid_profile: BidProfile::Conservative,
        },
        risk,
        metrics.clone(),
    );
    ensure!(exec.config.dry_run, "internal: dry_run");

    let boot = JsonRpcBootstrap::new(pool.clone());
    let mut simulate_results = Vec::new();

    // Pick a real mainnet account as simulate fee-payer (sigVerify=false — never signed).
    let mut sim_payer = [1u8; 32];
    let mut sim_payer_short = "synthetic".to_string();
    for cand in known::SIM_FEE_PAYER_CANDIDATES {
        if let Some(pk) = liq_core::Pubkey::from_base58(cand) {
            match boot.get_account_info(&pk).await {
                Ok(Some(a)) if a.lamports > 0 => {
                    sim_payer = a.pubkey.0;
                    sim_payer_short = liq_streaming::short_b58(cand);
                    info!(payer = %sim_payer_short, lamports = a.lamports, "simulate fee-payer selected");
                    break;
                }
                _ => continue,
            }
        }
    }


    // Prefer CRITICAL/HOT candidates from discovery; else simulate a synthetic plan proving RPC path.
    let hot: Vec<_> = discovery
        .candidates
        .iter()
        .filter(|c| {
            matches!(c.band.as_deref(), Some("CRITICAL") | Some("HOT"))
                || c.health.map(|h| h < 1.0).unwrap_or(false)
        })
        .cloned()
        .collect();

    let mut planned = 0usize;
    for cand in hot.iter().take(3) {
        let protocol = match cand.protocol.as_str() {
            "Kamino" => Protocol::Kamino,
            "Project0" => Protocol::Project0,
            "Save" => Protocol::Save,
            _ => continue,
        };
        let strategy = match protocol {
            Protocol::Kamino => FundingStrategy::KaminoFlashBorrow,
            Protocol::Save => FundingStrategy::SaveFlashLoan,
            Protocol::Project0 => FundingStrategy::Project0Receivership,
        };
        // Seed accounts from candidate short key bytes are unavailable — use deterministic seed.
        let seed = liq_core::Pubkey::test(0xA5, planned as u64);
        let accounts = PlanAccountSet::from_seed(seed);
        let built = build_strategy_ixs(
            protocol,
            strategy,
            &accounts,
            1_000_000,
            &JupiterQuoteBlob::from_env(),
        );
        let wire_ixs: Vec<_> = built.labeled.iter().map(|l| l.ix.clone()).collect();
        let planned_ixs_count = wire_ixs.len();
        let envelope = shadow_tx_base64(&wire_ixs, "11111111111111111111111111111111");
        let tx = PreparedTx {
            label: format!("mainnet-shadow-{}", cand.pubkey_short),
            protocol: cand.protocol.clone(),
            account: cand.pubkey_short.clone(),
            notional_usd_micro: 1_000_000,
            expected_profit_usd_micro: 0,
            wire: envelope.clone().into_bytes(),
            instructions: wire_ixs,
            funding_strategy: Some(strategy.as_str().to_string()),
            ixs: built.labeled.iter().map(|l| l.label.clone()).collect(),
        };
        let res = exec.execute(&tx, 0).await?;
        ensure!(res.dry_run && res.signature.is_none());
        planned += 1;

        // Plan builders emit Instruction lists; live RPC needs VersionedTransaction wire.
        // Simulate a minimal valid CU-limit vtx (sigVerify=false) as proof-of-RPC until signing is wired.
        let wire = minimal_cu_limit_tx_base64_with_payer(200_000, &sim_payer);
        let _ = envelope; // strategy envelope retained in PreparedTx path above (dry-run only)
        match boot.simulate_transaction(&wire, false).await {
            Ok(sim) => {
                info!(
                    account = %cand.pubkey_short,
                    err = ?sim.err,
                    units = ?sim.units_consumed,
                    plan_ixs = planned_ixs_count,
                    "simulateTransaction minimal vtx (sigVerify=false)"
                );
                simulate_results.push(json!({
                    "account": cand.pubkey_short,
                    "protocol": cand.protocol,
                    "rpc_ok": true,
                    "ok": sim.err.is_none(),
                    "err": sim.err,
                    "units_consumed": sim.units_consumed,
                    "log_count": sim.logs.len(),
                    "plan_ix_count": planned_ixs_count,
                    "fee_payer": sim_payer_short,
                    "note": "simulated minimal CU vtx; strategy plan not yet VersionedTransaction-encoded",
                }));
            }
            Err(e) => {
                warn!(error = %e, "simulate failed");
                simulate_results.push(json!({
                    "account": cand.pubkey_short,
                    "protocol": cand.protocol,
                    "rpc_ok": false,
                    "ok": false,
                    "error": e.to_string(),
                    "plan_ix_count": planned_ixs_count,
                }));
            }
        }
    }

    if planned == 0 {
        let wire = minimal_cu_limit_tx_base64_with_payer(200_000, &sim_payer);
        match boot.simulate_transaction(&wire, false).await {
            Ok(sim) => {
                info!(
                    err = ?sim.err,
                    units = ?sim.units_consumed,
                    "zero-candidate prove simulate (sigVerify=false)"
                );
                simulate_results.push(json!({
                    "account": null,
                    "protocol": "prove",
                    "rpc_ok": true,
                    "ok": sim.err.is_none(),
                    "err": sim.err,
                    "units_consumed": sim.units_consumed,
                    "fee_payer": sim_payer_short,
                    "note": "no HOT/CRITICAL candidates; simulated minimal CU vtx",
                }));
            }
            Err(e) => {
                simulate_results.push(json!({
                    "account": null,
                    "protocol": "prove",
                    "rpc_ok": false,
                    "ok": false,
                    "error": e.to_string(),
                }));
            }
        }
    }

    let report = json!({
        "mode": "mainnet_shadow",
        "dry_run": true,
        "broadcast": false,
        "slot": discovery.slot,
        "rpc_health": discovery.health,
        "endpoint_host": discovery.endpoint_host,
        "endpoint_stats": discovery.endpoint_stats,
        "accounts_scanned": discovery.accounts_scanned,
        "by_protocol": discovery.by_protocol,
        "known_markets": discovery.known_markets,
        "program_ids": discovery.program_ids,
        "candidates": discovery.candidates,
        "candidates_hot_critical": hot.len(),
        "plans_built": planned,
        "simulate_results": simulate_results,
        "gaps": discovery.gaps,
    });

    let out = PathBuf::from("artifacts/shadow-report.json");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, serde_json::to_vec_pretty(&report)?)?;
    info!(path = %out.display(), "wrote redacted shadow report");

    // Sanity: report must not contain api-key query strings.
    let raw = std::fs::read_to_string(&out)?;
    ensure!(
        !raw.to_lowercase().contains("api-key="),
        "refusing to leave api-key material in shadow report"
    );

    Ok(())
}

async fn run_fixture_shadow(fixtures_arg: Option<String>) -> anyhow::Result<()> {
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
            "GEYSER configured but fixture shadow uses fixtures/mock only"
        );
    }

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
            dry_run: true,
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
                    let rpc =
                        std::env::var("RPC_URL").unwrap_or_else(|_| "https://YOUR_PRIVATE_RPC".into());
                    if rpc_url_configured(&rpc) {
                        if let Ok(transport) = HttpJsonRpcTransport::new(&rpc) {
                            let boot = JsonRpcBootstrap::new(transport);
                            match boot.simulate_transaction(&envelope, false).await {
                                Ok(sim) => info!(
                                    err = ?sim.err,
                                    units = ?sim.units_consumed,
                                    "shadow simulateTransaction"
                                ),
                                Err(e) => info!(error = %e, "shadow simulate skipped"),
                            }
                        }
                    } else {
                        let boot = FixtureBootstrap::demo_for_protocols();
                        let sim = boot.simulate_transaction(&envelope, false).await?;
                        info!(logs = ?sim.logs, "shadow fixture simulate (sigVerify=false, no broadcast)");
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if wants_mainnet(&args) {
        run_mainnet_shadow().await
    } else {
        let fixtures_arg = args.into_iter().find(|a| !a.starts_with("--"));
        run_fixture_shadow(fixtures_arg).await
    }
}
