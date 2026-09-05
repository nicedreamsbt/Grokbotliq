# Grokbotliq

Production-oriented Solana multi-protocol liquidation bot (Kamino / Project 0 / Save).

**Status:** Phases 1–8 foundations. Core engine, adapters, execution path, fixtures, shadow/replay/bench. **`DRY_RUN=true` by default.** No secrets in repo.

## Docs

| Doc | Contents |
|-----|----------|
| [PROTOCOL_RESEARCH.md](./PROTOCOL_RESEARCH.md) | Program IDs, math, **IDL pin paths/versions** |
| [ARCHITECTURE.md](./ARCHITECTURE.md) | Stream-first pipeline |
| [BENCHMARKS.md](./BENCHMARKS.md) | Local microbench numbers |
| [fixtures/README.md](./fixtures/README.md) | Oracle + borrower JSON fixtures |

## Workspace

```
crates/
  liq-core          state store, candidate bands, price-trigger index, profitability
  liq-streaming     Geyser trait, mock, freshness failover, Yellowstone stubs, fixtures
  liq-kamino        klend health + liquidate v2 helpers (+ idls/)
  liq-project0      classic + receivership (+ idls/)
  liq-save          Save/Solend health + LiquidateObligation (+ idls/)
  liq-execution     dry-run / submit placeholders (RPC + Jito)
  liq-routing       swap quote trait + stub router
  liq-risk          limits + circuit breaker
  liq-telemetry     Prometheus-compatible metrics types
bins/
  liquidator  shadow  replay  bench
fixtures/             sample oracle ticks + borrower snapshots
config/               example.toml + example.env
docker/  deploy/      container + systemd unit
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
cargo run -p liquidator             # smoke + idle (dry-run)
cargo run -p shadow -- fixtures     # fixture stream, no signing
cargo run -p replay -- fixtures     # oracle→candidate→plan JSON lines
cargo run -p bench                  # microbench → update BENCHMARKS.md
```

### Config

| File | Role |
|------|------|
| `config/example.toml` | Safe defaults (`dry_run`, RPC placeholder, risk, protocols) |
| `config/example.toml` → `LIQ_CONFIG` | Override path (default `config/example.toml`) |
| `config/example.env` | Env template for Docker/systemd |

### Binaries

| Command | Behavior |
|---------|----------|
| `cargo run -p liquidator` | Load config; dry-run smoke tx; waits for Geyser only if configured |
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
| `LIQ_FIXTURES` | Override fixtures directory |
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
