# Grokbotliq

Solana multi-protocol liquidation bot (Kamino / Project 0 / Save).

**Status:** Beyond pure scaffold — funding path selection, protocol-exact instruction builders, flash/atomic compositions, and an ingestion→plan loop are wired. **Still not production-ready:** live Yellowstone gRPC client, live RPC decode, signing, and Jito auth are stubs. **`DRY_RUN=true` by default.** No secrets in repo.

## What's wired vs not

| Area | Wired | Not yet |
|------|-------|---------|
| FundingStrategy enum + EV path pick | Inventory / SaveFlash / KaminoFlash / P0Receivership | Live inventory balances / reserve fee reads |
| Save flash atomic plan | FlashBorrow(19) → liq → FlashRepay(20) + tests | Mainnet fee/tag re-verify (TODO in PROTOCOL_RESEARCH) |
| Kamino flash | IDL-backed borrow/repay + Anchor discriminators | SDK codegen disc re-pin before mainnet |
| P0 receivership | Full wire `Instruction` sequence (start→withdraw→repay→end) | Live bank/oracle remaining accounts |
| Tx builders | Non-empty data bytes + explicit account metas | VersionedTransaction sign + LUT fill |
| State ingestion | Fixture bootstrap + StateStore apply; JSON-RPC request shapes | reqwest/Yellowstone live clients |
| liquidator binary | Loop: bootstrap → fixture stream → funding plans → DRY_RUN JSON | Continuous live Geyser without fixtures |
| Routing | SwapRouter + Jupiter placeholder + DirectDex + HOT route cache | Real Jupiter/DEX quote HTTP |
| Jito / submit | Traits + dry-run engine | Live block-engine auth |

## Docs

| Doc | Contents |
|-----|----------|
| [PROTOCOL_RESEARCH.md](./PROTOCOL_RESEARCH.md) | Program IDs, math, IDL pins, **flash loan layouts** |
| [ARCHITECTURE.md](./ARCHITECTURE.md) | Pipeline + **FundingStrategy** diagram |
| [BENCHMARKS.md](./BENCHMARKS.md) | Local microbench numbers |
| [fixtures/README.md](./fixtures/README.md) | Oracle + borrower JSON fixtures |

## Workspace

```
crates/
  liq-core          state store, candidate bands, profitability, FundingStrategy, Instruction
  liq-streaming     Geyser trait, mock, failover, Yellowstone stub, fixtures, RPC bootstrap
  liq-kamino        klend health + liquidate v2 + flash + tx_builder
  liq-project0      classic + receivership wire builders
  liq-save          Save/Solend health + flash atomic plan
  liq-execution     dry-run / submit + funding opportunity JSON
  liq-routing       SwapRouter (stub / Jupiter placeholder / DirectDex) + RouteCache
  liq-risk          limits + circuit breaker
  liq-telemetry     Prometheus-compatible metrics types
bins/
  liquidator  shadow  replay  bench
```

## Safety defaults

- **`dry_run = true`** in `config/example.toml`; **`DRY_RUN=true`** in `config/example.env`
- `shadow` **refuses** to start if `DRY_RUN=false`
- Execution engine skips broadcast when dry-run; live submit errors until RPC/Jito creds wired
- Circuit breaker + profitability gates before any submit path
- `.gitignore` excludes `.env`, keypairs, `config/local.toml`, `target/`

## Quick start

```bash
cd /path/to/Grokbotliq
cp config/example.env .env          # keep DRY_RUN=true
cp config/example.toml config/local.toml   # optional; local.toml is gitignored

cargo test --workspace --lib
cargo run -p liquidator             # fixture bootstrap → plan loop (dry-run)
cargo run -p shadow -- fixtures     # fixture stream, no signing
cargo run -p replay -- fixtures     # oracle→candidate→plan JSON lines
cargo run -p bench                  # microbench → update BENCHMARKS.md
```

### Config

| File | Role |
|------|------|
| `config/example.toml` | Safe defaults (`dry_run`, RPC placeholder, risk, protocols) |
| `LIQ_CONFIG` | Override config path (default `config/example.toml`) |
| `LIQ_FIXTURES` | Fixtures directory |
| `LIQ_LOOP_TICKS` | Cap liquidator loop iterations (default: all fixture ticks in dry-run) |

### Binaries

| Command | Behavior |
|---------|----------|
| `cargo run -p liquidator` | Bootstrap (fixtures if no live RPC) → stream → FundingStrategy plans → DRY_RUN opportunity JSON |
| `cargo run -p shadow -- fixtures` | Load fixtures; print shadow opportunities; **assert DRY_RUN**; no signatures |
| `cargo run -p replay -- fixtures` | Same path + structured opportunity JSON + dry-run execute sample |
| `cargo run -p bench [N]` | Candidate lookup / health recompute / ix encode timings |

### Placeholders for live mode

| Var | Purpose |
|-----|---------|
| `RPC_URL` | Private Solana RPC |
| `GEYSER_ENDPOINT` / `GEYSER_X_TOKEN` | Yellowstone gRPC |
| `JITO_BLOCK_ENGINE_URL` | Bundle submission |
| `KEYPAIR_PATH` | Funded liquidator keypair (**never commit**) |
| `DRY_RUN` | Must stay `true` until creds + IDL path validated |

Set `DRY_RUN=false` only after credentials, keypair, and live market pubkeys are wired.

## Docker / systemd

```bash
docker compose -f docker/docker-compose.yml build
docker compose -f docker/docker-compose.yml up    # DRY_RUN forced true
```

Systemd unit: `deploy/liquidator.service` — expects `/opt/grokbotliq`, `config/local.env`, release `liquidator` binary. Mount key material via secrets (commented in unit); do not bake keys into the image.

## Tests

```bash
cargo test --workspace --lib
```
