//! Shadow mode: fixtures or `--mainnet` / `LIQ_MAINNET_SHADOW=1`.
//! Asserts DRY_RUN; never sendTransaction / Jito.

use anyhow::{bail, ensure, Context};
use liq_core::{
    CandidateIndex, FundingStrategy, OracleTriggerPath, PriceFx, ProfitConfig,
    ProfitDecision, ProfitInput, ProfitabilityCalculator, Protocol, TriggerHit, UpdateSource,
};
use liq_execution::{
    build_strategy_ixs, encode_versioned_tx_base64, BidProfile, ExecConfig, ExecutionEngine,
    PlanAccountSet, PreparedTx,
};
use liq_risk::{CircuitBreaker, RiskLimits};
use liq_routing::JupiterQuoteBlob;
use liq_streaming::{
    borrower_to_meta, borrower_triggers, discover_mainnet, drain_all, load_borrowers,
    load_local_env_files, load_oracle_ticks, pool_from_env, resolve_fixtures_dir,
    rpc_url_configured, rpc_urls_from_env, shadow_tx_base64, ticks_to_events, BorrowerFixture,
    FixtureBootstrap, HttpJsonRpcTransport, JsonRpcBootstrap, MockGeyser, known, RpcBootstrap,
    StreamEvent, YellowstoneConfig,
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

    // Fee payer for unsigned sim: SHADOW_FEE_PAYER pubkey (no private key) or known default.
    let (sim_payer, sim_payer_short, fee_payer_source) = resolve_shadow_fee_payer(&boot).await?;
    info!(
        payer = %sim_payer_short,
        source = %fee_payer_source,
        "simulate fee-payer selected"
    );

    let (blockhash, bh_slot) = match boot.get_latest_blockhash().await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "getLatestBlockhash failed — using zero hash + replaceRecentBlockhash");
            ([0u8; 32], discovery.slot)
        }
    };
    info!(bh_slot, "latest blockhash fetched for strategy vtx");

    // Prefer CRITICAL/HOT candidates from discovery; limit sims (RPS-friendly).
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
    let mut extra_gaps: Vec<String> = Vec::new();
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

        let (accounts, account_notes) =
            build_plan_accounts_for_candidate(&boot, &cand, protocol, &sim_payer).await;
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
            instructions: wire_ixs.clone(),
            funding_strategy: Some(strategy.as_str().to_string()),
            ixs: built.labeled.iter().map(|l| l.label.clone()).collect(),
        };
        let res = exec.execute(&tx, 0).await?;
        ensure!(res.dry_run && res.signature.is_none());
        planned += 1;

        let vtx_note;
        let wire = match encode_versioned_tx_base64(&wire_ixs, &sim_payer, &blockhash) {
            Ok(b64) => {
                vtx_note = "strategy VersionedTransaction-encoded (v0, unsigned dummy sigs)";
                b64
            }
            Err(e) => {
                warn!(error = %e, "vtx encode failed");
                simulate_results.push(json!({
                    "account": cand.pubkey_short,
                    "protocol": cand.protocol,
                    "rpc_ok": false,
                    "ok": false,
                    "error": format!("vtx encode: {e}"),
                    "plan_ix_count": planned_ixs_count,
                    "strategy_vtx": false,
                    "account_notes": account_notes,
                }));
                continue;
            }
        };

        match boot.simulate_transaction(&wire, false).await {
            Ok(sim) => {
                let trim = |l: &String| {
                    if l.len() > 240 {
                        format!("{}…", &l[..240])
                    } else {
                        l.clone()
                    }
                };
                // Keep head + tail so CreateIdempotent and liquidate failure both visible.
                let logs_trimmed: Vec<String> = if sim.logs.len() <= 28 {
                    sim.logs.iter().map(trim).collect()
                } else {
                    let mut v: Vec<String> = sim.logs.iter().take(12).map(trim).collect();
                    v.push("…".into());
                    v.extend(sim.logs.iter().rev().take(14).collect::<Vec<_>>().into_iter().rev().map(trim));
                    v
                };
                info!(
                    account = %cand.pubkey_short,
                    err = ?sim.err,
                    units = ?sim.units_consumed,
                    plan_ixs = planned_ixs_count,
                    live = accounts.from_live_decode,
                    "simulateTransaction strategy vtx (sigVerify=false)"
                );
                simulate_results.push(json!({
                    "account": cand.pubkey_short,
                    "protocol": cand.protocol,
                    "rpc_ok": true,
                    "ok": sim.err.is_none(),
                    "err": sim.err,
                    "units_consumed": sim.units_consumed,
                    "log_count": sim.logs.len(),
                    "logs_trimmed": logs_trimmed,
                    "plan_ix_count": planned_ixs_count,
                    "fee_payer": sim_payer_short,
                    "fee_payer_source": fee_payer_source,
                    "funding_strategy": strategy.as_str(),
                    "used_flash_builder": built.used_flash_builder,
                    "from_live_decode": accounts.from_live_decode,
                    "strategy_vtx": true,
                    "note": vtx_note,
                    "account_notes": account_notes,
                    "plan_ix_labels": built.labeled.iter().map(|l| l.label.clone()).collect::<Vec<_>>(),
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
                    "strategy_vtx": true,
                    "note": vtx_note,
                    "account_notes": account_notes,
                }));
            }
        }
    }

    if planned == 0 {
        extra_gaps.push("no HOT/CRITICAL candidates to strategy-simulate".into());
        // Still prove vtx path with CU+price ixs (not "minimal CU stub" alone — labeled strategy_vtx prove).
        let prove_ixs = vec![
            liq_core::compute_unit_limit(200_000),
            liq_core::compute_unit_price(1_000),
        ];
        match encode_versioned_tx_base64(&prove_ixs, &sim_payer, &blockhash) {
            Ok(wire) => match boot.simulate_transaction(&wire, false).await {
                Ok(sim) => {
                    simulate_results.push(json!({
                        "account": null,
                        "protocol": "prove",
                        "rpc_ok": true,
                        "ok": sim.err.is_none(),
                        "err": sim.err,
                        "units_consumed": sim.units_consumed,
                        "fee_payer": sim_payer_short,
                        "fee_payer_source": fee_payer_source,
                        "strategy_vtx": true,
                        "note": "no HOT/CRITICAL candidates; simulated VersionedTransaction CU ixs (not strategy plan)",
                    }));
                }
                Err(e) => {
                    simulate_results.push(json!({
                        "account": null,
                        "protocol": "prove",
                        "rpc_ok": false,
                        "ok": false,
                        "error": e.to_string(),
                        "strategy_vtx": true,
                    }));
                }
            },
            Err(e) => extra_gaps.push(format!("prove vtx encode failed: {e}")),
        }
    }

    let mut gaps = discovery.gaps.clone();
    gaps.extend(extra_gaps);
    gaps.push(format!(
        "shadow fee payer source={fee_payer_source}; SHADOW_FEE_PAYER is pubkey-only (never a private key)"
    ));
    // Progress note filled after sims when possible; keep a baseline gap marker.
    let kamino_progress = summarize_kamino_sim_progress(&simulate_results);
    gaps.push(kamino_progress);

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
        "strategy_vtx_encoding": true,
        "fee_payer_source": fee_payer_source,
        "gaps": gaps,
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



fn summarize_kamino_sim_progress(simulate_results: &[serde_json::Value]) -> String {
    let mut best = "Kamino shadow: no strategy sims".to_string();
    // Prefer sims that reached on-chain InstructionError over RPC encode/size failures.
    let mut kamino: Vec<&serde_json::Value> = simulate_results
        .iter()
        .filter(|s| s.get("protocol").and_then(|p| p.as_str()) == Some("Kamino"))
        .collect();
    kamino.sort_by_key(|s| {
        let err = s.get("err");
        let has_ix_err = err
            .map(|e| !e.is_null() && e.to_string().contains("InstructionError"))
            .unwrap_or(false);
        let ok = s.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        // ok first, then InstructionError, then others
        if ok {
            0
        } else if has_ix_err {
            1
        } else {
            2
        }
    });
    for s in kamino {
        let ok = s.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let units = s
            .get("units_consumed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let err = s.get("err").cloned().unwrap_or(serde_json::Value::Null);
        let logs = s
            .get("logs_trimmed")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let labels_hint = s
            .get("account_notes")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .filter(|t| t.contains("CreateIdempotent") || t.contains("liquidator_atas") || t.contains("missing"))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        if ok {
            return format!(
                "Kamino progress: strategy vtx sim SUCCEEDED (units={units}); CreateIdempotent path cleared AccountNotInitialized"
            );
        }
        // Prefer concrete InstructionError
        let err_s = err.to_string();
        if err.is_null() || err_s == "null" {
            // e.g. vtx too large — keep scanning for a better sample
            best = format!(
                "Kamino progress: sim without InstructionError (rpc/encode); notes={labels_hint}; units={units}"
            );
            continue;
        }
        let past_3012 = !err_s.contains("3012");
        let past_6009 = !err_s.contains("6009");
        let create_seen = logs.iter().any(|l| {
            l.as_str()
                .map(|t| t.contains("CreateIdempotent") || t.contains("Associated Token"))
                .unwrap_or(false)
        });
        best = format!(
            "Kamino progress: CreateIdempotent ATAs before flash/liquidate (seen_in_logs={create_seen}); sim err={err_s}; units={units}; notes={labels_hint}"
        );
        if past_3012 {
            let named = if err_s.contains("6009") {
                "Custom 6009 ReserveStale (reserve needs refresh — often post-flash_borrow before liquidate)"
            } else if err_s.contains("6017") {
                "Custom 6017 ObligationStale"
            } else if err_s.contains("6016") {
                "Custom 6016 ObligationHealthy (candidate selection/health decode — flash+refresh path past ReserveStale)"
            } else {
                "see err"
            };
            let cleared = if past_6009 && !err_s.contains("6009") {
                "past 3012 AccountNotInitialized and 6009 ReserveStale"
            } else {
                "past liquidate AccountNotInitialized (3012)"
            };
            return format!(
                "Kamino progress: {cleared}; next={named}; raw={err_s}; units={units}"
            );
        }
    }
    best
}

async fn resolve_shadow_fee_payer<T: liq_streaming::JsonRpcTransport>(
    boot: &JsonRpcBootstrap<T>,
) -> anyhow::Result<(liq_core::Pubkey, String, &'static str)> {
    if let Ok(s) = std::env::var("SHADOW_FEE_PAYER") {
        let t = s.trim();
        if !t.is_empty() {
            if let Some(pk) = liq_core::Pubkey::from_base58(t) {
                return Ok((
                    pk,
                    liq_streaming::short_b58(t),
                    "SHADOW_FEE_PAYER env",
                ));
            }
            warn!(value_len = t.len(), "SHADOW_FEE_PAYER not valid base58 — falling back");
        }
    }
    for cand in known::SIM_FEE_PAYER_CANDIDATES {
        if let Some(pk) = liq_core::Pubkey::from_base58(cand) {
            match boot.get_account_info(&pk).await {
                Ok(Some(a)) if a.lamports > 0 => {
                    return Ok((
                        a.pubkey,
                        liq_streaming::short_b58(cand),
                        "known SIM_FEE_PAYER_CANDIDATES",
                    ));
                }
                _ => continue,
            }
        }
    }
    // Documented default (Save market owner) — may AccountNotFound; still labeled.
    let default = known::SIM_FEE_PAYER_CANDIDATES[0];
    let pk = liq_core::Pubkey::from_base58(default)
        .context("default fee payer decode")?;
    Ok((
        pk,
        liq_streaming::short_b58(default),
        "default documented placeholder (Save market owner)",
    ))
}

async fn build_plan_accounts_for_candidate<T: liq_streaming::JsonRpcTransport>(
    boot: &JsonRpcBootstrap<T>,
    cand: &liq_streaming::DiscoveredAccount,
    protocol: Protocol,
    fee_payer: &liq_core::Pubkey,
) -> (PlanAccountSet, Vec<String>) {
    let mut notes = Vec::new();
    if protocol != Protocol::Kamino {
        notes.push("non-Kamino: PlanAccountSet::from_seed (live metas TBD)".into());
        let seed = cand
            .pubkey
            .as_deref()
            .and_then(liq_core::Pubkey::from_base58)
            .unwrap_or_else(|| liq_core::Pubkey::test(0xA5, 1));
        let mut a = PlanAccountSet::from_seed(seed);
        a.liquidator = *fee_payer;
        return (a, notes);
    }
    let Some(pk_s) = cand.pubkey.as_deref() else {
        notes.push("missing full pubkey on candidate — seed fallback".into());
        let mut a = PlanAccountSet::from_seed(liq_core::Pubkey::test(0xA5, 2));
        a.liquidator = *fee_payer;
        return (a, notes);
    };
    let Some(obl_pk) = liq_core::Pubkey::from_base58(pk_s) else {
        notes.push("bad obligation pubkey".into());
        let mut a = PlanAccountSet::from_seed(liq_core::Pubkey::test(0xA5, 3));
        a.liquidator = *fee_payer;
        return (a, notes);
    };
    let obl_acct = match boot.get_account_info(&obl_pk).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            notes.push("obligation account missing".into());
            let mut a = PlanAccountSet::from_seed(obl_pk);
            a.liquidator = *fee_payer;
            return (a, notes);
        }
        Err(e) => {
            notes.push(format!("get_account_info obl: {e}"));
            let mut a = PlanAccountSet::from_seed(obl_pk);
            a.liquidator = *fee_payer;
            return (a, notes);
        }
    };
    let positions = match liq_kamino::decode_obligation_live_positions(obl_pk, &obl_acct.data) {
        Ok(p) => p,
        Err(e) => {
            notes.push(format!("live_positions decode failed: {e}"));
            let mut a = PlanAccountSet::from_seed(obl_pk);
            a.liquidator = *fee_payer;
            a.lending_market = liq_core::Pubkey::from_base58(known::KLEND_MAIN_MARKET)
                .unwrap_or(a.lending_market);
            return (a, notes);
        }
    };
    notes.push(format!(
        "live_positions deposits={} borrows={}",
        positions.deposits.len(),
        positions.borrows.len()
    ));
    let Some((repay_pk, withdraw_pk)) = liq_kamino::pick_liquidate_reserves(&positions) else {
        notes.push("no repay/withdraw reserves in positions".into());
        let mut a = PlanAccountSet::from_seed(obl_pk);
        a.liquidator = *fee_payer;
        a.lending_market = positions.header.lending_market;
        a.obligation = obl_pk;
        return (a, notes);
    };
    // Fetch all unique reserves for refresh metas + repay/withdraw vaults.
    let mut reserve_keys = Vec::new();
    for d in &positions.deposits {
        if !reserve_keys.contains(&d.reserve) {
            reserve_keys.push(d.reserve);
        }
    }
    for b in &positions.borrows {
        if !reserve_keys.contains(&b.reserve) {
            reserve_keys.push(b.reserve);
        }
    }
    for extra in [repay_pk, withdraw_pk] {
        if !reserve_keys.contains(&extra) {
            reserve_keys.push(extra);
        }
    }
    let fetched = match boot.get_multiple_accounts(&reserve_keys).await {
        Ok(v) => v,
        Err(e) => {
            notes.push(format!("getMultipleAccounts reserves: {e}"));
            let mut a = PlanAccountSet::from_seed(obl_pk);
            a.liquidator = *fee_payer;
            a.obligation = obl_pk;
            a.lending_market = positions.header.lending_market;
            a.repay_reserve = repay_pk;
            a.withdraw_reserve = withdraw_pk;
            return (a, notes);
        }
    };
    let mut vaults = std::collections::HashMap::new();
    for (i, key) in reserve_keys.iter().enumerate() {
        if let Some(Some(raw)) = fetched.get(i) {
            match liq_kamino::decode_reserve_live_vaults(*key, &raw.data) {
                Ok(v) => {
                    vaults.insert(*key, v);
                }
                Err(e) => notes.push(format!("reserve decode {}: {e}", liq_streaming::short_b58(&key.to_base58()))),
            }
        } else {
            notes.push(format!("reserve missing {}", liq_streaming::short_b58(&key.to_base58())));
        }
    }
    let (Some(repay_v), Some(withdraw_v)) = (vaults.get(&repay_pk), vaults.get(&withdraw_pk)) else {
        notes.push("repay/withdraw vault decode incomplete".into());
        let mut a = PlanAccountSet::from_seed(obl_pk);
        a.liquidator = *fee_payer;
        a.obligation = obl_pk;
        a.repay_reserve = repay_pk;
        a.withdraw_reserve = withdraw_pk;
        return (a, notes);
    };
    notes.push(format!(
        "live_vaults repay_mint={} withdraw_coll={} n_reserves={}",
        liq_streaming::short_b58(&repay_v.liquidity_mint.to_base58()),
        liq_streaming::short_b58(&withdraw_v.collateral_mint.to_base58()),
        vaults.len()
    ));
    let market_auth = liq_kamino::lending_market_authority(&positions.header.lending_market);
    let mut accounts = PlanAccountSet::from_kamino_live(
        obl_pk,
        *fee_payer,
        &positions,
        repay_v,
        withdraw_v,
        market_auth,
    );
    // Overlay full refresh metas: deposits then borrows (matches refresh_obligation remaining).
    let mut refresh = Vec::new();
    let mut seen_r = Vec::new();
    for key in positions.deposits.iter().map(|d| d.reserve)
        .chain(positions.borrows.iter().map(|b| b.reserve))
    {
        if seen_r.contains(&key) {
            continue;
        }
        let Some(r) = vaults.get(&key) else { continue };
        seen_r.push(key);
        refresh.push(liq_kamino::RefreshReserveAccounts {
            reserve: r.address,
            lending_market: r.lending_market,
            pyth_oracle: r.pyth_oracle,
            switchboard_price: r.switchboard_price,
            switchboard_twap: r.switchboard_twap,
            scope_prices: r.scope_prices,
        });
    }
    accounts.refresh_reserve_metas = refresh;
    if let Some(ref_pk) = accounts.referrer {
        notes.push(format!(
            "referrer={} referrer_token_states={}",
            liq_streaming::short_b58(&ref_pk.to_base58()),
            accounts.referrer_token_states.len()
        ));
    }
    notes.push(format!(
        "ATAs liq={} coll={}",
        liq_streaming::short_b58(&accounts.user_liquidity.to_base58()),
        liq_streaming::short_b58(&accounts.user_collateral.to_base58())
    ));
    notes.push("PlanAccountSet from live Klend positions+reserves (ATA+farm+referrer wiring)".into());
    // Prefetch liquidator ATAs + referrer token states (existence noted; no secrets).
    // Missing liquidator ATAs → CreateIdempotent in planner before flash/liquidate.
    let mut ata_keys = vec![accounts.user_liquidity, accounts.user_collateral];
    if let Some(d) = accounts.user_destination_liquidity {
        if !ata_keys.contains(&d) {
            ata_keys.push(d);
        }
    }
    let mut prefetch = ata_keys.clone();
    prefetch.extend(accounts.referrer_token_states.iter().copied());
    match boot.get_multiple_accounts(&prefetch).await {
        Ok(fetched) => {
            let present = fetched.iter().flatten().count();
            notes.push(format!(
                "prefetched_ata_referrer_accounts present={}/{}",
                present,
                prefetch.len()
            ));
            let mut missing = Vec::new();
            for (i, key) in ata_keys.iter().enumerate() {
                let exists = fetched.get(i).and_then(|a| a.as_ref()).is_some();
                if !exists {
                    missing.push(*key);
                }
            }
            notes.push(format!(
                "liquidator_atas missing={}/{} (CreateIdempotent will run for missing)",
                missing.len(),
                ata_keys.len()
            ));
            // Always CreateIdempotent is fine; filter to missing saves CU when some exist.
            accounts.missing_ata_filter = Some(missing);
        }
        Err(e) => {
            notes.push(format!("prefetch ATA/referrer: {e}"));
            // Fall back to creating all liquidator ATAs (idempotent).
            accounts.missing_ata_filter = None;
        }
    }
    (accounts, notes)
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
