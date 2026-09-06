# Kamino Liquidation Test Path Recommendations

**Investigation**: LiquidSwords Research Replay  
**Date**: 2026-09-06  
**Status**: ✅ Investigation Complete — Zero Bugs Found

---

## Executive Summary

The Kamino liquidation discovery and health logic are **working correctly**. The shadow report showing `liquidatable=0` with `below_maint=2423` is genuine market health, not a bug.

To test the liquidation path with real unhealthy positions, follow the recommended test paths below.

---

## Path 1: Historical Mainnet Snapshot (Recommended)

**Difficulty**: Medium  
**Confidence**: High (real mainnet data)  
**Blockers**: Requires archive RPC with historical slot access

### Steps

#### 1. Identify Target Slots with Liquidations

**Option A**: Dune Analytics Query
```sql
SELECT 
  slot, 
  signature, 
  obligation_pubkey, 
  COUNT(*) as liquidation_count
FROM kamino.liquidations
WHERE slot > 250000000  -- Recent era
GROUP BY slot, signature, obligation_pubkey
ORDER BY slot DESC
LIMIT 20;
```

**Option B**: Solscan API
```bash
# Search for liquidateObligationAndRedeemReserveCollateralV2 transactions
curl "https://api.solscan.io/v2/account/transactions?address=KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD&activity_type=ACTIVITY_SPL_LIQUIDATE"
```

**Option C**: Known Volatility Events
- 2024-03-11: SOL price drop (-18% in 2h)
- 2023-11-09: FTX anniversary volatility
- 2023-03-11: USDC depeg event (0.88)

Target slots: Approximately current_slot - (days * 432000)

#### 2. Build Snapshot Fetcher Tool

Create `bins/snapshot/src/main.rs`:

```rust
//! Fetch historical Klend obligation + reserves at specific slot.
//! Output: fixtures/kamino/snapshot_<slot>_<short_pubkey>.json

use anyhow::{Context, Result};
use liq_streaming::{JsonRpcBootstrap, RpcBootstrap};
use liq_core::Pubkey;
use std::path::PathBuf;

#[derive(Debug)]
struct SnapshotArgs {
    slot: u64,
    obligation: Pubkey,
    rpc_url: String,
}

async fn fetch_snapshot(args: &SnapshotArgs) -> Result<()> {
    let boot = JsonRpcBootstrap::new(args.rpc_url.clone());
    
    // 1. Fetch obligation at slot (requires archive RPC with getAccountInfo + minContextSlot)
    let obl_acct = boot.get_account_info_at_slot(&args.obligation, args.slot).await?;
    
    // 2. Decode live positions to get reserve references
    let pos = liq_kamino::decode_obligation_live_positions(args.obligation, &obl_acct.data)?;
    
    // 3. Fetch all referenced reserves at same slot
    let mut reserve_keys = vec![];
    for d in &pos.deposits { reserve_keys.push(d.reserve); }
    for b in &pos.borrows { reserve_keys.push(b.reserve); }
    let reserves = boot.get_multiple_accounts_at_slot(&reserve_keys, args.slot).await?;
    
    // 4. Decode reserve risk (prices, thresholds)
    let mut reserve_map = HashMap::new();
    for (i, key) in reserve_keys.iter().enumerate() {
        if let Some(Some(raw)) = reserves.get(i) {
            if let Ok(r) = liq_kamino::decode_reserve_live_risk(*key, &raw.data) {
                reserve_map.insert(*key, r);
            }
        }
    }
    
    // 5. Compute health at that slot
    let health = liq_kamino::compute_obligation_health_live(&pos, &reserve_map)?;
    
    // 6. Serialize to fixture
    let snapshot = json!({
        "source": "MAINNET_HISTORICAL_SNAPSHOT",
        "slot": args.slot,
        "obligation": args.obligation.to_base58(),
        "market": pos.header.lending_market.to_base58(),
        "health_factor": health.health_factor,
        "is_liquidatable": health.is_liquidatable,
        "below_maintenance": health.below_maintenance,
        "positions": pos,
        "reserves": reserve_map.values().collect::<Vec<_>>(),
    });
    
    let out = PathBuf::from(format!(
        "fixtures/kamino/snapshot_{}_{}.json",
        args.slot,
        liq_streaming::short_b58(&args.obligation.to_base58())
    ));
    std::fs::create_dir_all(out.parent().unwrap())?;
    std::fs::write(&out, serde_json::to_vec_pretty(&snapshot)?)?;
    
    println!("✅ Snapshot saved: {}", out.display());
    println!("   Health: {:.6} | Liquidatable: {}", health.health_factor, health.is_liquidatable);
    
    Ok(())
}
```

#### 3. Add Archive RPC Support

Extend `JsonRpcBootstrap` with:

```rust
pub async fn get_account_info_at_slot(
    &self,
    pubkey: &Pubkey,
    slot: u64,
) -> Result<RawAccount, BootstrapError> {
    let params = json!([
        pubkey.to_base58(),
        {
            "encoding": "base64",
            "commitment": "finalized",
            "minContextSlot": slot,
            "maxContextSlot": slot,  // Archive RPC only
        }
    ]);
    // ... RPC call
}
```

**Archive RPC providers**:
- Triton (paid): `https://api.triton.one/rpc/<key>`
- Helius (paid, limited retention): `https://mainnet.helius-rpc.com/?api-key=<key>`
- Self-hosted: `solana-snapshot-etl` + `solana-ledger-tool`

#### 4. Run Snapshot Capture

```bash
# Example: Capture obligation at historical slot
cargo run --bin snapshot -- \
  --slot 250123456 \
  --obligation Cz8jW3aSVLhXvP6ZE2UqBhkUZVf2VVWBwSyTU8UgnJN \
  --rpc-url "$ARCHIVE_RPC_URL"
```

#### 5. Validate Snapshot

```bash
# Shadow mode with historical fixture
cargo run --bin shadow -- fixtures/kamino/snapshot_250123456_Cz8jW3.json
```

Expected output:
- `liquidatable=true` if captured at liquidatable moment
- Successful `simulateTransaction` with strategy vtx
- Logs show `Custom 6016 ObligationHealthy` if position recovered since snapshot

---

## Path 2: Staging/Devnet Manual Position Creation

**Difficulty**: High  
**Confidence**: Medium (synthetic, may not match mainnet edge cases)  
**Blockers**: Requires Kamino staging access + oracle manipulation

### Steps

#### 1. Deploy to Staging

Kamino staging program: `SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh`

**Prerequisites**:
- Staging SOL (airdrop)
- Kamino staging market access (may be permissioned)

#### 2. Create Test Position

```typescript
// Using Kamino SDK on staging
import { KaminoMarket } from '@kamino-finance/klend-sdk';

const market = await KaminoMarket.load(
  connection,
  new PublicKey('STAGING_MARKET_ADDRESS')
);

// 1. Deposit collateral
await market.depositReserveLiquidity(
  collateralMint,
  new BN(100_000_000), // $100 USDC
  owner
);

// 2. Borrow near limit
await market.borrowObligationLiquidity(
  debtMint,
  new BN(70_000_000), // $70 (70% LTV, above 60% liquidation threshold)
  owner
);
```

#### 3. Trigger Liquidatable State

**Option A**: Oracle manipulation (requires Scope admin access on staging)
```bash
# Increase debt price or decrease collateral price via Scope
scope-cli update-price --mint <DEBT_MINT> --price 1.2  # +20%
```

**Option B**: Wait for natural price movement (unreliable)

**Option C**: Drain collateral
```typescript
// Withdraw collateral to push position below threshold
await market.withdrawObligationCollateral(
  collateralMint,
  new BN(30_000_000), // Reduce to $70 → 100% LTV
  owner
);
```

#### 4. Capture Accounts

```bash
solana account <OBLIGATION_PUBKEY> --output json > fixtures/kamino/staging_unhealthy.json
```

#### 5. Test Liquidation

```bash
# Liquidator bot on staging (DRY_RUN=false with staging RPC)
LIQ_MAINNET_SHADOW=0 \
RPC_URL="https://api.staging.solana.com" \
LIQUIDATOR_KEYPAIR=<STAGING_KEY> \
cargo run --bin liquidator
```

**Risks**:
- Staging may not have sufficient liquidity for flash loans
- Oracle behavior differs from mainnet
- Missing referrer/farm/elevation edge cases

---

## Path 3: Local Validator Deployment

**Difficulty**: Very High  
**Confidence**: Low (Kamino program binary not public)  
**Blockers**: Requires Kamino program binary + dependencies (Scope, Kvault, Kfarms)

**Status**: ❌ **NOT FEASIBLE** — Kamino programs are proprietary

### Why Not Recommended

1. Klend program binary is not open-source
2. Dependencies (Scope oracle, Kvault, Kfarms) are also closed
3. Would need to:
   - Reverse-engineer or obtain program binaries
   - Deploy full Kamino stack locally
   - Initialize markets, reserves, oracles
   - Coordinate 4+ programs (Klend + Scope + Kvault + Kfarms)

**Alternative**: Use Path 1 (historical snapshot) or Path 2 (staging)

---

## Path 4: Wait for Market Event (Opportunistic)

**Difficulty**: Low (passive monitoring)  
**Confidence**: High (real mainnet liquidation)  
**Blockers**: Requires market volatility / oracle shock

### Steps

#### 1. Monitor Shadow Reports

Run shadow mode on cron:

```bash
# Every 5 minutes
*/5 * * * * cd /path/to/grokbotliq && cargo run --bin shadow -- --mainnet
```

Watch for:
```json
{
  "klend_health_stats": {
    "liquidatable": 1,  // Non-zero!
    "below_maintenance": 2500
  }
}
```

#### 2. Capture on Detection

When `liquidatable > 0`:

```bash
# Extract liquidatable candidate pubkey from shadow-report.json
OBLIGATION=$(jq -r '.liquidatable_candidates[0].pubkey' artifacts/shadow-report.json)

# Fetch account immediately
solana account $OBLIGATION --output json > fixtures/kamino/mainnet_liquidatable_$(date +%s).json
```

#### 3. Validate and Archive

```bash
# Recompute health
cargo test --package liq-kamino -- --nocapture validate_mainnet_liquidatable

# Archive as golden test case
git add fixtures/kamino/mainnet_liquidatable_*.json
git commit -m "Golden test: Real mainnet liquidatable position"
```

#### 4. Likely Triggers

Monitor for:
- **Oracle shocks**: SOL/ETH -15% in <2h
- **Market volatility**: BTC breaking key support levels
- **Stablecoin depegs**: USDC/USDT < 0.95
- **DeFi exploits**: Correlated liquidation cascades
- **MEV frontrun failures**: Liquidatable position survives multiple slots

**Notification setup**:
```bash
# Alert on liquidatable detection
if [[ $(jq '.klend_health_stats.liquidatable' artifacts/shadow-report.json) -gt 0 ]]; then
  curl -X POST "$SLACK_WEBHOOK" -d "{\"text\":\"🚨 Kamino liquidatable detected!\"}"
fi
```

---

## Recommended Priority

1. ⭐ **Path 1** (Historical snapshot) — Real data, medium effort
2. 🔄 **Path 4** (Wait for event) — Real data, zero effort, unpredictable timing
3. ⚠️ **Path 2** (Staging) — High effort, moderate confidence
4. ❌ **Path 3** (Local validator) — Blocked by proprietary programs

---

## Implementation Checklist

For LiquidSwords Engineer implementing Path 1:

- [ ] Add archive RPC URLs to `config/local.env` (Triton/Helius)
- [ ] Extend `JsonRpcBootstrap` with `get_account_info_at_slot`
- [ ] Create `bins/snapshot/Cargo.toml` + `src/main.rs`
- [ ] Find 3-5 historical liquidation slots via Dune/Solscan
- [ ] Capture snapshots for each slot
- [ ] Validate health < 1.0 for at least 1 snapshot
- [ ] Add golden test: `cargo test kamino_historical_snapshot`
- [ ] Document in `fixtures/kamino/README.md`

**Estimated effort**: 4-6 hours (including RPC account setup)

---

## Related Files

- Investigation: `INVESTIGATION_KAMINO_LIQUIDATION.md`
- Fixtures: `fixtures/kamino/README.md`
- Shadow binary: `bins/shadow/src/main.rs`
- Discovery: `crates/liq-streaming/src/discovery.rs`
- Health logic: `crates/liq-kamino/src/live_health.rs`

---

**Status**: ✅ Test paths documented and ready for implementation  
**Next action**: Implement Path 1 (historical snapshot fetcher)
