# Architecture — Grokbotliq

Stream-first Solana liquidation bot. Design target: sub-slot reaction from oracle/account updates to Jito/TPU submission.

## Design principles

1. **Stream first, poll second** — Geyser (Yellowstone) account + slot updates are the source of truth; RPC is for bootstrap, simulation, and fallback.
2. **Local health** — maintain a decoded state store; never wait on RPC to decide if an account is liquidatable after a price tick.
3. **Precompute** — keep partially built transactions (account metas, LUTs, refresh ixs) warm for HOT/CRITICAL candidates.
4. **Profit gates before submit** — configurable min USD profit, min ROI, gas/tip budget, inventory constraints.
5. **Circuit breakers** — halt on oracle staleness, error storms, inventory breach, or global pause signals.
6. **FundingStrategy selection** — evaluate Inventory / flash / receivership paths; pick max EV among feasible Accept decisions.

## Pipeline

```
Geyser / Yellowstone gRPC  (or fixtures / MockGeyser)
        |
        v
+------------------+
| Account decoder  |  -- protocol adapters (Kamino / P0 / Save)
+------------------+
        |
        v
+------------------+
| StateStore       |  slot, write_version, pubkey, decoded, source, recv_ts
+------------------+
        |
        +---> CandidateIndex (CRITICAL/HOT/WARM/COLD)
        |         +-- per-asset BTreeMap price-trigger indexes
        |
        v
Oracle price update ----> trigger crossing scan ----> local health recompute
        |
        v
FundingPathEnumerator  (Inventory | SaveFlash | KaminoFlash | P0Receivership)
        |
        +-- swap cost (SwapRouter) + flash fee + tip modeled
        +-- score ≈ net_profit × landing_prob / latency  (ROI via capital gate)
        |
        v
Protocol-exact Tx builder (CU + refresh + liquidate [+ flash wrap] [+ swap])
        |
        v
Execution (DRY_RUN log | Jito bundle / TPU / RPC send) + Telemetry
```

## FundingStrategy

```
                    +------------------+
                    | Liquidatable hit |
                    +--------+---------+
                             |
         +-------------------+-------------------+
         |                   |                   |
         v                   v                   v
   +-----------+      +-------------+     +------------------+
   | Inventory |      | SaveFlash / |     | P0 Receivership  |
   |  wallet   |      | KaminoFlash |     | start→wd→rp→end  |
   +-----+-----+      +------+------+     +--------+---------+
         |                   |                     |
         +---------+---------+---------------------+
                   |
                   v
         pick max expected_value_score
         among ProfitDecision::Accept
```

| Strategy | Capital | Flash fee | Typical latency weight |
|----------|---------|-----------|------------------------|
| Inventory | Face notional | 0 | lowest |
| SaveFlashLoan | ~0 (flash) | reserve fee (bps) | higher |
| KaminoFlashBorrow | ~0 (flash) | reserve fee | higher |
| Project0Receivership | collateral-first | 0 (avoids flash) | highest (more ixs) |

## Candidate bands

| Band | Meaning | Action |
|------|---------|--------|
| CRITICAL | Already unhealthy / HF below liq threshold | Always evaluate + prebuild tx |
| HOT | Within price epsilon of liquidation | Recompute on every related oracle tick |
| WARM | Elevated risk but not near trigger | Recompute on large price moves / periodic |
| COLD | Healthy with wide buffer | Lazy / batch refresh |

## Crates

| Crate | Responsibility |
|-------|----------------|
| `liq-core` | StateStore, CandidateIndex, profitability, FundingStrategy, Instruction |
| `liq-streaming` | Geyser trait, mock, failover, Yellowstone stubs, fixtures, RPC bootstrap |
| `liq-kamino` | Klend health + flash + tx_builder |
| `liq-project0` | Classic + receivership wire builders |
| `liq-save` | Save health + flash atomic plan |
| `liq-execution` | Tx submit + opportunity JSON |
| `liq-routing` | SwapRouter + route cache |
| `liq-risk` | Limits + circuit breaker |
| `liq-telemetry` | Prometheus-compatible metric types |

## Bins

| Bin | Role |
|-----|------|
| `liquidator` | Bootstrap → stream → funding plans → dry-run / submit |
| `shadow` | Observe + log would-be liquidations without submit |
| `replay` | Replay recorded Geyser fixtures through the pipeline |
| `bench` | Microbench index / health / profitability |

## Config

See `config/example.toml` and `config/example.env`. **DRY_RUN=true** by default.

## Remaining gaps (honest)

- Live Yellowstone gRPC client (trait + stub compile without creds)
- Full IDL-accurate zero-copy account decode
- Production Jito auth + VersionedTransaction signing
- Live reserve flash-fee reads; re-verify Save tags 19/20 and Kamino flash discs on mainnet
