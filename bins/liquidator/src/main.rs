//! Liquidator binary: config → bootstrap → subscribe/fixtures → plan loop.
//! DRY_RUN=true by default; never signs without explicit live config.

use anyhow::Context;
use liq_core::{
    programs, CandidateIndex, FundingPathEnumerator, OracleTriggerPath,
    PriceFx, ProfitConfig, ProfitDecision, Pubkey, StateStore, UpdateSource,
};
use liq_execution::{
    build_strategy_ixs, evaluate_funding, opportunity_from_best, strategy_ix_labels, BidProfile,
    ExecConfig, ExecutionEngine, PlanAccountSet, PreparedTx,
};
use liq_risk::{CircuitBreaker, RiskLimits};
use liq_routing::{RouteCache, StubRouter};
use liq_streaming::{
    apply_account_update, apply_raw_to_store, borrower_to_meta, borrower_triggers, discover_mainnet,
    load_borrowers, load_local_env_files, load_oracle_ticks, pool_from_env, resolve_fixtures_dir,
    rpc_url_configured, rpc_urls_from_env, shadow_tx_base64, ticks_to_events, FixtureBootstrap,
    GeyserSubscriber, HttpJsonRpcTransport, JsonRpcBootstrap, MockGeyser, RpcBootstrap,
    StreamEvent, SubscribeFilter, YellowstoneConfig, YellowstoneSubscriber,
};
use liq_routing::JupiterQuoteBlob;
use liq_telemetry::Metrics;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

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
    #[serde(default)]
    loop_ticks: Option<u64>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            dry_run: true,
            rpc_url: "https://YOUR_PRIVATE_RPC".into(),
            jito_block_engine_url: None,
            geyser_endpoint: None,
            min_profit_usd: 0.5,
            loop_ticks: None,
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

#[allow(dead_code)]
fn use_fixtures() -> bool {
    std::env::var("LIQ_FIXTURES").is_ok()
        || PathBuf::from("fixtures/oracle_ticks.json").exists()
        || resolve_fixtures_dir(None).join("oracle_ticks.json").exists()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    for (path, keys) in load_local_env_files(None) {
        info!(path = %path.display(), keys = ?keys, "loaded local env (values not logged)");
    }

    let cfg = load_config();
    let dry = std::env::var("DRY_RUN")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(cfg.dry_run);

    let rpc_log = std::env::var("RPC_URL")
        .ok()
        .filter(|u| rpc_url_configured(u))
        .map(|u| liq_streaming::rpc_url_host_only(&u))
        .unwrap_or_else(|| liq_streaming::rpc_url_host_only(&cfg.rpc_url));
    info!(
        dry_run = dry,
        rpc_host = %rpc_log,
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
    let funding_enum = FundingPathEnumerator::new(profit_cfg.clone());
    let _router = StubRouter;
    let route_cache = RouteCache::new();

    let exec = ExecutionEngine::new(
        ExecConfig {
            dry_run: dry,
            rpc_url: cfg.rpc_url.clone(),
            jito_block_engine_url: cfg.jito_block_engine_url.clone(),
            bid_profile: BidProfile::Balanced,
        },
        risk,
        metrics.clone(),
    );

    // --- State store + bootstrap ---
    let store: Arc<StateStore<Vec<u8>>> = Arc::new(StateStore::new());
    // LIQ_FIXTURES forces offline CI path; otherwise prefer live RPC_URL/RPC_URLS from local.env.
    let force_fixtures = std::env::var("LIQ_FIXTURES").is_ok();
    let env_urls = rpc_urls_from_env();
    let live_configured = env_urls.iter().any(|u| rpc_url_configured(u))
        || rpc_url_configured(&cfg.rpc_url);
    let rpc_url = env_urls
        .first()
        .cloned()
        .unwrap_or_else(|| cfg.rpc_url.clone());

    if force_fixtures || !live_configured {
        let boot = FixtureBootstrap::demo_for_protocols();
        for owner in [programs::klend(), programs::save(), programs::marginfi()] {
            let accts = boot
                .get_program_accounts(&owner)
                .await
                .context("fixture bootstrap")?;
            apply_raw_to_store(&store, &accts, UpdateSource::Rpc);
        }
        info!(accounts = store.len(), "bootstrapped from fixtures (offline CI path)");
    } else {
        let pool = pool_from_env().or_else(|_| liq_streaming::RotatingRpcPool::from_urls(vec![rpc_url.clone()]));
        match pool {
            Ok(pool) => {
                info!(host = %pool.current_host(), n = pool.len(), "live rotating RPC bootstrap");
                match discover_mainnet(&pool).await {
                    Ok(rep) => {
                        info!(
                            slot = rep.slot,
                            scanned = rep.accounts_scanned,
                            candidates = rep.candidates.len(),
                            host = %rep.endpoint_host,
                            "mainnet discovery bootstrap (live over Pubkey::test demo)"
                        );
                        // Warm store with known market accounts (scoped, not full GPA dump).
                        let boot = JsonRpcBootstrap::new(pool.clone());
                        let keys: Vec<_> = [
                            liq_streaming::known::KLEND_MAIN_MARKET,
                            liq_streaming::known::MARGINFI_MAIN_GROUP,
                            liq_streaming::known::SAVE_MAIN_MARKET,
                        ]
                        .iter()
                        .filter_map(|s| liq_core::Pubkey::from_base58(s))
                        .collect();
                        if let Ok(accts) = boot.get_multiple_accounts(&keys).await {
                            let raw: Vec<_> = accts.into_iter().flatten().collect();
                            apply_raw_to_store(&store, &raw, UpdateSource::Rpc);
                            info!(n = raw.len(), "known market/group accounts stored");
                        }
                        let _ = rep;
                    }
                    Err(e) => {
                        warn!(error = %e, "discover_mainnet failed — fixture fallback");
                        let boot = FixtureBootstrap::demo_for_protocols();
                        for owner in [programs::klend(), programs::save(), programs::marginfi()] {
                            let accts = boot.get_program_accounts(&owner).await?;
                            apply_raw_to_store(&store, &accts, UpdateSource::Rpc);
                        }
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "RPC pool unavailable — falling back to fixtures");
                let boot = FixtureBootstrap::demo_for_protocols();
                for owner in [programs::klend(), programs::save(), programs::marginfi()] {
                    let accts = boot.get_program_accounts(&owner).await?;
                    apply_raw_to_store(&store, &accts, UpdateSource::Rpc);
                }
            }
        }
    }

    // Prefetch HOT route cache structure (empty quotes OK).
    route_cache.prefetch_hot(
        &[(Pubkey::test(1, 1), Pubkey::test(1, 2))],
        &[liq_routing::Quote {
            amount_out: 0,
            cost_usd_micro: 0,
            route_label: "prefetch-slot".into(),
        }],
    );

    // --- Load candidate fixtures when available ---
    let dir = resolve_fixtures_dir(std::env::var("LIQ_FIXTURES").ok().as_deref());
    let borrowers = if dir.join("borrowers.json").exists() {
        let file = load_borrowers(dir.join("borrowers.json")).context("borrowers")?;
        for b in &file.borrowers {
            if let Ok(meta) = borrower_to_meta(b) {
                index.upsert_candidate(meta);
            }
            if let Ok(trigs) = borrower_triggers(b) {
                index.set_triggers(b.account.to_pubkey(), trigs);
            }
        }
        info!(n = file.borrowers.len(), "loaded borrower fixtures into candidate index");
        file.borrowers
    } else {
        vec![]
    };

    // --- Stream: fixtures mock or Yellowstone stub ---
    let events: Vec<StreamEvent> = if dir.join("oracle_ticks.json").exists() {
        let ticks = load_oracle_ticks(dir.join("oracle_ticks.json"))?;
        ticks_to_events(&ticks)
    } else {
        vec![StreamEvent::Price {
            asset: Pubkey::test(9, 1),
            price_fx: PriceFx::from_f64(110.0).0,
            slot: 1,
            write_version: 1,
        }]
    };

    if let Some(ys_cfg) = YellowstoneConfig::from_env() {
        info!(
            endpoint = %ys_cfg.endpoint,
            has_token = ys_cfg.has_credentials(),
            "Yellowstone config present — live subscribe still behind stub until gRPC client linked"
        );
        let ys = YellowstoneSubscriber::new(ys_cfg);
        if let Err(e) = ys.subscribe(SubscribeFilter::default()).await {
            info!(error = %e, "Yellowstone stub not live; continuing with mock/fixture stream");
        }
    } else if cfg.geyser_endpoint.is_some() {
        info!("geyser_endpoint in config but GEYSER_ENDPOINT env unset — using fixtures/mock");
    }

    let mock = MockGeyser::named("liquidator-loop", events.clone());
    let mut rx = mock
        .subscribe(SubscribeFilter::default())
        .await
        .context("subscribe mock")?;

    let max_ticks = std::env::var("LIQ_LOOP_TICKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .or(cfg.loop_ticks)
        .unwrap_or(if dry {
            events.len().max(1) as u64
        } else {
            u64::MAX
        });

    let mut ticks_done = 0u64;
    info!(max_ticks, dry_run = dry, "entering ingestion→plan loop");

    while ticks_done < max_ticks {
        let ev = match rx.recv().await {
            Some(e) => e,
            None => {
                // Re-arm mock stream for continued dry loops
                if dry && ticks_done + 1 < max_ticks {
                    let mock2 = MockGeyser::named("liquidator-loop-replay", events.clone());
                    rx = mock2.subscribe(SubscribeFilter::default()).await?;
                    continue;
                }
                break;
            }
        };

        match ev {
            StreamEvent::Account(upd) => {
                apply_account_update(&store, &upd);
                metrics.oracle_updates.inc(); // account path
            }
            StreamEvent::Slot(s) => {
                info!(slot = s.slot, "slot update");
            }
            StreamEvent::Price {
                asset,
                price_fx,
                slot,
                write_version,
            } => {
                let price = PriceFx(price_fx);
                let hits = oracle_path.apply_oracle_update(
                    asset,
                    price,
                    slot,
                    write_version,
                    UpdateSource::Mock,
                );
                info!(%asset, price = price.to_f64(), hits = hits.len(), slot, "oracle tick");

                for hit in hits {
                    let meta = match index.get(&hit.account) {
                        Some(m) => m,
                        None => continue,
                    };
                    let borrower = borrowers
                        .iter()
                        .find(|b| b.account.to_pubkey() == hit.account);
                    let (gross, notional, capital) = if let Some(b) = borrower {
                        (
                            (b.plan.gross_profit_usd * 1_000_000.0) as i64,
                            (b.plan.notional_usd * 1_000_000.0) as u64,
                            (b.plan.capital_usd * 1_000_000.0) as u64,
                        )
                    } else {
                        (2_000_000, 50_000_000, 50_000_000)
                    };

                    let plan = evaluate_funding(
                        &funding_enum,
                        meta.protocol,
                        gross,
                        100_000,
                        80_000,
                        50_000,
                        notional,
                        capital,
                        9,
                    );
                    let Some(best) = plan.best.clone() else {
                        continue;
                    };
                    let labels = strategy_ix_labels(best.strategy, meta.protocol);
                    let opp = opportunity_from_best(
                        dry,
                        &format!("{}", hit.account),
                        &plan,
                        &best,
                        labels.clone(),
                        slot,
                    );
                    let opp_json = serde_json::to_string(&opp).unwrap_or_default();
                    info!(%opp_json, "opportunity");

                    // Structured accounts from borrower key (fixtures/config shaped), not inside builders.
                    let plan_accounts = PlanAccountSet::from_seed(hit.account);
                    let swap_blob = JupiterQuoteBlob::from_env();
                    let amount = borrower.map(|b| b.plan.repay_amount).unwrap_or(1_000_000);
                    let planned = build_strategy_ixs(
                        meta.protocol,
                        best.strategy,
                        &plan_accounts,
                        amount,
                        &swap_blob,
                    );
                    let wire_ixs = planned.labeled;
                    let envelope_b64 = shadow_tx_base64(
                        &wire_ixs.iter().map(|l| l.ix.clone()).collect::<Vec<_>>(),
                        "11111111111111111111111111111111",
                    );
                    let prepared = PreparedTx {
                        label: format!("{:?}-{:?}", meta.protocol, best.strategy),
                        protocol: format!("{:?}", meta.protocol),
                        account: format!("{}", hit.account),
                        notional_usd_micro: notional,
                        expected_profit_usd_micro: best.net_profit_usd_micro,
                        wire: envelope_b64.into_bytes(),
                        ixs: labels,
                        instructions: wire_ixs.iter().map(|l| l.ix.clone()).collect(),
                        funding_strategy: Some(best.strategy.as_str().to_string()),
                    };

                    if dry {
                        info!(
                            strategy = best.strategy.as_str(),
                            ixs = prepared.instructions.len(),
                            flash_builder = planned.used_flash_builder,
                            swap_incomplete = planned.swap_incomplete,
                            accepted = matches!(best.decision, ProfitDecision::Accept { .. }),
                            "DRY_RUN/shadow: planned liquidation (no broadcast)"
                        );
                        // Optional simulate via RPC when configured (sigVerify=false); never broadcast.
                        if rpc_url_configured(&rpc_url) {
                            if let Ok(transport) = HttpJsonRpcTransport::new(&rpc_url) {
                                let boot = JsonRpcBootstrap::new(transport);
                                let b64 = String::from_utf8_lossy(&prepared.wire);
                                match boot.simulate_transaction(&b64, false).await {
                                    Ok(sim) => info!(
                                        err = ?sim.err,
                                        units = ?sim.units_consumed,
                                        "shadow simulateTransaction (sigVerify=false)"
                                    ),
                                    Err(e) => info!(error = %e, "simulate skipped/failed"),
                                }
                            }
                        }
                    } else {
                        match exec.execute(&prepared, 0).await {
                            Ok(res) => info!(?res, "submit result"),
                            Err(e) => warn!(error = %e, "submit failed"),
                        }
                    }
                }
            }
        }

        ticks_done += 1;
        if dry {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    info!(
        ticks = ticks_done,
        store = store.len(),
        routes = route_cache.len(),
        "liquidator loop complete"
    );
    let _ = (_router, funding_enum);
    Ok(())
}

