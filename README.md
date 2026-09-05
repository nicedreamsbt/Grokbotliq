# Grokbotliq

Production-oriented Solana multi-protocol liquidation bot (Kamino / Project 0 / Save).

**Status:** Phase 1 research + Phase 2 foundations. `DRY_RUN=true` by default. No secrets in repo.

## Docs

- [PROTOCOL_RESEARCH.md](./PROTOCOL_RESEARCH.md) — program IDs, liquidation math, ix ordering, TODOs
- [ARCHITECTURE.md](./ARCHITECTURE.md) — stream-first pipeline design

## Workspace

```
crates/
  liq-core          state store, candidate bands, price-trigger index, profitability
  liq-streaming     Geyser subscriber trait + mock provider
  liq-kamino        klend health + liquidate v2 helpers
  liq-project0      classic + receivership health / profit caps
  liq-save          Save/Solend health + LiquidateObligation encoding
  liq-execution     dry-run / submit placeholders (RPC + Jito)
  liq-routing       swap quote trait + stub router
  liq-risk          limits + circuit breaker
  liq-telemetry     Prometheus-compatible metrics types
bins/
  liquidator  shadow  replay  bench
```

## Quick start

```bash
cd /path/to/Grokbotliq
cp config/example.env .env   # keep DRY_RUN=true
cargo test
cargo run -p liquidator
```

### Placeholders for live mode

| Var | Purpose |
|-----|---------|
| `RPC_URL` | Private Solana RPC |
| `GEYSER_ENDPOINT` / token | Yellowstone gRPC |
| `JITO_BLOCK_ENGINE_URL` | Bundle submission |
| `KEYPAIR_PATH` | Funded liquidator keypair (never commit) |

Set `DRY_RUN=false` only after credentials + IDL discriminators are wired.

## Docker / systemd

See `docker/` and `deploy/liquidator.service`.

## Safety

- Default dry-run; circuit breaker; profitability gates
- `.gitignore` excludes `target/`, `.env`, keypairs
