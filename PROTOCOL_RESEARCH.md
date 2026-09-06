# Protocol Research — Grokbotliq

Research date: 2026-09-05 (America/Phoenix). Sources: official GitHub READMEs, kamino.com docs, docs.0.xyz, docs.save.finance. No private credentials used.

## IDL pins (vendored)

| Protocol | Path | Version / pin | sha256 |
|----------|------|---------------|--------|
| Kamino klend full IDL | `crates/liq-kamino/idls/klend.json` | IDL `version` **1.25.0** via `@kamino-finance/klend-sdk@11.0.1` | `2a7e311eb33ffd79241e7cb8424e2170fb487a6ba678a115ce3d9a561375670d` |
| Kamino liquidation subset | `crates/liq-kamino/idls/klend_liquidation_subset.json` | same SDK pin; discriminators only | `288292e4e5b6a65f8a13eb83531225eff7a096bcf182b9722ec72ad7710efd20` |
| Project 0 / marginfi subset | `crates/liq-project0/idls/marginfi_liquidation_subset.json` | extracted 2026-09-05 from `0dotxyz/marginfi-v2` type-crate | `ce9b34af57c252784a2553fcb9dda3968406b5e3d52d80176d59148847408dfb` |
| Project 0 discriminators (Rust) | `crates/liq-project0/idls/discriminators.rs` | mirror of type-crate constants | — |
| Project 0 FeeState layout | `crates/liq-project0/idls/fee_state.rs` | type-crate `FeeState` | — |
| Project 0 type constants excerpt | `crates/liq-project0/idls/type_constants_excerpt.rs` | type-crate constants | — |
| Save / Solend ix tags | `crates/liq-save/idls/save_lending_ix.json` | pinned 2026-09-05; SPL token-lending enum | `a5ed2421818fdfed5bdf7ec76223ff5718e46ff2bc43b9418753d821d90e72de` |
| Save solend-sdk instruction source | `crates/liq-save/idls/solend_sdk_0.1.0_instruction.rs` | **solend-sdk 0.1.0** vendored excerpt | — |

Discriminator smoke tests live in `liq-kamino`, `liq-project0`, and `liq-save` unit tests.

## 1. Kamino Lending (klend)

### Program IDs (verified)

| Env | Program | Address |
|-----|---------|---------|
| Mainnet | Klend | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` |
| Staging | Klend | `SLendK7ySfcEzyaFqy93gDnD3RtrpXJcnRwb6zFHJSh` |
| Mainnet | Kvault | `KvauGMspG5k6rtzrqqn7WNn3oZdyKqLKwK2XWQ8FLjd` |
| Mainnet | Kfarms | `FarmsPZpWu9i7Kky8tPN37rs2TpmMrAZrC7S7vJa91Hr` |
| Mainnet | Scope | `HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ` |

Sources: Kamino-Finance/klend README; kamino.com program-addresses.

Documented main market (mainnet): `7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF` (kamino.com market-data / SDK default).
Market authority PDA `["lma", market]`: `9DrvZvyWh1HuAoZxvYWMvkf2XCzryCpGgHqrMjyDWpmo` (bump 248).

### Math / ordering

- Liquidatable when borrowed > sum(deposit * liq_threshold)
- Ix: `liquidate_obligation_and_redeem_reserve_collateral_v2`
- Order: ComputeBudget → refresh reserves/obligation → liquidate v2
- Bonus ~5–10%; Scope oracle max age **512 slots** (`SCOPE_MAX_AGE_SLOTS`)

### Discriminators (pinned)

| Ix | Bytes |
|----|-------|
| refreshReserve | `[2, 218, 138, 235, 79, 201, 25, 102]` |
| refreshObligation | `[33, 132, 147, 228, 151, 192, 72, 89]` |
| liquidateObligationAndRedeemReserveCollateralV2 | `[162, 161, 35, 143, 30, 187, 185, 103]` |

## 2. Project 0

| Env | Address |
|-----|---------|
| Mainnet | `MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA` |
| Staging | `stag8sTKds2h4KzjUw3zKTsxbqvT4XKHdaR9X9E6Rct` |

Sources: docs.0.xyz program-addresses; `0dotxyz/marginfi-v2`.

Main group (mainnet): `4qp6Fx6tnZkY5Wropq9wUYgtFxXKwE6viZxFHg3rdAG8` (docs.marginfi.com).

- Maint health < 0 ⇒ liquidatable
- Classic ~2.5% + 2.5%; receivership start/end; max fee ~10% (`FeeState`)
- Seeds: `feestate`, `liq_record`
- Classic liquidate Anchor sighash: `[214, 169, 151, 213, 251, 167, 86, 219]`

## 3. Save Finance

- Program: `So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo`
- Source: docs.save.finance/architecture/addresses.md
- Tags: RefreshReserve=3, RefreshObligation=7, LiquidateObligation=12, FlashLoan=13 (legacy), LiquidateAndRedeem=17, FlashBorrow=19, FlashRepay=20
- Close factor classic 50% (`DEFAULT_CLOSE_FACTOR_BPS = 5000`)
- Order: RefreshReserve* → RefreshObligation → LiquidateObligationAndRedeemReserveCollateral
- Upgrade authority: `2Fwvr3MKhHhqakgjjEWcpWZZabbRCetHjukHi1zfKxjk`
- Market owner: `5pHk2TmnqQzRF9L6egy5FfiyBgS7G9cMZ5RFaJAvghzw`
- Main lending market (classic pool): `4UpD2fh7xH3VP9QQaXtsS1YY3bxzWhtfpks7FatyKvdY`
- Fee receiver: `9RuqAN42PTUi9ya59k9suGATrkqzvb9gk2QABJtQzGP5`
- SLND mint: `SLNDpmoWTVADgEdndyvWzroNL7zSi1dF9PC3xHGtPwp`


## Flash loans / atomic funding

### Save / Solend

Vendored source: `crates/liq-save/idls/solend_sdk_0.1.0_instruction.rs`.

| Tag | Instruction | Data layout |
|-----|-------------|-------------|
| **13** | `FlashLoan` (legacy / deprecated CPI-receiver) | `u8 tag` + `u64 amount` |
| **19** | `FlashBorrowReserveLiquidity` (**preferred**) | `u8 tag` + `u64 liquidity_amount` |
| **20** | `FlashRepayReserveLiquidity` | `u8 tag` + `u64 liquidity_amount` + `u8 borrow_instruction_index` |

**Preferred atomic composition** (implemented in `liq-save::flash`):

1. ComputeBudget (limit + price)
2. RefreshReserve* (all touched reserves)
3. RefreshObligation
4. **FlashBorrowReserveLiquidity** (record ix index)
5. LiquidateObligationAndRedeemReserveCollateral
6. Optional swap (SwapRouter)
7. **FlashRepayReserveLiquidity** (same amount + borrow index)

FlashBorrow accounts (7): source_liquidity(mut), destination_liquidity(mut), reserve(mut), lending_market, lending_market_authority, instructions sysvar, token_program.

FlashRepay accounts (9): source(mut), destination(mut), fee_receiver(mut), host_fee_receiver(mut), reserve(mut), lending_market, user_transfer_authority(signer), instructions sysvar, token_program.

**Account metas (W/RO):** Reconciled against public solend-sdk 0.1.0 instruction helpers — FlashBorrow (7), FlashRepay (9), LiquidateAndRedeem (15). Unit tests lock writable vs readonly flags. Remaining uncertainty: Save mainnet may still diverge on tag 19/20 packing or host-fee account semantics before live submit.

Default modeled flash fee: 9 bps (`DEFAULT_FLASH_FEE_BPS`) until reserve `flash_loan_fee_wad` is read on-chain.

### Kamino (klend)

IDL (`klend.json`) includes `flashBorrowReserveLiquidity` / `flashRepayReserveLiquidity`.  
Discriminators (Anchor `sha256("global:<snake>")[0..8]`, same method as pinned refresh/liq):

| Ix | Bytes |
|----|-------|
| flash_borrow_reserve_liquidity | `[135, 231, 52, 167, 7, 52, 212, 193]` |
| flash_repay_reserve_liquidity | `[185, 117, 0, 203, 96, 245, 180, 186]` |

Args: borrow `{ liquidityAmount: u64 }`; repay `{ liquidityAmount: u64, borrowInstructionIndex: u8 }`.

**Discriminators:** User-verified 2026-09-05 against current klend-sdk codegen (borrow/repay arrays above). TODO closed for disc values.

**Optional referrer metas:** When `referrerTokenState` / `referrerAccount` absent, official codegen uses **KLend program ID as readonly** — not lending_market as a writable placeholder. Implemented in `liq-kamino::flash`.

`KAMINO_FLASH_SUPPORTED = true`; inventory + post-liq swap remains available if flash is disabled at runtime.

**refresh_obligation remaining accounts:** deposits (slot order) + borrows (slot order) [+ `ReferrerTokenState` PDA per borrow when `obligation.referrer ≠ default`]. Count mismatch → **Custom 6006 `InvalidAccountInput`**. Referrer PDA seeds: `["referrer_acc", referrer, reserve]`.

**liquidate_v2 farm placeholders:** official klend-sdk codegen uses **KLend program ID readonly** when optional farm accounts are absent (not the Farms program id). `farmsProgram` meta remains `FarmsPZp…`.

**Shadow liquidator ATAs:** derived via Associated Token Program (`[owner, token_program, mint]`) for `SHADOW_FEE_PAYER` / sim fee payer — no private key. Token program is Tokenkeg or Token-2022 from reserve decode (`token_program_from_mint_owner` helper available). Shadow/planner insert Associated Token Program **CreateIdempotent** (data byte `1`) for missing liquidator ATAs (repay liquidity, withdraw collateral, withdraw liquidity) **after refresh / before flash_borrow & liquidate_v2** so flash `borrowInstructionIndex` stays correct. After CreateIdempotent clears `AccountNotInitialized` (3012), liquidate_v2 may next hit **Custom 6009 `ReserveStale`** (reserve needs refresh — often because flash_borrow mutates the repay reserve before liquidate).

### Project 0 receivership (flash alternative)

Receivership is a first-class `FundingStrategy::Project0Receivership` that can avoid flash:

`ComputeBudget → start_liquidation → withdraw → [swap] → repay → end_liquidation`

Wire builders in `liq-project0::tx_builder` emit real `Instruction` lists (program_id, account metas, data bytes). Upstream also exposes `START_FLASHLOAN` / `END_FLASHLOAN` discriminators for marginfi flash loans (not required for receivership path).

## 4. Streaming / Yellowstone / RPC

- Trait + mock + **multi-provider freshness failover** in `liq-streaming`
- **HTTP JSON-RPC (reqwest)** `HttpJsonRpcTransport` + `JsonRpcBootstrap`: `getSlot`, `getHealth`, `getAccountInfo`, `getMultipleAccounts`, filtered `getProgramAccounts`, `simulateTransaction`
- **`RotatingRpcPool`**: `RPC_URLS` / `RPC_URL` from gitignored `config/local.env`; rotate on 429/5xx/timeout; host-only telemetry
- **Mainnet discovery**: known Klend market / marginfi group / Save market + filtered GPA; Klend `live_positions` (deposits/borrows) + reserve vault/oracle decode
- **Shadow strategy vtx**: `liq-execution::encode_versioned_tx_base64` (v0, unsigned dummy sigs) simulates real plan ixs (`simulateTransaction`, sigVerify=false) — not a CU-only stub
- **Save obligation pin**: dataSize **1300**, market memcmp offset **10** (classic main pool GPA sample)
- Yellowstone: stub retained; **prefer working RPC bootstrap** over half-broken gRPC until yellowstone-grpc is linked
- Env: `GEYSER_ENDPOINT`, `GEYSER_X_TOKEN`, optional `GEYSER_COMMITMENT`, `GEYSER_PING_MS`; `LIQ_FIXTURES` for offline CI; `LIQ_MAINNET_SHADOW=1`

## 5. Needs live credentials (still)

1. Geyser / Yellowstone gRPC endpoint + auth token
2. Private RPC URL
3. Jito block engine URL (+ auth UUID if required)
4. Funded liquidator keypair + ATAs (never commit)
5. Live market / bank / reserve pubkeys per protocol
6. Optional: re-pin IDLs when upstream releases
7. Live FeeState account for receivership max fee
8. Per-asset liq threshold / bonus / close-factor confirmation on-chain
9. Flash fee / Save tag 19/20 mainnet re-verify (Kamino flash discs verified)
10. Full Anchor zero-copy decode (planning fixture decode exists for Kamino obligations)
11. Yellowstone live gRPC client (RPC path is real; gRPC still stub)
12. VersionedTransaction signing + Jito auth
