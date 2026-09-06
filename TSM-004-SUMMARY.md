# TSM-004: Yellowstone gRPC Rust Client Implementation

## Status: ✅ COMPLETE

**PR**: https://github.com/nicedreamsbt/Grokbotliq/pull/2  
**Branch**: `cursor/yellowstone-real-client-9436`

---

## What Was Done

### 1. Replaced Stub with Real yellowstone-grpc Client

**File**: `crates/liq-streaming/src/yellowstone.rs`

- Replaced no-op stub in `YellowstoneSubscriber::subscribe()` with real gRPC client
- Kept existing public API unchanged: `GeyserSubscriber` trait + `StreamEvent` shapes
- Feature-gated behind `yellowstone` (default enabled) for DRY_RUN fallback

### 2. Dependencies Added

**File**: `crates/liq-streaming/Cargo.toml`

```toml
yellowstone-grpc-client = { version = "13.5", optional = true }
yellowstone-grpc-proto = { version = "12.7", optional = true }
futures = { workspace = true, optional = true }

[features]
default = ["yellowstone"]
yellowstone = ["yellowstone-grpc-client", "yellowstone-grpc-proto", "futures"]
```

**Workspace**: Added `futures = "0.3"` to `Cargo.toml`

### 3. Klend Filter Implementation

Subscribes to Klend program owner (`KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`) with datasize filters:

- **Obligations**: `datasize = 3344` (matches `discovery::known::KLEND_OBLIGATION_DATASIZE`)
- **Reserves**: `datasize = 8624` (matches `discovery::known::KLEND_RESERVE_DATASIZE`)

Uses yellowstone's `SubscribeRequestFilterAccountsFilter` with `AccountsFilterOneof::Datasize(n)` variant.

### 4. Environment Configuration

Loads from environment:
- `GEYSER_ENDPOINT` (required, e.g., `https://grpc.example.com:443`)
- `GEYSER_X_TOKEN` (required auth header, **redacted in logs**)
- `GEYSER_COMMITMENT` (optional, defaults to `processed`)
- `GEYSER_PING_MS` (optional, defaults to `15000`)

Missing endpoint → `YellowstoneConfig::from_env()` returns `None` (safe for DRY_RUN).

### 5. Event Translation

Maps yellowstone gRPC updates → existing `StreamEvent`:

```rust
UpdateOneof::Account → StreamEvent::Account(AccountUpdate {
    pubkey, slot, write_version, data, owner,
    source: UpdateSource::Geyser
})

UpdateOneof::Slot → StreamEvent::Slot(SlotUpdate {
    slot, parent, root: (status == 2)  // Rooted
})
```

Spawns async task draining gRPC stream into `mpsc::Receiver<StreamEvent>` (consumed by `StreamDetectionPath::ingest`).

### 6. Rust Version Upgrade

**File**: `rust-toolchain.toml`

Changed: `1.85.0` → `1.93.0` (required by yellowstone deps: tonic 0.14.6, solana-pubkey 4.3.0)

---

## Testing

```bash
✅ cargo test -p liq-streaming --lib    # All 25 tests pass
✅ cargo test --workspace --lib          # All 109 tests pass
```

New tests added:
- `subscribe_errors_without_token` — validates credential check
- `klend_datasize_constants` — verifies constants match discovery.rs

---

## Public API (Unchanged)

```rust
pub struct SubscribeFilter {
    pub owners: Vec<Pubkey>,
    pub accounts: Vec<Pubkey>,
}

#[async_trait]
pub trait GeyserSubscriber: Send + Sync {
    async fn subscribe(&self, filter: SubscribeFilter)
        -> Result<mpsc::Receiver<StreamEvent>, StreamError>;
}

pub enum StreamEvent {
    Account(AccountUpdate),
    Slot(SlotUpdate),
    Price { ... },
}
```

No changes to `StreamDetectionPath`, `HotPathCache`, or liquidator wiring.

---

## Gaps / Not Implemented

### 1. **Live Production Testing**
- **Reason**: Requires secrets (`GEYSER_X_TOKEN`) not available in CI
- **Current state**: Compiles + unit tests pass; stub fallback when credentials missing
- **Next step**: Integration test in staging with real geyser endpoint

### 2. **Auto-Reconnect**
- yellowstone-grpc-client v13.1+ supports `ReconnectConfig::default()`
- Not enabled yet (basic connection only)
- **Next step**: Add `.reconnect_config(ReconnectConfig::default())` to client builder

### 3. **Oracle Subscriptions**
- Per constraint: "only what existing watch/config / OracleTriggerPath already expects"
- No oracle-specific filters added (Klend obligations/reserves use Scope oracles via on-chain refs)
- **Gap**: Scope program (`HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ`) not directly subscribed

### 4. **kFarms Filtering**
- Program ID known (`FarmsPZpWu9i7Kky8tPN37rs2TpmMrAZrC7S7vJa91Hr`)
- Not filtered by datasize (farm account size unknown)
- **Gap**: Liquidator farm account discovery may be HTTP-only

### 5. **Compressed Account Filters**
- yellowstone v13.1+ supports `CompressedAccountFilterSet` (cuckoo filters for millions of accounts)
- Not used yet (current filter: owner + datasize pins only)
- **Next step**: If tracking >10k specific obligations, migrate to cuckoo filter

---

## Wire Path Verification

1. **Config load**: `YellowstoneConfig::from_env()` → `Some(config)` if `GEYSER_ENDPOINT` present
2. **Subscribe**: `YellowstoneSubscriber::subscribe(filter)` → opens gRPC stream
3. **Translation**: Spawned task maps `SubscribeUpdate` → `StreamEvent`
4. **Consumption**: `mpsc::Receiver<StreamEvent>` → `StreamDetectionPath::ingest` (liquidator HotPathCache)

Existing liquidator code already drains receiver; no changes needed to detection path.

---

## Files Modified

```
Cargo.lock                                  # +dependencies
Cargo.toml                                  # +futures workspace dep
rust-toolchain.toml                         # 1.85.0 → 1.93.0
crates/liq-streaming/Cargo.toml             # +yellowstone deps + feature
crates/liq-streaming/src/yellowstone.rs     # stub → real client (170 lines → 330 lines)
```

---

## Follow-Up Tasks (Post-Merge)

1. **Staging test** with real geyser credentials (`GEYSER_ENDPOINT` + `GEYSER_X_TOKEN`)
2. **Enable auto-reconnect** (`.reconnect_config(ReconnectConfig::default())`)
3. **Profile datasize filters** — confirm obligations (3344) + reserves (8624) capture all Klend accounts
4. **Oracle gap assessment** — determine if Scope program subscription needed or on-chain refs sufficient
5. **kFarms clarification** — confirm farm accounts discoverable via HTTP or add geyser filter

---

## Success Criteria Met

✅ Stub replaced with real client path  
✅ Builds + tests green (`cargo test -p liq-streaming --lib`)  
✅ PR opened: https://github.com/nicedreamsbt/Grokbotliq/pull/2  
✅ Klend filters configured (obligations 3344, reserves 8624)  
✅ Env var loading (`GEYSER_ENDPOINT` + `GEYSER_X_TOKEN`, redacted in logs)  
✅ DRY_RUN gates intact (feature fallback)  
✅ Public API unchanged (`StreamEvent` shapes preserved)  
✅ Documentation gap noted (production liquidator integration requires secrets)

---

**Toolsmith notes**: Ready for artifacts/manager/tsm-004-yellowstone-rust-client.md on nicedreams.
