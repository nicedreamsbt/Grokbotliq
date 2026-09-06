# TSM-004: Type Flow Verification ✅

## Integration Path (Complete & Correct)

### 1. Yellowstone gRPC → AccountUpdate → StreamEvent
**File**: `crates/liq-streaming/src/yellowstone.rs:262-269`

```rust
let event = StreamEvent::Account(crate::AccountUpdate {
    pubkey,                                    // Pubkey from yellowstone
    slot: account_update.slot,                 // u64
    write_version: account_info.write_version, // u64
    data: account_info.data,                   // Vec<u8>
    owner,                                     // Pubkey
    source: UpdateSource::Geyser,              // ✅ Existing variant in liq_core::types
});
```

### 2. Types (Existing in lib.rs) ✅
**File**: `crates/liq-streaming/src/lib.rs:30-56`

```rust
pub struct AccountUpdate {
    pub pubkey: Pubkey,
    pub slot: u64,
    pub write_version: u64,
    pub data: Vec<u8>,
    pub owner: Pubkey,
    pub source: UpdateSource,  // liq_core::UpdateSource
}

pub enum StreamEvent {
    Account(AccountUpdate),  // ✅ Wraps AccountUpdate
    Slot(SlotUpdate),
    Price { ... },
}
```

### 3. UpdateSource (Existing in liq-core) ✅
**File**: `crates/liq-core/src/types.rs:87-92`

```rust
pub enum UpdateSource {
    Geyser,    // ✅ Used for yellowstone-grpc updates
    Rpc,
    Replay,
    Mock,
    Computed,
}
```

### 4. Liquidator Consumption ✅
**File**: `bins/liquidator/src/main.rs:302-306`

```rust
match ev {
    StreamEvent::Account(upd) => {
        apply_account_update(&store, &upd);  // ✅ Existing function
        metrics.oracle_updates.inc();
    }
    // ...
}
```

### 5. apply_account_update (Existing) ✅
**File**: `crates/liq-streaming/src/bootstrap.rs:620`

```rust
pub fn apply_account_update(store: &StateStore<Vec<u8>>, update: &crate::AccountUpdate) -> bool {
    // Upserts account into StateStore
    // Returns true if update applied (not stale)
}
```

---

## Future: StreamDetectionPath Integration

**When detection.rs merges**, the path will be:

```
yellowstone → AccountUpdate → StreamEvent::Account(upd)
  ↓
liquidator rx.recv()
  ↓
StreamDetectionPath::ingest(&event)  // ← Future refactor
  ↓
hot.ingest_account(to_observation(upd))
  ↓
HotPathCache
```

**Current** (correct for now):
```
yellowstone → AccountUpdate → StreamEvent::Account(upd)
  ↓
liquidator rx.recv()
  ↓
apply_account_update(&store, &upd)  // ← Direct call (works today)
  ↓
StateStore
```

---

## Verification Checklist ✅

- [x] AccountUpdate has all required fields (pubkey, slot, write_version, data, owner, source)
- [x] UpdateSource::Geyser exists in liq_core::types
- [x] StreamEvent::Account wraps AccountUpdate correctly
- [x] yellowstone.rs emits StreamEvent::Account with UpdateSource::Geyser
- [x] Liquidator imports and uses apply_account_update
- [x] apply_account_update exists in bootstrap.rs
- [x] TODO hooks added for StreamDetectionPath (detection.rs not merged yet)
- [x] No breaking changes to existing types
- [x] Token never logged (GEYSER_X_TOKEN redacted)
- [x] Stub fallthrough when credentials missing

---

## Engineer Requirements: ALL MET ✅

✅ **Emit into existing path**: yellowstone → AccountUpdate → StreamEvent::Account → mpsc channel  
✅ **Use existing types**: AccountUpdate, StreamEvent from lib.rs (not modified)  
✅ **UpdateSource variant**: UpdateSource::Geyser (already exists in liq_core)  
✅ **detection.rs**: LEFT ALONE (doesn't exist yet; TODO hooks added)  
✅ **File anchors**: lib.rs types ✅, detection.rs (not merged), cache.rs (not edited)  
✅ **Env**: GEYSER_ENDPOINT, GEYSER_X_TOKEN; token never logged ✅  
✅ **Stub fallthrough**: Falls back to MockGeyser when credentials missing ✅

---

**Status**: Type flow verified correct. Integration ready for StreamDetectionPath when available.
