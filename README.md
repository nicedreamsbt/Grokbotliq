# Grokbotliq

Solana multi-protocol liquidation bot (Kamino / Project 0 / Save).

**Status (honest):** Flash/atomic ix builders are real and planner-wired. **Multi-RPC rotation**, mainnet discovery (known markets + filtered GPA), and **`shadow --mainnet`** write a redacted `artifacts/shadow-report.json` after live `getSlot` / simulate. **Not production-ready:** live Yellowstone gRPC client, full Anchor zero-copy decode, signing, and Jito auth remain unwired. **`DRY_RUN=true` by default — never broadcasts in dry/shadow.**

## What's wired vs not

| Area | Wired | Not yet |
|------|-------|---------|
| FundingStrategy enum + EV path pick | Inventory / SaveFlash / KaminoFlash / P0Receivership | Live inventory balances / reserve fee reads |
| Save flash atomic plan | FlashBorrow(19) → liq → FlashRepay(20); RO/W metas vs solend-sdk | Mainnet tag 19/20 re-verify |
| Kamino flash | IDL-backed borrow/repay; **referrer absent → KLend program id RO**; discs user-verified | Live reserve fee / farm remaining accounts |
| Planner | **`KaminoFlashBorrow` → `build_flash_tx()`**; SaveFlash → flash builder; Inventory non-flash | Config-loaded mainnet pubkeys (fixtures/seed for now) |
| P0 receivership | Full wire `Instruction` sequence | Live bank/oracle remaining accounts |
| State ingestion | **reqwest** `HttpJsonRpcTransport` + fixture bootstrap; planning decode for Kamino | Full IDL zero-copy + Yellowstone stream decode |
| Shadow / simulate | Envelope + `simulateTransaction` (`sigVerify:false`); **no broadcast** | `solana-sdk` VersionedTransaction sign |
| Routing | Jupiter **quote JSON/ix blob** attach (omit if missing); DirectDex trait | Live Jupiter quote HTTP without blob |
| Jito / submit | Traits + dry-run engine; live submit errors if forced | Block-engine auth |

## Docs

| Doc | Contents |
|-----|----------|
| [PROTOCOL_RESEARCH.md](./PROTOCOL_RESEARCH.md) | Program IDs, math, IDL pins, flash layouts, meta notes |
| [ARCHITECTURE.md](./ARCHITECTURE.md) | Pipeline + FundingStrategy + **shadow-mode boundary** |
| [BENCHMARKS.md](./BENCHMARKS.md) | Local microbench numbers |
| [fixtures/README.md](./fixtures/README.md) | Oracle + borrower JSON fixtures |

## Workspace

```
crates/
  liq-core          state store, candidate bands, profitability, FundingStrategy, Instruction
  liq-streaming     Geyser trait, mock, failover, Yellowstone stub, fixtures, **reqwest RPC**
  liq-kamino        klend health + flash + tx_builder + planning decode
  liq-project0      classic + receivership wire builders
  liq-save          Save/Solend health + flash atomic plan
  liq-execution     dry-run / submit + funding opportunity + **strategy planner**
  liq-routing       SwapRouter + JupiterQuoteBlob + DirectDex + RouteCache
  liq-risk          limits + circuit breaker
  liq-telemetry     Prometheus-compatible metrics types
bins/
  liquidator  shadow  replay  bench
```

## Safety defaults

- **`dry_run = true`** in `config/example.toml`; **`DRY_RUN=true`** in `config/example.env`
- `shadow` **refuses** to start if `DRY_RUN=false`
- Execution engine skips broadcast when dry-run; simulate path never broadcasts
- Jito submit stays unwired / errors if forced live
- `.gitignore` excludes `.env`, `config/local.env`, keypairs, `config/local.toml`, `target/`
- Logs / reports print **RPC host only** (never API keys or full query-string URLs)

## Quick start

```bash
cd /path/to/Grokbotliq
cp config/example.env .env          # keep DRY_RUN=true
# Real keys go in gitignored config/local.env (never commit):
#   RPC_URLS=https://YOUR_RPC_HOST/?api-key=...,https://YOUR_RPC_HOST_2/?api-key=...
#   RPC_URL=https://YOUR_RPC_HOST/?api-key=...
#   DRY_RUN=true
cp config/example.toml config/local.toml   # optional; local.toml is gitignored

cargo test --workspace --lib
cargo run -p liquidator             # live RPC discovery if local.env present; else fixtures
LIQ_FIXTURES=fixtures cargo run -p liquidator   # force CI fixture path
cargo run -p shadow -- fixtures     # fixture stream + simulate envelope, no signing
cargo run -p shadow -- --mainnet    # or LIQ_MAINNET_SHADOW=1 — discover + simulateTransaction
cargo run -p replay -- fixtures
cargo run -p bench
```

### Config

| File / env | Role |
|------------|------|
| `config/example.toml` | Safe defaults (`dry_run`, RPC placeholder) |
| `config/local.env` | **Gitignored** runtime secrets: `RPC_URL` / `RPC_URLS` (read at startup) |
| `LIQ_CONFIG` | Override config path |
| `LIQ_FIXTURES` | Offline CI fixtures path (**forces** fixture bootstrap) |
| `RPC_URL` | Single JSON-RPC endpoint (fallback if `RPC_URLS` unset) |
| `RPC_URLS` | Comma-separated rotation / failover pool (preferred) |
| `LIQ_MAINNET_SHADOW` | `1` enables mainnet shadow path (same as `shadow --mainnet`) |
| `JUPITER_SWAP_IX_JSON` | Path to Jupiter swap-instructions JSON; omit swap if unset |
| `LIQ_LOOP_TICKS` | Cap liquidator loop iterations |

### Multi-RPC rotation

`liq-streaming::RotatingRpcPool` parses `RPC_URLS` (else `RPC_URL`), sticky-selects a working endpoint, and on **429 / 5xx / timeout** advances to the next URL with short backoff. Per-endpoint latency / error counters land in `artifacts/shadow-report.json` as **host-only** names (path and query stripped).

### Shadow mainnet

```bash
# loads config/local.env, rotates RPC, discovers known markets + filtered GPA, builds plans, simulateTransaction(sigVerify=false)
cargo run -p shadow -- --mainnet
# writes artifacts/shadow-report.json (redacted; no API keys)
```

`DRY_RUN` stays true — never `sendTransaction` / Jito.

### Placeholders for live mode

| Var | Purpose |
|-----|---------|
| `RPC_URL` / `RPC_URLS` | Private Solana RPC(s) — put real values only in `config/local.env` |
| `GEYSER_ENDPOINT` / `GEYSER_X_TOKEN` | Yellowstone gRPC (stub until client linked) |
| `JITO_BLOCK_ENGINE_URL` | Bundle submission (errors if forced live) |
| `KEYPAIR_PATH` | Funded liquidator keypair (**never commit**) |
| `DRY_RUN` | Must stay `true` until creds + IDL path validated |

## Tests

```bash
cargo test --workspace --lib
```

## Remaining gaps (honest)

1. Yellowstone live gRPC (HTTP RPC bootstrap + rotation is the real path today)
2. Full Anchor zero-copy for marginfi balances / Save obligations (Kamino live SF header decode exists; deposit/borrow arrays incomplete)
3. VersionedTransaction signing + LUT fill (shadow simulates a JSON envelope, not a signed vtx)
4. Live Jupiter quote HTTP (blob attach works when JSON provided)
5. Jito auth / broadcast path
6. Save obligation GPA `dataSize` / market memcmp offset may need mainnet retune if counts are zero
