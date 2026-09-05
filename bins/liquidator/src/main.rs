//! Liquidator binary: config → bootstrap → subscribe/fixtures → plan loop.
//! DRY_RUN=true by default; never signs without explicit live config.

use anyhow::Context;
use liq_core::{
    programs, CandidateIndex, FundingPathEnumerator, FundingStrategy, OracleTriggerPath,
    PriceFx, ProfitConfig, ProfitDecision, Protocol, Pubkey, StateStore, UpdateSource,
};
use liq_execution::{
    evaluate_funding, opportunity_from_best, strategy_ix_labels, BidProfile, ExecConfig,
    ExecutionEngine, PreparedTx,
};
use liq_risk::{CircuitBreaker, RiskLimits};
use liq_routing::{RouteCache, StubRouter};
use liq_streaming::{
    apply_account_update, apply_raw_to_store, borrower_to_meta, borrower_triggers, load_borrowers,
    load_oracle_ticks, resolve_fixtures_dir, ticks_to_events, FixtureBootstrap, GeyserSubscriber,
    MockGeyser, RpcBootstrap, StreamEvent, SubscribeFilter, YellowstoneConfig,
    YellowstoneSubscriber,
};
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
    let fixtures_mode = use_fixtures()
        || cfg.rpc_url.contains("YOUR_")
        || std::env::var("LIQ_FIXTURES").is_ok();

    if fixtures_mode {
        let boot = FixtureBootstrap::demo_for_protocols();
        for owner in [programs::klend(), programs::save(), programs::marginfi()] {
            let accts = boot
                .get_program_accounts(&owner)
                .await
                .context("fixture bootstrap")?;
            apply_raw_to_store(&store, &accts, UpdateSource::Rpc);
        }
        info!(accounts = store.len(), "bootstrapped from fixtures/demo accounts");
    } else {
        warn!("live RPC bootstrap hook present but not enabled without credentials — using empty store");
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

                    // Build protocol-exact instruction lists when feasible (demo keys).
                    let wire_ixs = build_demo_ixs(meta.protocol, best.strategy);
                    let prepared = PreparedTx {
                        label: format!("{:?}-{:?}", meta.protocol, best.strategy),
                        protocol: format!("{:?}", meta.protocol),
                        account: format!("{}", hit.account),
                        notional_usd_micro: notional,
                        expected_profit_usd_micro: best.net_profit_usd_micro,
                        wire: vec![],
                        ixs: labels,
                        instructions: wire_ixs
                            .iter()
                            .map(|l| l.ix.clone())
                            .collect(),
                        funding_strategy: Some(best.strategy.as_str().to_string()),
                    };

                    if dry {
                        info!(
                            strategy = best.strategy.as_str(),
                            ixs = prepared.instructions.len(),
                            accepted = matches!(best.decision, ProfitDecision::Accept { .. }),
                            "DRY_RUN: planned liquidation (no submit)"
                        );
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

fn build_demo_ixs(protocol: Protocol, strategy: FundingStrategy) -> Vec<liq_core::LabeledIx> {
    match (protocol, strategy) {
        (Protocol::Save, FundingStrategy::SaveFlashLoan) => {
            let pk = |i| Pubkey::test(70, i);
            let accounts = liq_save::SaveFlashPlanAccounts {
                flash_borrow: liq_save::FlashBorrowAccounts {
                    source_liquidity: pk(1),
                    destination_liquidity: pk(2),
                    reserve: pk(3),
                    lending_market: pk(4),
                    lending_market_authority: pk(5),
                },
                liquidate: liq_save::SaveLiquidateAccounts {
                    source_liquidity: pk(2),
                    destination_collateral: pk(10),
                    destination_liquidity: pk(11),
                    repay_reserve: pk(3),
                    repay_reserve_liquidity_supply: pk(12),
                    withdraw_reserve: pk(13),
                    withdraw_reserve_collateral_mint: pk(14),
                    withdraw_reserve_collateral_supply: pk(15),
                    withdraw_reserve_liquidity_supply: pk(16),
                    withdraw_reserve_fee_receiver: pk(17),
                    obligation: pk(18),
                    lending_market: pk(4),
                    lending_market_authority: pk(5),
                    user_transfer_authority: pk(19),
                },
                flash_repay: liq_save::FlashRepayAccounts {
                    source_liquidity: pk(2),
                    destination_liquidity: pk(1),
                    fee_receiver: pk(20),
                    host_fee_receiver: pk(21),
                    reserve: pk(3),
                    lending_market: pk(4),
                    user_transfer_authority: pk(19),
                },
                refresh_reserves: vec![pk(3), pk(13)],
                obligation: pk(18),
            };
            liq_save::build_flash_atomic_plan(&accounts, 1_000_000, &[], 400_000, 1_000).labeled
        }
        (Protocol::Kamino, FundingStrategy::KaminoFlashBorrow)
        | (Protocol::Kamino, FundingStrategy::Inventory) => {
            // Minimal refresh+liquidate labels via inventory builder would need full LiquidateV2Accounts;
            // emit compute budget + refresh data as proof of non-empty builders.
            vec![
                liq_core::LabeledIx {
                    label: "ComputeBudget:SetComputeUnitLimit".into(),
                    ix: liq_core::compute_unit_limit(400_000),
                },
                liq_core::LabeledIx {
                    label: "refresh_reserve".into(),
                    ix: liq_core::Instruction::new(
                        programs::klend(),
                        vec![liq_core::AccountMeta::new(Pubkey::test(1, 5), false)],
                        liq_kamino::encode_refresh_reserve(),
                    ),
                },
                liq_core::LabeledIx {
                    label: "liquidate_v2".into(),
                    ix: liq_core::Instruction::new(
                        programs::klend(),
                        vec![liq_core::AccountMeta::new_readonly(Pubkey::test(1, 1), true)],
                        liq_kamino::encode_liquidate_v2_data(1_000, 0, 0),
                    ),
                },
            ]
        }
        (Protocol::Project0, FundingStrategy::Project0Receivership) => {
            use liq_project0::*;
            let pk = |i| Pubkey::test(8, i);
            let params = ReceivershipBuildParams {
                start: StartLiquidationAccounts {
                    marginfi_account: pk(1),
                    liquidation_record: pk(2),
                    group: pk(3),
                    liquidation_receiver: pk(4),
                    instruction_sysvar: programs::sysvar_instructions(),
                    remaining_writable: vec![pk(5)],
                },
                withdraw: WithdrawAccounts {
                    group: pk(3),
                    marginfi_account: pk(4),
                    authority: pk(4),
                    bank: pk(5),
                    vault: pk(7),
                    destination: pk(8),
                    bank_liquidity_vault_authority: pk(9),
                    token_program: programs::token(),
                },
                repay: RepayAccounts {
                    group: pk(3),
                    marginfi_account: pk(4),
                    authority: pk(4),
                    bank: pk(6),
                    signer_token_account: pk(10),
                    vault: pk(11),
                    token_program: programs::token(),
                },
                end: EndLiquidationAccounts {
                    marginfi_account: pk(1),
                    liquidation_record: pk(2),
                    group: pk(3),
                    liquidation_receiver: pk(4),
                    fee_state: pk(12),
                    global_fee_wallet: pk(13),
                    system_program: programs::system(),
                    fee_payer: None,
                },
                withdraw_amount: 1_000,
                repay_amount: 900,
                cu_limit: 500_000,
                cu_price: 1000,
            };
            build_receivership_tx(&params, &[])
        }
        _ => vec![liq_core::LabeledIx {
            label: "ComputeBudget:SetComputeUnitLimit".into(),
            ix: liq_core::compute_unit_limit(200_000),
        }],
    }
}
