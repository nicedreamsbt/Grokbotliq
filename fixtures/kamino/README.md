# Kamino Test Fixtures

⚠️ **WARNING: LOCAL SYNTHETIC FIXTURES ONLY** ⚠️

This directory contains **constructed test cases** for Kamino liquidation scenarios. These are NOT mainnet evidence.

## Files

### `unhealthy_obligation_LOCAL_SYNTHETIC.json`

**Status**: 🔴 **SYNTHETIC / MODIFIED / NOT MAINNET**

**Purpose**: Demonstrates what a liquidatable Kamino position would look like, based on unit test fixtures from `crates/liq-kamino/src/live_health.rs:561`.

**Key parameters**:
- $100 collateral @ 60% liquidation threshold → $60 max safe debt
- $70 borrowed → **LIQUIDATABLE** (health factor 0.857)
- Also `below_maintenance` (exceeds 50% LTV)

**Use case**: Test liquidation instruction builders and bonus calculations without requiring mainnet liquidatable positions.

## Mainnet Reality Check

**As of shadow report baseline (commit ec2afb1)**:
- ✅ 106,036 obligations scanned
- ✅ 2,423 below_maintenance (warning zone: LTV < debt < liquidation threshold)
- ✅ **0 liquidatable** (healthy market!)

**Interpretation**: Kamino mainnet is currently healthy. This fixture shows a scenario that does NOT exist on-chain.

## Finding Real Liquidatable Positions

Since mainnet has 0 liquidatable positions currently, to test with real data you need:

### Option 1: Historical Snapshot (Recommended)

1. **Identify target slots** with historical liquidations:
   - Dune Analytics: `kamino.liquidations WHERE slot > X`
   - Solscan API: Search for `liquidateObligationAndRedeemReserveCollateralV2` txs
   - Target events: Oracle shocks (SOL -20%), market volatility, USDC depeg

2. **Fetch accounts at historical slot**:
   ```bash
   # Use archive RPC with minContextSlot
   bins/snapshot --slot <HISTORICAL_SLOT> --obligation <PUBKEY>
   ```

3. **Validate health < 1.0**:
   - Use reserve.marketPriceSf from that slot
   - Recompute health factor
   - Confirm liquidatable=true

### Option 2: Staging/Devnet Testing

1. Use Kamino staging program: `SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh`
2. Create test position with low LTV
3. Manipulate oracle or drain collateral
4. Test real liquidation flow

### Option 3: Wait for Market Event

- Monitor `shadow --mainnet` output for `liquidatable > 0`
- Capture obligation pubkeys when market conditions deteriorate
- Store as fixtures for future testing

## Adding New Fixtures

When adding fixtures, **ALWAYS**:

1. ✅ Label clearly as `LOCAL_SYNTHETIC` or `MAINNET_HISTORICAL` in filename
2. ✅ Include `warning` field in JSON explaining source
3. ✅ Document slot number for historical data
4. ✅ Never claim modified state is current mainnet evidence

## Related Files

- Unit tests: `crates/liq-kamino/src/live_health.rs` (tests section)
- Shadow binary: `bins/shadow/src/main.rs`
- Discovery: `crates/liq-streaming/src/discovery.rs`
- Investigation report: `INVESTIGATION_KAMINO_LIQUIDATION.md`

---

**Generated**: 2026-09-06 (Cloud Agent investigation)  
**Baseline**: commit ec2afb1 (Fix Klend candidate health)
