# Architecture — Grokbotliq

Stream-first Solana liquidation bot. Design target: sub-slot reaction from oracle/account updates to Jito/TPU submission.

## Design principles

1. **Stream first, poll second** — Geyser (Yellowstone) when live; **RPC bootstrap + poll/fixtures** is the working path today.
2. **Local health** — maintain a decoded state store; never wait on RPC to decide if an account is liquidatable after a price tick.
3. **Precompute** — keep partially built transactions (account metas, LUTs, refresh ixs) warm for HOT/CRITICAL candidates.
4. **Profit gates before submit** — configurable min USD profit, min ROI, gas/tip budget, inventory constraints.
5. **Circuit breakers** — halt on oracle staleness, error storms, inventory breach, or global pause signals.
6. **FundingStrategy selection** — evaluate Inventory / flash / receivership paths; pick max EV among feasible Accept decisions.
7. **Shadow-mode boundary** — real decode/build/simulate; **never broadcast** when `DRY_RUN` or shadow.

## Shadow-mode boundary

```
                    +------------------------------------------+
                    |           SHADOW / DRY_RUN               |
                    |  (signed-but-not-broadcast OR unsigned   |
                    |   + simulateTransaction sigVerify=false) |
                    +--------------------+---------------------+
                                         |
     real path below                     |     never crosses ↓
                                         v
+-------------+   +--------------+   +-----------+   +----------------+
| RPC URL     |-->| HttpJsonRpc  |-->| Decode    |-->| Health / index |
| or FIXTURES |   | Transport    |   | (planning |   | FundingStrategy|
+-------------+   | (reqwest)    |   |  layouts) |   +--------+-------+
                  +------+-------+   +-----------+            |
                         |                                      v
                         |                            +------------------+
                         |                            | Tx builder       |
                         |                            | (flash/inventory |
                         |                            |  + optional swap |
                         |                            |  from quote blob)|
                         |                            +--------+---------+
                         |                                      |
                         |                                      v
                         |                            +------------------+
                         +--------------------------->| simulateTransaction|
                                                      | NO sendTransaction|
                                                      | NO Jito broadcast |
                                                      +------------------+

Live submit (Jito/TPU) stays behind ExecConfig.dry_run=false + credentials;
forced live without wiring returns an error.
```

## Pipeline

```
Geyser / Yellowstone gRPC  (stub)  OR  fixtures / MockGeyser  OR  RPC poll
        |
        v
+------------------+
| Account decoder  |  -- protocol adapters (Kamino planning decode / P0 / Save)
+------------------+
        |
        v
+------------------+
| StateStore       |  slot, write_version, pubkey, decoded, source, recv_ts
+------------------+
        |
        +---> CandidateIndex (CRITICAL/HOT/WARM/COLD)
        |
        v
Oracle price update ----> trigger crossing scan ----> local health recompute
        |
        v
FundingPathEnumerator  (Inventory | SaveFlash | KaminoFlash | P0Receivership)
        |
        v
Protocol-exact Tx builder
  KaminoFlashBorrow → build_flash_tx()
  SaveFlashLoan     → build_flash_atomic_plan()
  Inventory         → non-flash builder
        |
        v
Shadow: simulateTransaction  |  Live: Jito/TPU (unwired)  + Telemetry
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
| `liq-streaming` | Geyser trait, mock, failover, Yellowstone stubs, fixtures, **reqwest RPC** |
| `liq-kamino` | Klend health + flash + tx_builder + planning decode |
| `liq-project0` | Classic + receivership wire builders |
| `liq-save` | Save health + flash atomic plan |
| `liq-execution` | Tx submit + opportunity JSON + **strategy planner** |
| `liq-routing` | SwapRouter + JupiterQuoteBlob + route cache |
| `liq-risk` | Limits + circuit breaker |
| `liq-telemetry` | Prometheus-compatible metric types |

## Bins

| Bin | Role |
|-----|------|
| `liquidator` | Bootstrap → stream → funding plans → dry-run / simulate |
| `shadow` | Observe + build ixs + simulate; never sign/broadcast |
| `replay` | Replay recorded Geyser fixtures through the pipeline |
| `bench` | Microbench index / health / profitability |

## Config

See `config/example.toml` and `config/example.env`. **DRY_RUN=true** by default.

## Remaining gaps (honest)

- Live Yellowstone gRPC client (RPC is the real bootstrap path)
- Full IDL-accurate zero-copy account decode (planning fixture decode for Kamino only)
- Production Jito auth + VersionedTransaction signing
- Live reserve flash-fee reads; Save tag 19/20 mainnet re-verify
