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
    /// Referrer wallet; default pubkey means no referrer.
    pub referrer: Pubkey,
}

/// Account data length for a full Obligation including Anchor discriminator.
pub const LIVE_OBLIGATION_DATASIZE: usize = 3344;
/// `referrer` pubkey offset (incl. disc) — after hasDebt @ 2287.
pub const LIVE_REFERRER_OFFSET: usize = 2288;

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
    let referrer = if data.len() >= LIVE_REFERRER_OFFSET + 32 {
        Pubkey::from_bytes(&data[LIVE_REFERRER_OFFSET..LIVE_REFERRER_OFFSET + 32])
            .ok_or(DecodeError::TooShort)?
    } else {
        Pubkey::default()
    };
    Ok(LiveObligationHeader {
        address,
        lending_market,
        owner,
        deposited_value_sf,
        borrowed_assets_market_value_sf,
        allowed_borrow_value_sf,
        unhealthy_borrow_value_sf,
        has_debt,
        referrer,
    })
}

/// True when on-chain SF values indicate liquidatable (borrowed > unhealthy threshold).
pub fn live_obligation_is_liquidatable(h: &LiveObligationHeader) -> bool {
    h.has_debt
        && h.borrowed_assets_market_value_sf > 0
        && h.borrowed_assets_market_value_sf > h.unhealthy_borrow_value_sf
}

/// True when obligation.referrer is a non-default pubkey (klend has_referrer).
pub fn live_obligation_has_referrer(h: &LiveObligationHeader) -> bool {
    h.referrer.0 != [0u8; 32]
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

// ---------------------------------------------------------------------------
// Live Obligation deposits / borrows (zero-copy style offsets from klend IDL)
// ---------------------------------------------------------------------------

/// ObligationCollateral size (IDL).
pub const OBLIGATION_COLLATERAL_SIZE: usize = 136;
/// ObligationLiquidity size (IDL).
pub const OBLIGATION_LIQUIDITY_SIZE: usize = 200;
/// deposits[8] starts at this offset (incl. 8-byte discriminator).
pub const LIVE_DEPOSITS_OFFSET: usize = 96;
/// borrows[5] starts here.
pub const LIVE_BORROWS_OFFSET: usize = 1208;
pub const LIVE_MAX_DEPOSITS: usize = 8;
pub const LIVE_MAX_BORROWS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveDepositPosition {
    pub reserve: Pubkey,
    pub deposited_amount: u64,
    pub market_value_sf: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveBorrowPosition {
    pub reserve: Pubkey,
    /// Scaled fraction borrowed amount (SF); use >0 to detect active debt.
    pub borrowed_amount_sf: u128,
    pub market_value_sf: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveObligationPositions {
    pub header: LiveObligationHeader,
    pub deposits: Vec<LiveDepositPosition>,
    pub borrows: Vec<LiveBorrowPosition>,
}

/// Decode deposit/borrow position arrays from a live Obligation account.
pub fn decode_obligation_live_positions(
    address: Pubkey,
    data: &[u8],
) -> Result<LiveObligationPositions, DecodeError> {
    let header = decode_obligation_live_header(address, data)?;
    if data.len() < LIVE_BORROWS_OFFSET + LIVE_MAX_BORROWS * OBLIGATION_LIQUIDITY_SIZE {
        return Err(DecodeError::TooShort);
    }
    let mut deposits = Vec::new();
    for i in 0..LIVE_MAX_DEPOSITS {
        let off = LIVE_DEPOSITS_OFFSET + i * OBLIGATION_COLLATERAL_SIZE;
        let reserve = Pubkey::from_bytes(&data[off..off + 32]).ok_or(DecodeError::TooShort)?;
        let deposited_amount = u64::from_le_bytes(data[off + 32..off + 40].try_into().unwrap());
        let market_value_sf = u128::from_le_bytes(data[off + 40..off + 56].try_into().unwrap());
        if deposited_amount == 0 && reserve.0 == [0u8; 32] {
            continue;
        }
        if deposited_amount > 0 || market_value_sf > 0 {
            deposits.push(LiveDepositPosition {
                reserve,
                deposited_amount,
                market_value_sf,
            });
        }
    }
    let mut borrows = Vec::new();
    for i in 0..LIVE_MAX_BORROWS {
        let off = LIVE_BORROWS_OFFSET + i * OBLIGATION_LIQUIDITY_SIZE;
        let reserve = Pubkey::from_bytes(&data[off..off + 32]).ok_or(DecodeError::TooShort)?;
        // cumulativeBorrowRateBsf = 48 bytes @ +32; lastBorrowedAtTimestamp u64 @ +80
        // borrowedAmountSf u128 @ +88
        let borrowed_amount_sf =
            u128::from_le_bytes(data[off + 88..off + 104].try_into().unwrap());
        let market_value_sf =
            u128::from_le_bytes(data[off + 104..off + 120].try_into().unwrap());
        if borrowed_amount_sf == 0 && reserve.0 == [0u8; 32] {
            continue;
        }
        if borrowed_amount_sf > 0 || market_value_sf > 0 {
            borrows.push(LiveBorrowPosition {
                reserve,
                borrowed_amount_sf,
                market_value_sf,
            });
        }
    }
    Ok(LiveObligationPositions {
        header,
        deposits,
        borrows,
    })
}

/// Pick repay (largest borrow MV) and withdraw (largest deposit MV) reserves.
pub fn pick_liquidate_reserves(
    pos: &LiveObligationPositions,
) -> Option<(Pubkey, Pubkey)> {
    let repay = pos
        .borrows
        .iter()
        .max_by_key(|b| b.market_value_sf)
        .map(|b| b.reserve)?;
    let withdraw = pos
        .deposits
        .iter()
        .max_by_key(|d| d.market_value_sf)
        .map(|d| d.reserve)?;
    Some((repay, withdraw))
}

// ---------------------------------------------------------------------------
// Live Reserve vault metas (liquidity + collateral)
// ---------------------------------------------------------------------------

/// Reserve.liquidity starts at this offset (incl. disc).
pub const LIVE_RESERVE_LIQUIDITY_OFFSET: usize = 128;
/// Reserve.collateral starts here.
pub const LIVE_RESERVE_COLLATERAL_OFFSET: usize = 2560;
pub const LIVE_RESERVE_DATASIZE: usize = 8624;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveReserveVaults {
    pub address: Pubkey,
    pub lending_market: Pubkey,
    pub farm_collateral: Pubkey,
    pub farm_debt: Pubkey,
    pub liquidity_mint: Pubkey,
    pub liquidity_supply: Pubkey,
    pub fee_vault: Pubkey,
    pub mint_decimals: u8,
    pub token_program: Pubkey,
    pub collateral_mint: Pubkey,
    pub collateral_supply: Pubkey,
    /// Oracle metas for refresh_reserve (from ReserveConfig.tokenInfo).
    pub pyth_oracle: Pubkey,
    pub switchboard_price: Pubkey,
    pub switchboard_twap: Pubkey,
    pub scope_prices: Pubkey,
}

pub fn decode_reserve_live_vaults(
    address: Pubkey,
    data: &[u8],
) -> Result<LiveReserveVaults, DecodeError> {
    if data.len() < LIVE_RESERVE_COLLATERAL_OFFSET + 72 {
        return Err(DecodeError::TooShort);
    }
    let lending_market = Pubkey::from_bytes(&data[32..64]).ok_or(DecodeError::TooShort)?;
    let farm_collateral = Pubkey::from_bytes(&data[64..96]).ok_or(DecodeError::TooShort)?;
    let farm_debt = Pubkey::from_bytes(&data[96..128]).ok_or(DecodeError::TooShort)?;
    let liq = LIVE_RESERVE_LIQUIDITY_OFFSET;
    let liquidity_mint = Pubkey::from_bytes(&data[liq..liq + 32]).ok_or(DecodeError::TooShort)?;
    let liquidity_supply =
        Pubkey::from_bytes(&data[liq + 32..liq + 64]).ok_or(DecodeError::TooShort)?;
    let fee_vault = Pubkey::from_bytes(&data[liq + 64..liq + 96]).ok_or(DecodeError::TooShort)?;
    let mint_decimals = data[liq + 144] as u8; // u64 mintDecimals, low byte
    let token_program =
        Pubkey::from_bytes(&data[liq + 280..liq + 312]).ok_or(DecodeError::TooShort)?;
    let col = LIVE_RESERVE_COLLATERAL_OFFSET;
    let collateral_mint = Pubkey::from_bytes(&data[col..col + 32]).ok_or(DecodeError::TooShort)?;
    let collateral_supply =
        Pubkey::from_bytes(&data[col + 40..col + 72]).ok_or(DecodeError::TooShort)?;
    // TokenInfo oracles inside Reserve.config (config @ 4856, tokenInfo @ 5032)
    // scope @ 5112, switchboard price @ 5160, twap @ 5192, pyth @ 5224
    if data.len() < 5256 {
        return Err(DecodeError::TooShort);
    }
    let scope_prices = Pubkey::from_bytes(&data[5112..5144]).ok_or(DecodeError::TooShort)?;
    let switchboard_price = Pubkey::from_bytes(&data[5160..5192]).ok_or(DecodeError::TooShort)?;
    let switchboard_twap = Pubkey::from_bytes(&data[5192..5224]).ok_or(DecodeError::TooShort)?;
    let pyth_oracle = Pubkey::from_bytes(&data[5224..5256]).ok_or(DecodeError::TooShort)?;
    Ok(LiveReserveVaults {
        address,
        lending_market,
        farm_collateral,
        farm_debt,
        liquidity_mint,
        liquidity_supply,
        fee_vault,
        mint_decimals,
        token_program,
        collateral_mint,
        collateral_supply,
        pyth_oracle,
        switchboard_price,
        switchboard_twap,
        scope_prices,
    })
}

/// Documented main-market authority PDA (seeds: b"lma" + market), bump 248.
/// PDA `["lma", main_market]` under Klend — verified via find_program_address.
pub const KLEND_MAIN_MARKET_AUTHORITY: &str = "9DrvZvyWh1HuAoZxvYWMvkf2XCzryCpGgHqrMjyDWpmo";

/// Derive lending market authority PDA: seeds `b"lma" + market` under Klend.
pub fn lending_market_authority(market: &Pubkey) -> Pubkey {
    liq_core::find_program_address(&[b"lma", market.0.as_ref()], &liq_core::programs::klend()).0
}

#[cfg(test)]
mod live_position_tests {
    use super::*;

    #[test]
    fn live_positions_extract_deposit_and_borrow() {
        let mut data = vec![0u8; LIVE_OBLIGATION_DATASIZE];
        let market = Pubkey::test(20, 1);
        let owner = Pubkey::test(20, 2);
        data[32..64].copy_from_slice(&market.0);
        data[64..96].copy_from_slice(&owner.0);
        let dep_res = Pubkey::test(20, 3);
        let bor_res = Pubkey::test(20, 4);
        let dep_off = LIVE_DEPOSITS_OFFSET;
        data[dep_off..dep_off + 32].copy_from_slice(&dep_res.0);
        data[dep_off + 32..dep_off + 40].copy_from_slice(&1_000u64.to_le_bytes());
        data[dep_off + 40..dep_off + 56].copy_from_slice(&50u128.to_le_bytes());
        let bor_off = LIVE_BORROWS_OFFSET;
        data[bor_off..bor_off + 32].copy_from_slice(&bor_res.0);
        data[bor_off + 88..bor_off + 104].copy_from_slice(&10u128.to_le_bytes());
        data[bor_off + 104..bor_off + 120].copy_from_slice(&40u128.to_le_bytes());
        data[2224..2240].copy_from_slice(&40u128.to_le_bytes());
        data[2256..2272].copy_from_slice(&30u128.to_le_bytes());
        data[2287] = 1;
        let pos = decode_obligation_live_positions(Pubkey::test(20, 5), &data).unwrap();
        assert_eq!(pos.deposits.len(), 1);
        assert_eq!(pos.deposits[0].reserve, dep_res);
        assert_eq!(pos.deposits[0].deposited_amount, 1_000);
        assert_eq!(pos.borrows.len(), 1);
        assert_eq!(pos.borrows[0].reserve, bor_res);
        let (repay, withdraw) = pick_liquidate_reserves(&pos).unwrap();
        assert_eq!(repay, bor_res);
        assert_eq!(withdraw, dep_res);
    }

    #[test]
    fn live_reserve_vault_offsets() {
        let mut data = vec![0u8; LIVE_RESERVE_DATASIZE];
        let market = Pubkey::test(21, 1);
        let mint = Pubkey::test(21, 2);
        let supply = Pubkey::test(21, 3);
        let fee = Pubkey::test(21, 4);
        let c_mint = Pubkey::test(21, 5);
        let c_supply = Pubkey::test(21, 6);
        let token = Pubkey::test(21, 7);
        data[32..64].copy_from_slice(&market.0);
        let liq = LIVE_RESERVE_LIQUIDITY_OFFSET;
        data[liq..liq + 32].copy_from_slice(&mint.0);
        data[liq + 32..liq + 64].copy_from_slice(&supply.0);
        data[liq + 64..liq + 96].copy_from_slice(&fee.0);
        data[liq + 144] = 6;
        data[liq + 280..liq + 312].copy_from_slice(&token.0);
        let col = LIVE_RESERVE_COLLATERAL_OFFSET;
        data[col..col + 32].copy_from_slice(&c_mint.0);
        data[col + 40..col + 72].copy_from_slice(&c_supply.0);
        let v = decode_reserve_live_vaults(Pubkey::test(21, 8), &data).unwrap();
        assert_eq!(v.liquidity_mint, mint);
        assert_eq!(v.liquidity_supply, supply);
        assert_eq!(v.fee_vault, fee);
        assert_eq!(v.collateral_mint, c_mint);
        assert_eq!(v.collateral_supply, c_supply);
        assert_eq!(v.token_program, token);
        assert_eq!(v.mint_decimals, 6);
    }

    #[test]
    fn main_market_authority_decodes() {
        let pk = Pubkey::from_base58(KLEND_MAIN_MARKET_AUTHORITY).unwrap();
        let market = Pubkey::from_base58("7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF").unwrap();
        assert_eq!(lending_market_authority(&market), pk);
    }
}
