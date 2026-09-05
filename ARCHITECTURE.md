# Architecture — Grokbotliq

Stream-first Solana liquidation bot. Design target: sub-slot reaction from oracle/account updates to Jito/TPU submission.

## Design principles

1. **Stream first, poll second** — Geyser (Yellowstone) account + slot updates are the source of truth; RPC is for bootstrap, simulation, and fallback.
2. **Local health** — maintain a decoded state store; never wait on RPC to decide if an account is liquidatable after a price tick.
3. **Precompute** — keep partially built transactions (account metas, LUTs, refresh ixs) warm for HOT/CRITICAL candidates.
4. **Profit gates before submit** — configurable min USD profit, min ROI, gas/tip budget, inventory constraints.
5. **Circuit breakers** — halt on oracle staleness, error storms, inventory breach, or global pause signals.

## Pipeline

```
Geyser / Yellowstone gRPC
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
ProfitabilityCalculator + RiskLimits
        |
        v
Precomputed Tx builder (refresh + liquidate + optional swap)
        |
        v
Execution (Jito bundle / TPU / RPC send) + Telemetry
```

## Candidate bands

| Band | Meaning | Action |
|------|---------|--------|
| CRITICAL | Already unhealthy / HF below liq threshold | Always evaluate + prebuild tx |
| HOT | Within price epsilon of liquidation | Recompute on every related oracle tick |
| WARM | Elevated risk but not near trigger | Recompute on large price moves / periodic |
| COLD | Healthy with wide buffer | Lazy / batch refresh |

Price-trigger indexes: for each asset mint, a `BTreeMap<OrderedPrice, Vec<AccountId>>` stores the price level at which the account would cross into liquidatable territory (holding other prices fixed). On oracle update for mint M at price P, scan all triggers with key <= P (or >= P depending on exposure side).

## Crates

| Crate | Responsibility |
|-------|----------------|
| `liq-core` | StateStore, CandidateIndex, profitability, types, health traits |
| `liq-streaming` | Geyser subscriber trait + mock provider |
| `liq-kamino` | Klend health math + ix layout helpers |
| `liq-project0` | Classic + receivership builders / health |
| `liq-save` | Save/Solend health + liquidate ix encoding |
| `liq-execution` | Tx submit (dry-run, RPC, Jito placeholder) |
| `liq-routing` | Swap route quotes (stub + trait) |
| `liq-risk` | Limits + circuit breaker |
| `liq-telemetry` | Prometheus-compatible metric types |

## Bins

| Bin | Role |
|-----|------|
| `liquidator` | Live / dry-run bot |
| `shadow` | Observe + log would-be liquidations without submit |
| `replay` | Replay recorded Geyser fixtures through the pipeline |
| `bench` | Microbench index / health / profitability |

## Config

See `config/example.toml` and `config/example.env`. **DRY_RUN=true** by default.

## Non-goals (this phase)

- Live Geyser connection (trait + mock only)
- Full IDL-accurate zero-copy account decode
- Production Jito auth
