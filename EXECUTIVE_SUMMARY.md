# Executive Summary: Kamino Liquidation Investigation

**For**: Michael (LiquidSwords Lead)  
**From**: Cloud Agent (Research Replay)  
**Date**: 2026-09-06  
**PR**: https://github.com/nicedreamsbt/Grokbotliq/pull/1

---

## TL;DR

✅ **Shadow report is CORRECT**  
✅ **Zero bugs found in discovery/decode/health logic**  
✅ **liquidatable=0 / below_maint=2423 = healthy market**

---

## What Was Investigated

Task: Validate whether empty liquidatable set (0) vs 2423 below_maint positions indicates:
- A) Genuine healthy market ✅ **THIS**
- B) Incomplete coverage ❌
- C) Decode/filter bug ❌

## Answer: A) Genuine Healthy Market

### The Two Thresholds

Every Kamino reserve has two risk thresholds:

1. **LTV (loan_to_value_pct)**: Max allowed for *new* borrows (e.g. 65%)
   - Positions above this → **`below_maintenance`**
   - Warning zone: you can't borrow more

2. **Liquidation threshold**: Actual liquidation point (e.g. 75%)
   - Positions above this → **`liquidatable`**
   - Action zone: can be liquidated

**Gap between thresholds = safety buffer** (typically 10-15%)

### Your Numbers

- **106,036** obligations scanned
- **2,423** (2.3%) above LTV but below liquidation threshold → **below_maintenance**
- **0** above liquidation threshold → **liquidatable**

**This is healthy!** 

Normal distribution: Most positions well-collateralized, small tail in warning zone, zero crossed liquidation line.

### Where This Lives in Code

**Health calculation** (`crates/liq-kamino/src/live_health.rs:360-365`):
```rust
let is_liquidatable = meaningful && bf_debt > unhealthy;     // line 364
let below_maintenance = meaningful && bf_debt > allowed;      // line 365
```

Where:
- `unhealthy` = Σ(deposit × liquidation_threshold_pct / 100)
- `allowed` = Σ(deposit × loan_to_value_pct / 100)

**Discovery** (`crates/liq-streaming/src/discovery.rs:404-429`):
```rust
let is_liq = d.notes.iter().any(|n| n == "liquidatable=true");
let is_bm = d.notes.iter().any(|n| n == "below_maintenance=true");

if is_liq {
    liquidatable += 1;
    liquidatable_candidates.push(d.clone());
} else if is_bm {
    below_maint += 1;
    // Above max LTV but not liquidatable — do not simulate liquidate.
}
```

**Verified**: Naming is correct, not inverted.

---

## Coverage Verified

### ✅ Reserves
- GPA filter: `dataSize=8624` + `market_memcmp`
- Fetches ALL reserves for main market
- Decodes prices from `reserve.marketPriceSf` (on-chain Scope oracle)
- Decodes `liquidation_threshold_pct` and `loan_to_value_pct`

### ✅ Obligations
- GPA filter: `dataSize=3344` + `market_memcmp`
- Fetches ALL obligations for main market
- Scans in-memory after download (no pagination gaps)
- Decodes live positions (deposits + borrows arrays)

### ✅ Elevation Groups
- Decoded from LendingMarket account
- Overrides reserve thresholds when `obligation.elevation_group != 0`
- Example: LST/SOL loops use 87% LTV / 92% liquidation (vs base 55%)
- **Fixed in commit ec2afb1** (was causing false CRITICAL before)

### ✅ Oracles
- Prices from on-chain `marketPriceSf` (U68F60 fixed-point)
- Scope max age: 512 slots
- No external feed required

---

## No Bugs Found

Checked and cleared:

1. ❌ Inverted naming (below_maint vs liquidatable)
2. ❌ Missing markets (main market 7u3He...5PfF covered)
3. ❌ Missing reserves (comprehensive GPA)
4. ❌ Missing oracles (prices from on-chain state)
5. ❌ Stale prices (Scope age limit enforced)
6. ❌ Wrong discriminators (pinned from klend-sdk@11.0.1)
7. ❌ LTV math error (unit tests pass)
8. ❌ Borrow factor bug (correctly applied)
9. ❌ Elevation group missing (decoded and applied)
10. ❌ Dust positions (filtered via MIN_LIQUIDATION_NOTIONAL_USD)

---

## Test Path Problem: Where Are Unhealthy Positions?

**Challenge**: Mainnet has ZERO liquidatable positions right now. How do we test the liquidation path?

### Recommended: Historical Snapshot (Path 1)

**Concept**: Fetch real mainnet accounts from past liquidations using archive RPC.

**Steps**:
1. Find historical slots with liquidations (Dune Analytics / Solscan API)
   - Target: Oracle shocks (SOL -20%), USDC depeg (0.88), FTX collapse
2. Build snapshot fetcher tool (`bins/snapshot`)
   - Fetch obligation + reserves at specific slot
   - Uses `getAccountInfo` with `minContextSlot` (archive RPC)
3. Validate health < 1.0 at that slot
4. Store as test fixtures
5. Shadow mode replays with historical state

**Effort**: 4-6 hours (includes archive RPC setup)

**Alternatives**:
- **Path 2**: Staging/devnet manual positions (high effort, synthetic)
- **Path 3**: Local validator (blocked — Kamino programs proprietary)
- **Path 4**: Wait for market event (passive monitoring, unpredictable)

**Details**: See `KAMINO_TEST_PATH.md` in PR

---

## Deliverables Created

### 1. Investigation Report
**File**: `INVESTIGATION_KAMINO_LIQUIDATION.md`

- Complete evidence with code line numbers
- Health formula explanation
- Coverage verification
- Market interpretation
- Statistical analysis

### 2. Test Path Guide
**File**: `KAMINO_TEST_PATH.md`

- 4 test approaches (historical snapshot recommended)
- Implementation checklist for snapshot fetcher
- Archive RPC provider list
- Expected effort estimates

### 3. Labeled Fixtures
**Files**: 
- `fixtures/kamino/unhealthy_obligation_LOCAL_SYNTHETIC.json`
- `fixtures/kamino/README.md`

**⚠️ CLEARLY LABELED AS SYNTHETIC**

Demonstrates unhealthy position ($100 collateral, $70 debt, HF=0.857) based on unit tests. NOT mainnet evidence.

Use case: Test liquidate_v2 builders without requiring mainnet liquidations.

---

## Recommended Next Action

**For LiquidSwords Engineer**:

Implement **Historical Snapshot Fetcher** (4-6 hour task):

1. Add archive RPC support to `JsonRpcBootstrap`
   - `get_account_info_at_slot(pubkey, slot)`
   - `get_multiple_accounts_at_slot(pubkeys, slot)`

2. Create `bins/snapshot/src/main.rs`
   ```bash
   cargo run --bin snapshot -- \
     --slot 250123456 \
     --obligation Cz8jW3aSVLhXvP6ZE2UqBhkUZVf2VVWBwSyTU8UgnJN \
     --rpc-url "$ARCHIVE_RPC_URL"
   ```

3. Find 3-5 historical liquidation slots
   - Dune: `SELECT slot FROM kamino.liquidations WHERE slot > 250000000 LIMIT 20`
   - Solscan: Search for `liquidateObligationAndRedeemReserveCollateralV2` txs

4. Capture snapshots and validate health < 1.0

**Why**: Tests liquidation path with real edge cases (elevation groups, referrer states, dust, etc.)

---

## For Review

**PR #1**: https://github.com/nicedreamsbt/Grokbotliq/pull/1

**Status**: Draft (documentation + fixtures only, no code changes)

**Contains**:
- Full investigation report with evidence
- Test path recommendations
- Labeled synthetic fixtures
- Implementation guide for snapshot fetcher

**Approval needed?**: Up to you. This is investigation output, not production code.

---

## Questions?

**Q: Why not just increase sensitivity to capture below_maint as liquidatable?**  
A: Would cause 2,423 false liquidations. These positions are mathematically safe (health > 1.0). Protocol would reject txs with `Custom 6016 ObligationHealthy`.

**Q: Could reserves be missing?**  
A: No. GPA with `dataSize=8624` + market memcmp fetches all Klend reserves. Discovery logs show all were decoded successfully with prices.

**Q: Could prices be stale?**  
A: No. Prices are from on-chain `reserve.marketPriceSf` which Scope updates within 512 slots. Discovery reads current state at scan slot.

**Q: How do we test without mainnet liquidations?**  
A: Historical snapshot (recommended) or staging manual positions. See `KAMINO_TEST_PATH.md`.

---

## Summary

✅ **liquidatable=0 is CORRECT**  
✅ **below_maint=2423 is expected healthy market behavior**  
✅ **Discovery/decode/health logic all validated**  
✅ **Test path documented (historical snapshot recommended)**

Zero bugs. Codebase is working as designed.

---

**Investigation complete.**  
**Ready for next assignment or historical snapshot implementation.**
