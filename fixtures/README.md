# Fixtures

Sample JSON used by `replay` and `shadow` (no live chain / no secrets).

| File | Purpose |
|------|---------|
| `oracle_ticks.json` | Oracle price ticks; later ticks cross borrower triggers |
| `borrowers.json` | Candidate snapshots + triggers + rough plan economics |

Pubkey fields use `{ "tag": u8, "index": u64 }` → `Pubkey::test(tag, index)`.

```bash
cargo run -p replay -- fixtures
cargo run -p shadow -- fixtures
```
