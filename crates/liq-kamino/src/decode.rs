//! Minimal klend account decode for liquidation planning (health + meta fields).
//!
//! Layout is a **simplified planning fixture format**, not full zero-copy Anchor
//! Obligation/Reserve. Live bytes from mainnet need the full IDL layout before
//! production; fixture / shadow paths use this packed header:
//!
//! ```text
//! [0..8)   magic b"KOBLplan"
//! [8]      version = 1
//! [9]      n_deposits
//! [10]     n_borrows
//! [11..19) deposited_amount u64 LE (first deposit)
//! [19..21) liq_threshold_bps u16 LE
//! [21]     deposit_decimals
//! [22..54) deposit_mint [u8;32]
//! [54..86) deposit_reserve [u8;32]
//! [86..94) borrowed_amount u64 LE (first borrow)
//! [94]     borrow_decimals
//! [95..127) borrow_mint
//! [127..159) borrow_reserve
//! [159..191) market
//! ```

use crate::{KaminoBorrow, KaminoDeposit, KaminoObligation};
use liq_core::Pubkey;
use thiserror::Error;

pub const OBLIGATION_MAGIC: &[u8; 8] = b"KOBLplan";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("account too short")]
    TooShort,
    #[error("bad magic")]
    BadMagic,
    #[error("unsupported version")]
    BadVersion,
}

/// Decode a planning-format obligation (fixtures / tests).
pub fn decode_obligation_planning(address: Pubkey, data: &[u8]) -> Result<KaminoObligation, DecodeError> {
    if data.len() < 191 {
        return Err(DecodeError::TooShort);
    }
    if &data[0..8] != OBLIGATION_MAGIC {
        return Err(DecodeError::BadMagic);
    }
    if data[8] != 1 {
        return Err(DecodeError::BadVersion);
    }
    let n_deposits = data[9];
    let n_borrows = data[10];
    let deposited_amount = u64::from_le_bytes(data[11..19].try_into().unwrap());
    let liq_threshold_bps = u16::from_le_bytes(data[19..21].try_into().unwrap());
    let deposit_decimals = data[21];
    let deposit_mint = Pubkey::from_bytes(&data[22..54]).ok_or(DecodeError::TooShort)?;
    let deposit_reserve = Pubkey::from_bytes(&data[54..86]).ok_or(DecodeError::TooShort)?;
    let borrowed_amount = u64::from_le_bytes(data[86..94].try_into().unwrap());
    let borrow_decimals = data[94];
    let borrow_mint = Pubkey::from_bytes(&data[95..127]).ok_or(DecodeError::TooShort)?;
    let borrow_reserve = Pubkey::from_bytes(&data[127..159]).ok_or(DecodeError::TooShort)?;
    let market = Pubkey::from_bytes(&data[159..191]).ok_or(DecodeError::TooShort)?;

    let mut deposits = Vec::new();
    if n_deposits > 0 {
        deposits.push(KaminoDeposit {
            reserve: deposit_reserve,
            mint: deposit_mint,
            deposited_amount,
            decimals: deposit_decimals,
            liq_threshold_bps,
        });
    }
    let mut borrows = Vec::new();
    if n_borrows > 0 {
        borrows.push(KaminoBorrow {
            reserve: borrow_reserve,
            mint: borrow_mint,
            borrowed_amount,
            decimals: borrow_decimals,
        });
    }
    Ok(KaminoObligation {
        address,
        market,
        deposits,
        borrows,
    })
}

/// Encode planning fixture bytes (tests / LIQ_FIXTURES offline path).
pub fn encode_obligation_planning(obl: &KaminoObligation) -> Vec<u8> {
    let mut d = Vec::with_capacity(191);
    d.extend_from_slice(OBLIGATION_MAGIC);
    d.push(1);
    d.push(obl.deposits.len().min(255) as u8);
    d.push(obl.borrows.len().min(255) as u8);
    let (dep_amt, thr, dep_dec, dep_mint, dep_res) = obl
        .deposits
        .first()
        .map(|x| {
            (
                x.deposited_amount,
                x.liq_threshold_bps,
                x.decimals,
                x.mint,
                x.reserve,
            )
        })
        .unwrap_or((0, 0, 0, Pubkey::default(), Pubkey::default()));
    d.extend_from_slice(&dep_amt.to_le_bytes());
    d.extend_from_slice(&thr.to_le_bytes());
    d.push(dep_dec);
    d.extend_from_slice(&dep_mint.0);
    d.extend_from_slice(&dep_res.0);
    let (bor_amt, bor_dec, bor_mint, bor_res) = obl
        .borrows
        .first()
        .map(|x| (x.borrowed_amount, x.decimals, x.mint, x.reserve))
        .unwrap_or((0, 0, Pubkey::default(), Pubkey::default()));
    d.extend_from_slice(&bor_amt.to_le_bytes());
    d.push(bor_dec);
    d.extend_from_slice(&bor_mint.0);
    d.extend_from_slice(&bor_res.0);
    d.extend_from_slice(&obl.market.0);
    d
}

/// Live mainnet Obligation header (IDL offsets; discriminator included at 0).
///
/// Offsets from klend IDL account layout (+8 disc):
/// - lending_market @ 32
/// - deposited_value_sf @ 1192
/// - borrowed_assets_market_value_sf @ 2224
/// - unhealthy_borrow_value_sf @ 2256
/// - has_debt @ 2287
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveObligationHeader {
    pub address: Pubkey,
    pub lending_market: Pubkey,
    pub owner: Pubkey,
    pub deposited_value_sf: u128,
    pub borrowed_assets_market_value_sf: u128,
    pub allowed_borrow_value_sf: u128,
    pub unhealthy_borrow_value_sf: u128,
    pub has_debt: bool,
}

/// Account data length for a full Obligation including Anchor discriminator.
pub const LIVE_OBLIGATION_DATASIZE: usize = 3344;

pub fn decode_obligation_live_header(
    address: Pubkey,
    data: &[u8],
) -> Result<LiveObligationHeader, DecodeError> {
    if data.len() < 2288 {
        return Err(DecodeError::TooShort);
    }
    let lending_market = Pubkey::from_bytes(&data[32..64]).ok_or(DecodeError::TooShort)?;
    let owner = Pubkey::from_bytes(&data[64..96]).ok_or(DecodeError::TooShort)?;
    let deposited_value_sf = u128::from_le_bytes(data[1192..1208].try_into().unwrap());
    let borrowed_assets_market_value_sf =
        u128::from_le_bytes(data[2224..2240].try_into().unwrap());
    let allowed_borrow_value_sf = u128::from_le_bytes(data[2240..2256].try_into().unwrap());
    let unhealthy_borrow_value_sf = u128::from_le_bytes(data[2256..2272].try_into().unwrap());
    let has_debt = data[2287] != 0;
    Ok(LiveObligationHeader {
        address,
        lending_market,
        owner,
        deposited_value_sf,
        borrowed_assets_market_value_sf,
        allowed_borrow_value_sf,
        unhealthy_borrow_value_sf,
        has_debt,
    })
}

/// True when on-chain SF values indicate liquidatable (borrowed > unhealthy threshold).
pub fn live_obligation_is_liquidatable(h: &LiveObligationHeader) -> bool {
    h.has_debt
        && h.borrowed_assets_market_value_sf > 0
        && h.borrowed_assets_market_value_sf > h.unhealthy_borrow_value_sf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{obligation_health, PriceMap};
    use liq_core::PriceFx;

    #[test]
    fn roundtrip_planning_obligation_and_health() {
        let mint_c = Pubkey::test(10, 1);
        let mint_d = Pubkey::test(10, 2);
        let obl = KaminoObligation {
            address: Pubkey::test(10, 3),
            market: Pubkey::test(10, 4),
            deposits: vec![KaminoDeposit {
                reserve: Pubkey::test(10, 5),
                mint: mint_c,
                deposited_amount: 1_000_000_000,
                decimals: 9,
                liq_threshold_bps: 8000,
            }],
            borrows: vec![KaminoBorrow {
                reserve: Pubkey::test(10, 6),
                mint: mint_d,
                borrowed_amount: 500_000_000,
                decimals: 9,
            }],
        };
        let bytes = encode_obligation_planning(&obl);
        let decoded = decode_obligation_planning(obl.address, &bytes).unwrap();
        assert_eq!(decoded.deposits[0].deposited_amount, 1_000_000_000);
        assert_eq!(decoded.borrows[0].borrowed_amount, 500_000_000);
        let prices = PriceMap {
            prices: vec![
                (mint_c, PriceFx::from_f64(100.0)),
                (mint_d, PriceFx::from_f64(100.0)),
            ],
        };
        let (hf, _, _) = obligation_health(&decoded, &prices).unwrap();
        assert!(hf.to_f64() > 1.0);
    }

    #[test]
    fn live_header_offsets_roundtrip_synthetic() {
        let mut data = vec![0u8; LIVE_OBLIGATION_DATASIZE];
        let market = Pubkey::test(11, 1);
        let owner = Pubkey::test(11, 2);
        data[32..64].copy_from_slice(&market.0);
        data[64..96].copy_from_slice(&owner.0);
        let borrowed: u128 = 200;
        let unhealthy: u128 = 100;
        data[2224..2240].copy_from_slice(&borrowed.to_le_bytes());
        data[2256..2272].copy_from_slice(&unhealthy.to_le_bytes());
        data[2287] = 1;
        let h = decode_obligation_live_header(Pubkey::test(11, 3), &data).unwrap();
        assert_eq!(h.lending_market, market);
        assert_eq!(h.owner, owner);
        assert!(live_obligation_is_liquidatable(&h));
    }
}
