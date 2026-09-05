use liq_core::{
    CandidateIndex, PriceFx, PriceTrigger, Pubkey, TriggerSide,
};
use std::time::Instant;

fn main() {
    let idx = CandidateIndex::new();
    let asset = Pubkey::test(1, 1);
    let n = 10_000usize;
    for i in 0..n {
        let acct = Pubkey::test(2, i as u64);
        idx.set_triggers(
            acct,
            vec![PriceTrigger {
                account: acct,
                asset,
                side: TriggerSide::CollateralDown,
                trigger_price: PriceFx::from_f64(100.0 + (i % 50) as f64),
            }],
        );
    }
    let t0 = Instant::now();
    let hits = idx.on_price_update(asset, PriceFx::from_f64(150.0), PriceFx::from_f64(90.0));
    let dt = t0.elapsed();
    println!(
        "bench: {} triggers, {} hits in {:?} ({:.2} ns/trigger)",
        n,
        hits.len(),
        dt,
        dt.as_nanos() as f64 / n as f64
    );
}
