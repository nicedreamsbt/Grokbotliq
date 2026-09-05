# Benchmarks — Grokbotliq

Local microbenchmarks from `cargo run -p bench --release`.

**Host:** Linux box (shared agent VM), measured **2026-09-05 ~13:25 PT** (UTC-7).  
**Command:** `cargo run -p bench --release -- 10000`  
**Build:** release, rustc from `rust-toolchain.toml`

## Results

| Bench | Workload | Wall time | Unit cost |
|-------|----------|-----------|-----------|
| **candidate_lookup** | 10 000 collateral-down triggers on one asset; price 150→90 | **693.78 µs** | **69.38 ns/trigger** (10 000 hits) |
| **health_recompute** | 100 000 Kamino `is_liquidatable` + USD/HF helpers | **6.819 ms** | **68.19 ns/iter** |
| **ix_encode** | 100 000× (Kamino liquidate v2 + 2 refresh + P0 classic + Save redeem) | **6.386 ms** | **63.86 ns/iter** (~7.3 MB encoded) |

Raw stdout:

```
=== Grokbotliq microbench (n=10000) ===
candidate_lookup: 10000 triggers, 10000 hits in 693.78µs (69.38 ns/trigger)
health_recompute: 100000 iters in 6.819365ms (68.19 ns/iter)
ix_encode: 100000 iters (7300000 bytes) in 6.386075ms (63.86 ns/iter)
ok
```

## Notes

- These are **in-process** microbenches (no RPC, no gRPC, no disk).
- Candidate lookup cost is dominated by BTreeMap range scan + hit collection; O(hits) on a dense trigger band.
- Re-run after index/health changes and paste updated numbers here.
- Optional: `cargo run -p bench --release -- 50000` for a larger N.

## Related

- Implementation: `bins/bench/src/main.rs`
- Hot paths: `liq-core::CandidateIndex::on_price_update`, `liq-kamino::is_liquidatable`, protocol `encode_*`
