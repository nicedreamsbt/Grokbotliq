# Protocol Research — Grokbotliq

Research date: 2026-09-05 (America/Phoenix). Sources: official GitHub READMEs, kamino.com docs, docs.0.xyz, docs.save.finance. No private credentials used.

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

Related: klend, klend-sdk, terminator, kbots-rust-public, scope, scope-sdk.

### Math / ordering

- Liquidatable when borrowed > sum(deposit * liq_threshold)
- Ix: liquidate_obligation_and_redeem_reserve_collateral_v2
- Order: ComputeBudget -> refresh reserves/obligation -> liquidate v2
- Bonus ~5-10%; Scope oracle 512 slots

### TODOs

- IDL discriminators; live close-factor/bonus; borsh layouts

## 2. Project 0

| Env | Address |
|-----|---------|
| Mainnet | `MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA` |
| Staging | `stag8sTKds2h4KzjUw3zKTsxbqvT4XKHdaR9X9E6Rct` |

Sources: docs.0.xyz program-addresses.

- Maint health < 0 => liquidatable
- Classic ~2.5%+2.5%; receivership start/end, max fee ~10% (FeeState)
- TODOs: IDL discriminators, FeeState PDA, group pubkey

## 3. Save Finance

- Program: `So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo`
- Source: docs.save.finance/architecture/addresses.md
- LiquidateObligation; close factor classic 50%; RefreshReserve -> RefreshObligation -> Liquidate
- Upgrade authority: `2Fwvr3MKhHhqakgjjEWcpWZZabbRCetHjukHi1zfKxjk`
- Market owner: `5pHk2TmnqQzRF9L6egy5FfiyBgS7G9cMZ5RFaJAvghzw`
- Fee receiver: `9RuqAN42PTUi9ya59k9suGATrkqzvb9gk2QABJtQzGP5`
- SLND mint: `SLNDpmoWTVADgEdndyvWzroNL7zSi1dF9PC3xHGtPwp`

## 4. Needs live credentials

1. Geyser gRPC + auth
2. Private RPC
3. Jito block engine
4. Keypair + ATAs
5. Live market/bank/reserve pubkeys
6. IDL pin
7. FeeState fields
8. Per-asset params
