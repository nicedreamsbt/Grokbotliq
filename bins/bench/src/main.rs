//! Local microbenchmarks: candidate lookup, health recompute, ix encode.

use liq_core::{
    amount_to_usd_micro, health_factor_ratio, CandidateIndex, HealthFx, PriceFx, PriceTrigger,
    Pubkey, TriggerSide,
};
use liq_kamino::{
    encode_liquidate_v2_data, encode_refresh_obligation, encode_refresh_reserve, is_liquidatable,
    KaminoBorrow, KaminoDeposit, KaminoObligation, PriceMap,
};
use liq_project0::encode_classic_liquidate;
use liq_save::encode_liquidate_and_redeem;
use std::time::Instant;

fn ns_per(iters: usize, elapsed: std::time::Duration) -> f64 {
    elapsed.as_nanos() as f64 / iters as f64
}

fn bench_candidate_lookup(n: usize) -> (usize, std::time::Duration, f64) {
    let idx = CandidateIndex::new();
    let asset = Pubkey::test(1, 1);
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
    (hits.len(), dt, ns_per(n, dt))
}

fn bench_health_recompute(iters: usize) -> std::time::Duration {
    let coll = Pubkey::test(1, 1);
    let debt = Pubkey::test(1, 2);
    let obl = KaminoObligation {
        address: Pubkey::test(2, 1),
        market: Pubkey::test(2, 2),
        deposits: vec![KaminoDeposit {
            reserve: Pubkey::test(3, 1),
            mint: coll,
            deposited_amount: 10_000_000_000,
            decimals: 9,
            liq_threshold_bps: 8000,
        }],
        borrows: vec![KaminoBorrow {
            reserve: Pubkey::test(3, 2),
            mint: debt,
            borrowed_amount: 700_000_000,
            decimals: 6,
        }],
    };
    let prices = PriceMap {
        prices: vec![
            (coll, PriceFx::from_f64(100.0)),
            (debt, PriceFx::from_f64(1.0)),
        ],
    };
    let t0 = Instant::now();
    for _ in 0..iters {
        let _ = is_liquidatable(&obl, &prices).unwrap();
        let coll_usd = amount_to_usd_micro(10_000_000_000, 9, PriceFx::from_f64(100.0));
        let borrow_usd = amount_to_usd_micro(700_000_000, 6, PriceFx::from_f64(1.0));
        let _hf: HealthFx = health_factor_ratio(coll_usd * 8000 / 10_000, borrow_usd);
    }
    t0.elapsed()
}

fn bench_ix_encode(iters: usize) -> (std::time::Duration, usize) {
    let t0 = Instant::now();
    let mut bytes = 0usize;
    for i in 0..iters {
        let a = encode_liquidate_v2_data(1_000_000 + i as u64, 0, 0);
        let b = encode_refresh_reserve();
        let c = encode_refresh_obligation();
        let d = encode_classic_liquidate(42 + i as u64);
        let e = encode_liquidate_and_redeem(100 + i as u64);
        bytes += a.len() + b.len() + c.len() + d.len() + e.len();
    }
    (t0.elapsed(), bytes)
}

fn main() {
    let n = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000usize);
    let health_iters = (n * 10).max(50_000);
    let encode_iters = (n * 10).max(50_000);

    println!("=== Grokbotliq microbench (n={n}) ===");

    let (hits, dt_lookup, ns_trig) = bench_candidate_lookup(n);
    println!(
        "candidate_lookup: {n} triggers, {hits} hits in {dt_lookup:?} ({ns_trig:.2} ns/trigger)"
    );

    let dt_health = bench_health_recompute(health_iters);
    println!(
        "health_recompute: {health_iters} iters in {dt_health:?} ({:.2} ns/iter)",
        ns_per(health_iters, dt_health)
    );

    let (dt_enc, total_bytes) = bench_ix_encode(encode_iters);
    println!(
        "ix_encode: {encode_iters} iters ({total_bytes} bytes) in {dt_enc:?} ({:.2} ns/iter)",
        ns_per(encode_iters, dt_enc)
    );

    println!("ok");
}
