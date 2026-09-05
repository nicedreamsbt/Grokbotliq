//! FeeState parsing skeleton from `type-crate/src/types/fee_state.rs`.
//!
//! On-chain account = 8-byte Anchor account discriminator + `FeeState` (repr C).
//! Field offsets below assume that layout (V1 body = 256 bytes after disc).

use crate::{account_disc, P0Error, DEFAULT_RECEIVERSHIP_MAX_FEE_BPS};
use liq_core::Pubkey;
use serde::{Deserialize, Serialize};

/// FeeState fields we care about for liquidation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeeStateView {
    pub liquidation_max_fee_bps: u16,
    pub liquidation_flat_sol_fee_lamports: u64,
    pub global_fee_wallet: Pubkey,
    pub paused: bool,
}

impl Default for FeeStateView {
    fn default() -> Self {
        Self {
            liquidation_max_fee_bps: DEFAULT_RECEIVERSHIP_MAX_FEE_BPS,
            liquidation_flat_sol_fee_lamports: 0,
            global_fee_wallet: Pubkey::default(),
            paused: false,
        }
    }
}

/// Offsets within the FeeState struct body (after optional 8-byte account disc).
mod off {
    pub const KEY: usize = 0;
    pub const GLOBAL_FEE_ADMIN: usize = 32;
    pub const GLOBAL_FEE_WALLET: usize = 64;
    pub const LIQUIDATION_MAX_FEE: usize = 112; // WrappedI80F48
    pub const PANIC_STATE: usize = 160; // PanicState (32 bytes)
    pub const LIQUIDATION_FLAT_SOL_FEE: usize = 200; // u32
    pub const V1_LEN: usize = 256;
}

const I80F48_FRAC_BITS: u32 = 48;

/// Convert I80F48 LE bytes to basis points (rounded toward zero).
pub fn i80f48_le_to_bps(bytes: &[u8; 16]) -> Result<u16, P0Error> {
    let raw = i128::from_le_bytes(*bytes);
    if raw < 0 {
        return Err(P0Error::FeeState("negative fee"));
    }
    // bps = round(raw * 10000 / 2^48)
    let bps = (raw.saturating_mul(10_000) + (1i128 << (I80F48_FRAC_BITS - 1))) >> I80F48_FRAC_BITS;
    if bps > u16::MAX as i128 {
        return Err(P0Error::FeeState("fee bps overflow"));
    }
    Ok(bps as u16)
}

/// Build I80F48 LE bytes from a rational `numer/denom` (for fixtures).
pub fn i80f48_from_ratio(numer: i128, denom: i128) -> [u8; 16] {
    // value = (numer/denom) * 2^48
    let v = (numer << I80F48_FRAC_BITS) / denom;
    v.to_le_bytes()
}

fn read_pubkey(data: &[u8], at: usize) -> Result<Pubkey, P0Error> {
    let slice = data
        .get(at..at + 32)
        .ok_or(P0Error::FeeState("truncated pubkey"))?;
    let mut b = [0u8; 32];
    b.copy_from_slice(slice);
    Ok(Pubkey::new(b))
}

/// Parse FeeState account bytes. Accepts either raw struct or disc-prefixed account data.
pub fn parse_fee_state(data: &[u8]) -> Result<FeeStateView, P0Error> {
    let body = if data.len() >= 8 + off::V1_LEN && data[..8] == account_disc::FEE_STATE {
        &data[8..]
    } else if data.len() >= off::V1_LEN {
        data
    } else if data.len() >= 8 && data[..8] == account_disc::FEE_STATE {
        return Err(P0Error::FeeState("body too short after disc"));
    } else {
        return Err(P0Error::FeeState("too short"));
    };

    let mut fee_bytes = [0u8; 16];
    fee_bytes.copy_from_slice(
        body.get(off::LIQUIDATION_MAX_FEE..off::LIQUIDATION_MAX_FEE + 16)
            .ok_or(P0Error::FeeState("missing liquidation_max_fee"))?,
    );
    let liquidation_max_fee_bps = i80f48_le_to_bps(&fee_bytes)?;

    let flat = body
        .get(off::LIQUIDATION_FLAT_SOL_FEE..off::LIQUIDATION_FLAT_SOL_FEE + 4)
        .ok_or(P0Error::FeeState("missing flat fee"))?;
    let liquidation_flat_sol_fee_lamports = u32::from_le_bytes(flat.try_into().unwrap()) as u64;

    let global_fee_wallet = read_pubkey(body, off::GLOBAL_FEE_WALLET)?;

    // PanicState.pause_flags at start of panic_state
    let pause_flags = *body
        .get(off::PANIC_STATE)
        .ok_or(P0Error::FeeState("missing panic_state"))?;
    let paused = (pause_flags & 1) != 0;

    // Silence unused offset warnings for documentation
    let _ = (off::KEY, off::GLOBAL_FEE_ADMIN);

    Ok(FeeStateView {
        liquidation_max_fee_bps,
        liquidation_flat_sol_fee_lamports,
        global_fee_wallet,
        paused,
    })
}

/// Build a minimal fixture FeeState body (V1_LEN zeros + filled fields) for unit tests.
pub fn fixture_fee_state_bytes(
    max_fee_bps: u16,
    flat_sol_fee: u32,
    wallet: Pubkey,
    paused: bool,
) -> Vec<u8> {
    let mut body = vec![0u8; off::V1_LEN];
    body[off::GLOBAL_FEE_WALLET..off::GLOBAL_FEE_WALLET + 32].copy_from_slice(&wallet.0);
    let fee = i80f48_from_ratio(max_fee_bps as i128, 10_000);
    body[off::LIQUIDATION_MAX_FEE..off::LIQUIDATION_MAX_FEE + 16].copy_from_slice(&fee);
    body[off::LIQUIDATION_FLAT_SOL_FEE..off::LIQUIDATION_FLAT_SOL_FEE + 4]
        .copy_from_slice(&flat_sol_fee.to_le_bytes());
    if paused {
        body[off::PANIC_STATE] = 1;
    }
    let mut out = account_disc::FEE_STATE.to_vec();
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture_10pct_fee() {
        let wallet = Pubkey::test(9, 9);
        let bytes = fixture_fee_state_bytes(1000, 5_000, wallet, false);
        let view = parse_fee_state(&bytes).unwrap();
        assert_eq!(view.liquidation_max_fee_bps, 1000);
        assert_eq!(view.liquidation_flat_sol_fee_lamports, 5_000);
        assert_eq!(view.global_fee_wallet, wallet);
        assert!(!view.paused);
    }

    #[test]
    fn parse_detects_pause_flag() {
        let bytes = fixture_fee_state_bytes(500, 0, Pubkey::default(), true);
        let view = parse_fee_state(&bytes).unwrap();
        assert!(view.paused);
        assert_eq!(view.liquidation_max_fee_bps, 500);
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(parse_fee_state(&[1, 2, 3]).is_err());
    }
}
