use liq_core::{
    CandidateIndex, OracleTriggerPath, PriceFx, PriceTrigger, Pubkey, TriggerSide, UpdateSource,
    CandidateBand, CandidateMeta, HealthFx, Protocol,
};
use liq_streaming::{drain_all, MockGeyser, StreamEvent};
use liq_telemetry::Metrics;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let asset = Pubkey::test(2, 1);
    let acct = Pubkey::test(1, 1);
    let index = Arc::new(CandidateIndex::new());
    index.upsert_candidate(CandidateMeta {
        account: acct,
        protocol: Protocol::Kamino,
        band: CandidateBand::Hot,
        health: HealthFx::from_f64(1.04),
        assets: vec![asset],
    });
    index.set_triggers(
        acct,
        vec![PriceTrigger {
            account: acct,
            asset,
            side: TriggerSide::CollateralDown,
            trigger_price: PriceFx::from_f64(100.0),
        }],
    );
    let metrics = Arc::new(Metrics::new());
    let path = OracleTriggerPath::new(index, metrics.clone());
    path.prices.set(asset, PriceFx::from_f64(120.0));

    let mock = Arc::new(MockGeyser::new(vec![StreamEvent::Price {
        asset,
        price_fx: PriceFx::from_f64(95.0).0,
        slot: 42,
        write_version: 1,
    }]));
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
            info!(?hits, "replay trigger hits");
        }
    }
    info!(prometheus = %metrics.encode_prometheus(), "metrics");
    Ok(())
}
