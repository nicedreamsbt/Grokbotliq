//! Protocol-closer Klend obligation health from live deposits/borrows + reserve risk.
//!
//! Liquidatable when borrow-factor-adjusted debt > unhealthy borrow value, where
//! unhealthy = Σ(deposit_value × liquidation_threshold_pct / 100). Values are
//! recomputed from position amounts and reserve `marketPriceSf` (oracle-backed
//! price cached on the reserve), not stale obligation SF headers.

use crate::decode::{
    DecodeError, LiveObligationPositions, LiveReserveVaults, LIVE_RESERVE_COLLATERAL_OFFSET,
    LIVE_RESERVE_DATASIZE, LIVE_RESERVE_LIQUIDITY_OFFSET,
};
use crate::{KaminoError, PriceMap};
use liq_core::{HealthFx, PriceFx, Pubkey};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Klend `fixed` / U68F60 scale: value_sf = value × 2^60.
pub const FRACTION_SCALE_BITS: u32 = 60;
pub const FRACTION_SCALE: u128 = 1u128 << FRACTION_SCALE_BITS;

/// LendingMarket.elevationGroups array offset (incl. disc).
pub const LIVE_MARKET_ELEVATION_GROUPS_OFFSET: usize = 200;
pub const ELEVATION_GROUP_SIZE: usize = 72;
pub const MAX_ELEVATION_GROUPS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElevationGroupParams {
    pub id: u8,
    pub ltv_pct: u8,
    pub liquidation_threshold_pct: u8,
    pub max_liquidation_bonus_bps: u16,
}

/// Decode elevation groups from a LendingMarket account.
pub fn decode_lending_market_elevation_groups(
    data: &[u8],
) -> Result<HashMap<u8, ElevationGroupParams>, DecodeError> {
    let need = LIVE_MARKET_ELEVATION_GROUPS_OFFSET + MAX_ELEVATION_GROUPS * ELEVATION_GROUP_SIZE;
    if data.len() < need {
        return Err(DecodeError::TooShort);
    }
    let mut map = HashMap::new();
    for i in 0..MAX_ELEVATION_GROUPS {
        let off = LIVE_MARKET_ELEVATION_GROUPS_OFFSET + i * ELEVATION_GROUP_SIZE;
        let max_bonus = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
        let id = data[off + 2];
        let ltv_pct = data[off + 3];
        let liquidation_threshold_pct = data[off + 4];
        if id == 0 && ltv_pct == 0 && liquidation_threshold_pct == 0 {
            continue;
        }
        map.insert(
            id,
            ElevationGroupParams {
                id,
                ltv_pct,
                liquidation_threshold_pct,
                max_liquidation_bonus_bps: max_bonus,
            },
        );
    }
    Ok(map)
}

/// Reserve.config offset (incl. disc) — IDL Reserve layout.
pub const LIVE_RESERVE_CONFIG_OFFSET: usize = 4856;
/// loanToValuePct within ReserveConfig.
pub const RESERVE_LTV_PCT_OFFSET: usize = LIVE_RESERVE_CONFIG_OFFSET + 16;
/// liquidationThresholdPct within ReserveConfig.
pub const RESERVE_LIQ_THRESHOLD_PCT_OFFSET: usize = LIVE_RESERVE_CONFIG_OFFSET + 17;
/// borrowFactorPct (u64) within ReserveConfig.
pub const RESERVE_BORROW_FACTOR_PCT_OFFSET: usize = LIVE_RESERVE_CONFIG_OFFSET + 152;
/// marketPriceSf within Reserve.liquidity.
pub const RESERVE_MARKET_PRICE_SF_OFFSET: usize = LIVE_RESERVE_LIQUIDITY_OFFSET + 120;
/// totalAvailableAmount within Reserve.liquidity.
pub const RESERVE_AVAILABLE_OFFSET: usize = LIVE_RESERVE_LIQUIDITY_OFFSET + 96;
/// borrowedAmountSf within Reserve.liquidity.
pub const RESERVE_BORROWED_SF_OFFSET: usize = LIVE_RESERVE_LIQUIDITY_OFFSET + 104;
/// accumulatedProtocolFeesSf.
pub const RESERVE_PROTOCOL_FEES_SF_OFFSET: usize = LIVE_RESERVE_LIQUIDITY_OFFSET + 216;
/// accumulatedReferrerFeesSf.
pub const RESERVE_REFERRER_FEES_SF_OFFSET: usize = LIVE_RESERVE_LIQUIDITY_OFFSET + 232;
/// pendingReferrerFeesSf.
pub const RESERVE_PENDING_REFERRER_FEES_SF_OFFSET: usize = LIVE_RESERVE_LIQUIDITY_OFFSET + 248;
/// collateral.mintTotalSupply.
pub const RESERVE_COLLATERAL_MINT_SUPPLY_OFFSET: usize = LIVE_RESERVE_COLLATERAL_OFFSET + 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveReserveRisk {
    pub address: Pubkey,
    pub lending_market: Pubkey,
    pub liquidity_mint: Pubkey,
    pub mint_decimals: u8,
    pub loan_to_value_pct: u8,
    pub liquidation_threshold_pct: u8,
    pub borrow_factor_pct: u64,
    pub market_price_sf: u128,
    pub total_available_amount: u64,
    pub borrowed_amount_sf: u128,
    pub accumulated_protocol_fees_sf: u128,
    pub accumulated_referrer_fees_sf: u128,
    pub pending_referrer_fees_sf: u128,
    pub collateral_mint_total_supply: u64,
    pub pyth_oracle: Pubkey,
    pub switchboard_price: Pubkey,
    pub scope_prices: Pubkey,
}

impl LiveReserveRisk {
    /// Human price (USD per whole token) from reserve marketPriceSf.
    pub fn price_f64(&self) -> f64 {
        sf_to_f64(self.market_price_sf)
    }

    pub fn price_fx(&self) -> PriceFx {
        PriceFx::from_f64(self.price_f64().max(0.0))
    }

    /// Total liquidity supply in token base units (available + borrowed − fees).
    pub fn total_liquidity_tokens_f64(&self) -> f64 {
        let available = self.total_available_amount as f64;
        let borrowed = sf_to_f64(self.borrowed_amount_sf);
        let fees = sf_to_f64(self.accumulated_protocol_fees_sf)
            + sf_to_f64(self.accumulated_referrer_fees_sf)
            + sf_to_f64(self.pending_referrer_fees_sf);
        (available + borrowed - fees).max(0.0)
    }

    /// Convert obligation collateral shares → underlying liquidity base units.
    pub fn collateral_to_liquidity(&self, deposited_collateral: u64) -> f64 {
        let supply = self.collateral_mint_total_supply;
        if supply == 0 {
            return deposited_collateral as f64;
        }
        let total_liq = self.total_liquidity_tokens_f64();
        deposited_collateral as f64 * total_liq / supply as f64
    }
}

/// Decode reserve risk/price/threshold fields (and vault oracle pubkeys).
pub fn decode_reserve_live_risk(
    address: Pubkey,
    data: &[u8],
) -> Result<LiveReserveRisk, DecodeError> {
    if data.len() < LIVE_RESERVE_DATASIZE.min(5256) || data.len() < RESERVE_BORROW_FACTOR_PCT_OFFSET + 8
    {
        return Err(DecodeError::TooShort);
    }
    let vaults = crate::decode_reserve_live_vaults(address, data)?;
    let market_price_sf =
        u128::from_le_bytes(data[RESERVE_MARKET_PRICE_SF_OFFSET..RESERVE_MARKET_PRICE_SF_OFFSET + 16].try_into().unwrap());
    let total_available_amount =
        u64::from_le_bytes(data[RESERVE_AVAILABLE_OFFSET..RESERVE_AVAILABLE_OFFSET + 8].try_into().unwrap());
    let borrowed_amount_sf =
        u128::from_le_bytes(data[RESERVE_BORROWED_SF_OFFSET..RESERVE_BORROWED_SF_OFFSET + 16].try_into().unwrap());
    let accumulated_protocol_fees_sf = u128::from_le_bytes(
        data[RESERVE_PROTOCOL_FEES_SF_OFFSET..RESERVE_PROTOCOL_FEES_SF_OFFSET + 16]
            .try_into()
            .unwrap(),
    );
    let accumulated_referrer_fees_sf = u128::from_le_bytes(
        data[RESERVE_REFERRER_FEES_SF_OFFSET..RESERVE_REFERRER_FEES_SF_OFFSET + 16]
            .try_into()
            .unwrap(),
    );
    let pending_referrer_fees_sf = u128::from_le_bytes(
        data[RESERVE_PENDING_REFERRER_FEES_SF_OFFSET..RESERVE_PENDING_REFERRER_FEES_SF_OFFSET + 16]
            .try_into()
            .unwrap(),
    );
    let collateral_mint_total_supply = u64::from_le_bytes(
        data[RESERVE_COLLATERAL_MINT_SUPPLY_OFFSET..RESERVE_COLLATERAL_MINT_SUPPLY_OFFSET + 8]
            .try_into()
            .unwrap(),
    );
    let loan_to_value_pct = data[RESERVE_LTV_PCT_OFFSET];
    let liquidation_threshold_pct = data[RESERVE_LIQ_THRESHOLD_PCT_OFFSET];
    let borrow_factor_pct = u64::from_le_bytes(
        data[RESERVE_BORROW_FACTOR_PCT_OFFSET..RESERVE_BORROW_FACTOR_PCT_OFFSET + 8]
            .try_into()
            .unwrap(),
    );
    Ok(LiveReserveRisk {
        address,
        lending_market: vaults.lending_market,
        liquidity_mint: vaults.liquidity_mint,
        mint_decimals: vaults.mint_decimals,
        loan_to_value_pct,
        liquidation_threshold_pct,
        borrow_factor_pct: if borrow_factor_pct == 0 {
            100
        } else {
            borrow_factor_pct
        },
        market_price_sf,
        total_available_amount,
        borrowed_amount_sf,
        accumulated_protocol_fees_sf,
        accumulated_referrer_fees_sf,
        pending_referrer_fees_sf,
        collateral_mint_total_supply,
        pyth_oracle: vaults.pyth_oracle,
        switchboard_price: vaults.switchboard_price,
        scope_prices: vaults.scope_prices,
    })
}

/// Combine vault decode + risk (convenience when both needed).
pub fn decode_reserve_live_risk_and_vaults(
    address: Pubkey,
    data: &[u8],
) -> Result<(LiveReserveRisk, LiveReserveVaults), DecodeError> {
    let risk = decode_reserve_live_risk(address, data)?;
    let vaults = crate::decode_reserve_live_vaults(address, data)?;
    Ok((risk, vaults))
}

pub fn sf_to_f64(sf: u128) -> f64 {
    sf as f64 / FRACTION_SCALE as f64
}

pub fn f64_to_sf(v: f64) -> u128 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    (v * FRACTION_SCALE as f64).round() as u128
}

/// Token base units → USD micro (1e-6 USD) given reserve price SF.
pub fn tokens_to_usd_micro(token_amount: f64, decimals: u8, price_sf: u128) -> u128 {
    let usd = tokens_to_usd_f64(token_amount, decimals, price_sf);
    if usd <= 0.0 {
        0
    } else {
        (usd * 1_000_000.0).round() as u128
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KlendComputedHealth {
    pub deposited_value_usd_micro: u128,
    pub borrowed_value_usd_micro: u128,
    pub borrow_factor_adjusted_debt_usd_micro: u128,
    /// Σ deposit_value × liquidation_threshold_pct / 100
    pub unhealthy_borrow_value_usd_micro: u128,
    /// Σ deposit_value × loan_to_value_pct / 100
    pub allowed_borrow_value_usd_micro: u128,
    /// HF = unhealthy / bf_debt (1.0 = at liquidation threshold).
    pub health_factor: f64,
    /// LTV = bf_debt / deposited (0 if no deposits).
    pub ltv: f64,
    pub is_liquidatable: bool,
    /// bf_debt > allowed (above max LTV / "maintenance" for new borrows).
    pub below_maintenance: bool,
    pub missing_reserves: Vec<String>,
}

impl KlendComputedHealth {
    pub fn health_fx(&self) -> HealthFx {
        HealthFx::from_f64(self.health_factor)
    }
}

/// Minimum BF-adjusted debt (USD) to consider for liquidation candidacy.
/// Dust obligations round to ~$0 and must not be marked CRITICAL.
pub const MIN_LIQUIDATION_NOTIONAL_USD: f64 = 0.01;

/// Token base units → USD (f64) given reserve price SF.
pub fn tokens_to_usd_f64(token_amount: f64, decimals: u8, price_sf: u128) -> f64 {
    if token_amount <= 0.0 || price_sf == 0 {
        return 0.0;
    }
    let price = sf_to_f64(price_sf);
    let whole = token_amount / 10f64.powi(decimals as i32);
    let usd = whole * price;
    if !usd.is_finite() || usd <= 0.0 {
        0.0
    } else {
        usd
    }
}

/// Recompute klend health from live positions + reserve risk/price map.
pub fn compute_obligation_health_live(
    pos: &LiveObligationPositions,
    reserves: &HashMap<Pubkey, LiveReserveRisk>,
) -> Result<KlendComputedHealth, KaminoError> {
    compute_obligation_health_live_with_elevation(pos, reserves, None)
}

/// Like [`compute_obligation_health_live`] but applies LendingMarket elevation-group
/// LTV / liquidation threshold when `pos.header.elevation_group != 0`.
pub fn compute_obligation_health_live_with_elevation(
    pos: &LiveObligationPositions,
    reserves: &HashMap<Pubkey, LiveReserveRisk>,
    elevation_groups: Option<&HashMap<u8, ElevationGroupParams>>,
) -> Result<KlendComputedHealth, KaminoError> {
    let elev = pos
        .header
        .elevation_group
        .checked_mul(1)
        .filter(|&id| id != 0)
        .and_then(|id| elevation_groups.and_then(|m| m.get(&id).copied()));

    let mut missing = Vec::new();
    let mut deposited_usd = 0.0f64;
    let mut unhealthy = 0.0f64;
    let mut allowed = 0.0f64;

    for d in &pos.deposits {
        if d.deposited_amount == 0 {
            continue;
        }
        let Some(r) = reserves.get(&d.reserve) else {
            missing.push(d.reserve.to_base58());
            continue;
        };
        if r.market_price_sf == 0 {
            missing.push(format!("{}:zero_price", d.reserve.to_base58()));
            continue;
        }
        let liq_tokens = r.collateral_to_liquidity(d.deposited_amount);
        let usd = tokens_to_usd_f64(liq_tokens, r.mint_decimals, r.market_price_sf);
        deposited_usd += usd;
        let (thr, ltv) = if let Some(e) = elev {
            (e.liquidation_threshold_pct, e.ltv_pct)
        } else {
            (r.liquidation_threshold_pct, r.loan_to_value_pct)
        };
        unhealthy += usd * (thr as f64) / 100.0;
        allowed += usd * (ltv as f64) / 100.0;
    }

    let mut borrowed_usd = 0.0f64;
    let mut bf_debt = 0.0f64;
    for b in &pos.borrows {
        if b.borrowed_amount_sf == 0 {
            continue;
        }
        let Some(r) = reserves.get(&b.reserve) else {
            missing.push(b.reserve.to_base58());
            continue;
        };
        if r.market_price_sf == 0 {
            missing.push(format!("{}:zero_price", b.reserve.to_base58()));
            continue;
        }
        let tokens = sf_to_f64(b.borrowed_amount_sf);
        let usd = tokens_to_usd_f64(tokens, r.mint_decimals, r.market_price_sf);
        borrowed_usd += usd;
        // Elevation-mode debt typically uses BF=100 (matches on-chain value_bf == value).
        let bf = if elev.is_some() {
            100.0
        } else {
            r.borrow_factor_pct.max(1) as f64
        };
        bf_debt += usd * bf / 100.0;
    }

    // Protocol check: bf_debt > unhealthy. Dust notionals are never actionable.
    let meaningful = bf_debt >= MIN_LIQUIDATION_NOTIONAL_USD
        && deposited_usd >= MIN_LIQUIDATION_NOTIONAL_USD;
    let is_liquidatable = meaningful && bf_debt > unhealthy;
    let below_maintenance = meaningful && bf_debt > allowed;
    let health_factor = if bf_debt <= 0.0 {
        1000.0
    } else if unhealthy <= 0.0 {
        // No counted collateral (missing reserves / zero thr) — not CRITICAL.
        1000.0
    } else {
        unhealthy / bf_debt
    };
    let ltv = if deposited_usd <= 0.0 {
        0.0
    } else {
        bf_debt / deposited_usd
    };

    let to_micro = |v: f64| -> u128 {
        if !v.is_finite() || v <= 0.0 {
            0
        } else {
            (v * 1_000_000.0).round() as u128
        }
    };

    Ok(KlendComputedHealth {
        deposited_value_usd_micro: to_micro(deposited_usd),
        borrowed_value_usd_micro: to_micro(borrowed_usd),
        borrow_factor_adjusted_debt_usd_micro: to_micro(bf_debt),
        unhealthy_borrow_value_usd_micro: to_micro(unhealthy),
        allowed_borrow_value_usd_micro: to_micro(allowed),
        health_factor,
        ltv,
        is_liquidatable,
        below_maintenance,
        missing_reserves: missing,
    })
}

/// Build a PriceMap from reserve risk entries (mint → price).
pub fn price_map_from_reserves(reserves: &HashMap<Pubkey, LiveReserveRisk>) -> PriceMap {
    let mut prices = Vec::new();
    for r in reserves.values() {
        if r.market_price_sf > 0 {
            prices.push((r.liquidity_mint, r.price_fx()));
        }
    }
    PriceMap { prices }
}

/// Classify band from computed klend health (liq-threshold HF).
pub fn band_from_klend_health(h: &KlendComputedHealth) -> liq_core::CandidateBand {
    use liq_core::CandidateBand;
    if h.is_liquidatable {
        CandidateBand::Critical
    } else if h.health_factor < 1.05 {
        CandidateBand::Hot
    } else if h.health_factor < 1.20 {
        CandidateBand::Warm
    } else {
        CandidateBand::Cold
    }
}

/// Stale SF-header heuristic (DO NOT use for CRITICAL selection).
/// Kept for diagnostics / comparison only.
pub fn stale_sf_header_health_ratio(
    borrowed_assets_market_value_sf: u128,
    unhealthy_borrow_value_sf: u128,
) -> Option<f64> {
    if borrowed_assets_market_value_sf == 0 {
        return None;
    }
    Some(unhealthy_borrow_value_sf as f64 / borrowed_assets_market_value_sf as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{
        LiveBorrowPosition, LiveDepositPosition, LiveObligationHeader, LIVE_OBLIGATION_DATASIZE,
        LIVE_RESERVE_DATASIZE,
    };
    use liq_core::amount_to_usd_micro;

    fn pk(i: u64) -> Pubkey {
        Pubkey::test(42, i)
    }

    /// Fixture matching shadow logs: MSOL deposit ~$10.29, USDC borrow ~$3.48, LTV~0.34.
    #[test]
    fn healthy_msol_usdc_fixture_not_liquidatable() {
        let msol = pk(1);
        let usdc = pk(2);
        let dep_res = pk(3);
        let bor_res = pk(4);

        let mut reserves = HashMap::new();
        // MSOL @ 149.558, decimals 9, liq threshold 75%, LTV 65%
        reserves.insert(
            dep_res,
            LiveReserveRisk {
                address: dep_res,
                lending_market: pk(9),
                liquidity_mint: msol,
                mint_decimals: 9,
                loan_to_value_pct: 65,
                liquidation_threshold_pct: 75,
                borrow_factor_pct: 100,
                market_price_sf: f64_to_sf(149.558),
                total_available_amount: 1_000_000_000_000,
                borrowed_amount_sf: 0,
                accumulated_protocol_fees_sf: 0,
                accumulated_referrer_fees_sf: 0,
                pending_referrer_fees_sf: 0,
                collateral_mint_total_supply: 1_000_000_000_000, // 1:1 exchange
                pyth_oracle: Pubkey::default(),
                switchboard_price: Pubkey::default(),
                scope_prices: Pubkey::default(),
            },
        );
        reserves.insert(
            bor_res,
            LiveReserveRisk {
                address: bor_res,
                lending_market: pk(9),
                liquidity_mint: usdc,
                mint_decimals: 6,
                loan_to_value_pct: 0,
                liquidation_threshold_pct: 0,
                borrow_factor_pct: 100,
                market_price_sf: f64_to_sf(0.9999),
                total_available_amount: 1_000_000_000_000,
                borrowed_amount_sf: 0,
                accumulated_protocol_fees_sf: 0,
                accumulated_referrer_fees_sf: 0,
                pending_referrer_fees_sf: 0,
                collateral_mint_total_supply: 1_000_000_000_000,
                pyth_oracle: Pubkey::default(),
                switchboard_price: Pubkey::default(),
                scope_prices: Pubkey::default(),
            },
        );

        // ~0.068785363 MSOL → ~$10.287
        let deposited = 68_785_363u64;
        // ~3.477764 USDC
        let borrowed_sf = f64_to_sf(3_477_764.1358);

        let pos = LiveObligationPositions {
            header: LiveObligationHeader {
                address: pk(5),
                lending_market: pk(9),
                owner: pk(6),
                deposited_value_sf: 0,
                borrowed_assets_market_value_sf: 0,
                borrow_factor_adjusted_debt_value_sf: 0,
                allowed_borrow_value_sf: 0,
                unhealthy_borrow_value_sf: 0,
                has_debt: true,
                elevation_group: 0,
                referrer: Pubkey::default(),
            },
            deposits: vec![LiveDepositPosition {
                reserve: dep_res,
                deposited_amount: deposited,
                market_value_sf: 0,
            }],
            borrows: vec![LiveBorrowPosition {
                reserve: bor_res,
                borrowed_amount_sf: borrowed_sf,
                market_value_sf: 0,
            }],
        };

        let h = compute_obligation_health_live(&pos, &reserves).unwrap();
        assert!(
            !h.is_liquidatable,
            "expected healthy, got liquidatable HF={} LTV={}",
            h.health_factor,
            h.ltv
        );
        assert!(
            (h.ltv - 0.338).abs() < 0.02,
            "LTV expected ~0.338 got {}",
            h.ltv
        );
        assert!(h.health_factor > 2.0, "HF should be high, got {}", h.health_factor);
        // Stale SF heuristic would lie if unhealthy≈borrowed:
        let stale = stale_sf_header_health_ratio(1000, 998).unwrap();
        assert!(stale < 1.0);
        assert_ne!(
            (stale < 1.0),
            h.is_liquidatable,
            "stale SF must not drive liquidatable"
        );
    }

    #[test]
    fn unhealthy_when_borrow_exceeds_deposit_times_liq_ltv() {
        let coll = pk(1);
        let debt = pk(2);
        let dep_res = pk(3);
        let bor_res = pk(4);
        let mut reserves = HashMap::new();
        reserves.insert(
            dep_res,
            LiveReserveRisk {
                address: dep_res,
                lending_market: pk(9),
                liquidity_mint: coll,
                mint_decimals: 6,
                loan_to_value_pct: 50,
                liquidation_threshold_pct: 60, // 60%
                borrow_factor_pct: 100,
                market_price_sf: f64_to_sf(1.0),
                total_available_amount: 1_000_000_000,
                borrowed_amount_sf: 0,
                accumulated_protocol_fees_sf: 0,
                accumulated_referrer_fees_sf: 0,
                pending_referrer_fees_sf: 0,
                collateral_mint_total_supply: 1_000_000_000,
                pyth_oracle: Pubkey::default(),
                switchboard_price: Pubkey::default(),
                scope_prices: Pubkey::default(),
            },
        );
        reserves.insert(
            bor_res,
            LiveReserveRisk {
                address: bor_res,
                lending_market: pk(9),
                liquidity_mint: debt,
                mint_decimals: 6,
                loan_to_value_pct: 0,
                liquidation_threshold_pct: 0,
                borrow_factor_pct: 100,
                market_price_sf: f64_to_sf(1.0),
                total_available_amount: 1_000_000_000,
                borrowed_amount_sf: 0,
                accumulated_protocol_fees_sf: 0,
                accumulated_referrer_fees_sf: 0,
                pending_referrer_fees_sf: 0,
                collateral_mint_total_supply: 1_000_000_000,
                pyth_oracle: Pubkey::default(),
                switchboard_price: Pubkey::default(),
                scope_prices: Pubkey::default(),
            },
        );

        // $100 deposit → unhealthy = $60; borrow $70 → liquidatable
        let pos = LiveObligationPositions {
            header: LiveObligationHeader {
                address: pk(5),
                lending_market: pk(9),
                owner: pk(6),
                deposited_value_sf: 0,
                borrowed_assets_market_value_sf: 0,
                borrow_factor_adjusted_debt_value_sf: 0,
                allowed_borrow_value_sf: 0,
                unhealthy_borrow_value_sf: 0,
                has_debt: true,
                elevation_group: 0,
                referrer: Pubkey::default(),
            },
            deposits: vec![LiveDepositPosition {
                reserve: dep_res,
                deposited_amount: 100_000_000, // 100 tokens @ 6 dec
                market_value_sf: 0,
            }],
            borrows: vec![LiveBorrowPosition {
                reserve: bor_res,
                borrowed_amount_sf: f64_to_sf(70_000_000.0),
                market_value_sf: 0,
            }],
        };
        let h = compute_obligation_health_live(&pos, &reserves).unwrap();
        assert!(h.is_liquidatable);
        assert!(h.health_factor < 1.0);
        assert!(h.below_maintenance);
        // At threshold: borrow 60 → not liquidatable
        let mut pos2 = pos.clone();
        pos2.borrows[0].borrowed_amount_sf = f64_to_sf(60_000_000.0);
        let h2 = compute_obligation_health_live(&pos2, &reserves).unwrap();
        assert!(!h2.is_liquidatable);
        assert!((h2.health_factor - 1.0).abs() < 1e-6);
    }

    #[test]
    fn borrow_factor_scales_debt() {
        let coll = pk(1);
        let debt = pk(2);
        let dep_res = pk(3);
        let bor_res = pk(4);
        let mut reserves = HashMap::new();
        reserves.insert(
            dep_res,
            LiveReserveRisk {
                address: dep_res,
                lending_market: pk(9),
                liquidity_mint: coll,
                mint_decimals: 6,
                loan_to_value_pct: 80,
                liquidation_threshold_pct: 90,
                borrow_factor_pct: 100,
                market_price_sf: f64_to_sf(1.0),
                total_available_amount: 1_000_000_000,
                borrowed_amount_sf: 0,
                accumulated_protocol_fees_sf: 0,
                accumulated_referrer_fees_sf: 0,
                pending_referrer_fees_sf: 0,
                collateral_mint_total_supply: 1_000_000_000,
                pyth_oracle: Pubkey::default(),
                switchboard_price: Pubkey::default(),
                scope_prices: Pubkey::default(),
            },
        );
        reserves.insert(
            bor_res,
            LiveReserveRisk {
                address: bor_res,
                lending_market: pk(9),
                liquidity_mint: debt,
                mint_decimals: 6,
                loan_to_value_pct: 0,
                liquidation_threshold_pct: 0,
                borrow_factor_pct: 200, // 2x
                market_price_sf: f64_to_sf(1.0),
                total_available_amount: 1_000_000_000,
                borrowed_amount_sf: 0,
                accumulated_protocol_fees_sf: 0,
                accumulated_referrer_fees_sf: 0,
                pending_referrer_fees_sf: 0,
                collateral_mint_total_supply: 1_000_000_000,
                pyth_oracle: Pubkey::default(),
                switchboard_price: Pubkey::default(),
                scope_prices: Pubkey::default(),
            },
        );
        // $100 deposit → unhealthy $90; raw borrow $50 → BF debt $100 > $90 → liquidatable
        let pos = LiveObligationPositions {
            header: LiveObligationHeader {
                address: pk(5),
                lending_market: pk(9),
                owner: pk(6),
                deposited_value_sf: 0,
                borrowed_assets_market_value_sf: 0,
                borrow_factor_adjusted_debt_value_sf: 0,
                allowed_borrow_value_sf: 0,
                unhealthy_borrow_value_sf: 0,
                has_debt: true,
                elevation_group: 0,
                referrer: Pubkey::default(),
            },
            deposits: vec![LiveDepositPosition {
                reserve: dep_res,
                deposited_amount: 100_000_000,
                market_value_sf: 0,
            }],
            borrows: vec![LiveBorrowPosition {
                reserve: bor_res,
                borrowed_amount_sf: f64_to_sf(50_000_000.0),
                market_value_sf: 0,
            }],
        };
        let h = compute_obligation_health_live(&pos, &reserves).unwrap();
        assert!(h.is_liquidatable);
        assert!(h.borrow_factor_adjusted_debt_usd_micro > h.borrowed_value_usd_micro);
    }

    #[test]
    fn decode_reserve_risk_offsets_roundtrip() {
        let mut data = vec![0u8; LIVE_RESERVE_DATASIZE];
        let market = pk(1);
        let mint = pk(2);
        data[32..64].copy_from_slice(&market.0);
        let liq = LIVE_RESERVE_LIQUIDITY_OFFSET;
        data[liq..liq + 32].copy_from_slice(&mint.0);
        data[liq + 144] = 6;
        let price = f64_to_sf(100.0);
        data[RESERVE_MARKET_PRICE_SF_OFFSET..RESERVE_MARKET_PRICE_SF_OFFSET + 16]
            .copy_from_slice(&price.to_le_bytes());
        data[RESERVE_LTV_PCT_OFFSET] = 65;
        data[RESERVE_LIQ_THRESHOLD_PCT_OFFSET] = 75;
        data[RESERVE_BORROW_FACTOR_PCT_OFFSET..RESERVE_BORROW_FACTOR_PCT_OFFSET + 8]
            .copy_from_slice(&100u64.to_le_bytes());
        data[RESERVE_AVAILABLE_OFFSET..RESERVE_AVAILABLE_OFFSET + 8]
            .copy_from_slice(&1_000u64.to_le_bytes());
        data[RESERVE_COLLATERAL_MINT_SUPPLY_OFFSET..RESERVE_COLLATERAL_MINT_SUPPLY_OFFSET + 8]
            .copy_from_slice(&1_000u64.to_le_bytes());
        let r = decode_reserve_live_risk(pk(8), &data).unwrap();
        assert_eq!(r.loan_to_value_pct, 65);
        assert_eq!(r.liquidation_threshold_pct, 75);
        assert_eq!(r.borrow_factor_pct, 100);
        assert!((r.price_f64() - 100.0).abs() < 1e-6);
        assert_eq!(r.mint_decimals, 6);
        let _ = LIVE_OBLIGATION_DATASIZE; // silence
    }

    #[test]
    fn dust_obligation_not_marked_liquidatable() {
        let coll = pk(1);
        let debt = pk(2);
        let dep_res = pk(3);
        let bor_res = pk(4);
        let mut reserves = HashMap::new();
        for (res, mint, thr) in [(dep_res, coll, 90u8), (bor_res, debt, 0u8)] {
            reserves.insert(
                res,
                LiveReserveRisk {
                    address: res,
                    lending_market: pk(9),
                    liquidity_mint: mint,
                    mint_decimals: 6,
                    loan_to_value_pct: 80,
                    liquidation_threshold_pct: thr,
                    borrow_factor_pct: 100,
                    market_price_sf: f64_to_sf(1.0),
                    total_available_amount: 100_000_000_000_000,
                    borrowed_amount_sf: 0,
                    accumulated_protocol_fees_sf: 0,
                    accumulated_referrer_fees_sf: 0,
                    pending_referrer_fees_sf: 0,
                    collateral_mint_total_supply: 100_000_000_000_000,
                    pyth_oracle: Pubkey::default(),
                    switchboard_price: Pubkey::default(),
                    scope_prices: Pubkey::default(),
                },
            );
        }
        // 1 collateral share → ~1.0 base unit = $0.000001 — dust
        let pos = LiveObligationPositions {
            header: LiveObligationHeader {
                address: pk(5),
                lending_market: pk(9),
                owner: pk(6),
                deposited_value_sf: 0,
                borrowed_assets_market_value_sf: 0,
                borrow_factor_adjusted_debt_value_sf: 0,
                allowed_borrow_value_sf: 0,
                unhealthy_borrow_value_sf: 0,
                has_debt: true,
                elevation_group: 0,
                referrer: Pubkey::default(),
            },
            deposits: vec![LiveDepositPosition {
                reserve: dep_res,
                deposited_amount: 1,
                market_value_sf: 0,
            }],
            borrows: vec![LiveBorrowPosition {
                reserve: bor_res,
                borrowed_amount_sf: f64_to_sf(1.0),
                market_value_sf: 0,
            }],
        };
        let h = compute_obligation_health_live(&pos, &reserves).unwrap();
        assert!(!h.is_liquidatable, "dust must not be CRITICAL: {:?}", h);
    }


    #[test]
    fn elevation_group_overrides_reserve_threshold() {
        let coll = pk(1);
        let debt = pk(2);
        let dep_res = pk(3);
        let bor_res = pk(4);
        let mut reserves = HashMap::new();
        reserves.insert(
            dep_res,
            LiveReserveRisk {
                address: dep_res,
                lending_market: pk(9),
                liquidity_mint: coll,
                mint_decimals: 9,
                loan_to_value_pct: 45,
                liquidation_threshold_pct: 55, // base — would falsely liquidate at LTV 0.87
                borrow_factor_pct: 125,
                market_price_sf: f64_to_sf(100.0),
                total_available_amount: 1_000_000_000_000,
                borrowed_amount_sf: 0,
                accumulated_protocol_fees_sf: 0,
                accumulated_referrer_fees_sf: 0,
                pending_referrer_fees_sf: 0,
                collateral_mint_total_supply: 1_000_000_000_000,
                pyth_oracle: Pubkey::default(),
                switchboard_price: Pubkey::default(),
                scope_prices: Pubkey::default(),
            },
        );
        reserves.insert(
            bor_res,
            LiveReserveRisk {
                address: bor_res,
                lending_market: pk(9),
                liquidity_mint: debt,
                mint_decimals: 9,
                loan_to_value_pct: 0,
                liquidation_threshold_pct: 0,
                borrow_factor_pct: 125,
                market_price_sf: f64_to_sf(100.0),
                total_available_amount: 1_000_000_000_000,
                borrowed_amount_sf: 0,
                accumulated_protocol_fees_sf: 0,
                accumulated_referrer_fees_sf: 0,
                pending_referrer_fees_sf: 0,
                collateral_mint_total_supply: 1_000_000_000_000,
                pyth_oracle: Pubkey::default(),
                switchboard_price: Pubkey::default(),
                scope_prices: Pubkey::default(),
            },
        );
        let mut elev = HashMap::new();
        elev.insert(
            2,
            ElevationGroupParams {
                id: 2,
                ltv_pct: 87,
                liquidation_threshold_pct: 92,
                max_liquidation_bonus_bps: 500,
            },
        );
        // $100 deposit, $87 borrow → LTV 0.87 < 0.92 elev thr → healthy
        let mut pos = LiveObligationPositions {
            header: LiveObligationHeader {
                address: pk(5),
                lending_market: pk(9),
                owner: pk(6),
                deposited_value_sf: 0,
                borrowed_assets_market_value_sf: 0,
                borrow_factor_adjusted_debt_value_sf: 0,
                allowed_borrow_value_sf: 0,
                unhealthy_borrow_value_sf: 0,
                has_debt: true,
                elevation_group: 2,
                referrer: Pubkey::default(),
            },
            deposits: vec![LiveDepositPosition {
                reserve: dep_res,
                deposited_amount: 1_000_000_000, // 1 token @ 9 dec → $100
                market_value_sf: 0,
            }],
            borrows: vec![LiveBorrowPosition {
                reserve: bor_res,
                borrowed_amount_sf: f64_to_sf(870_000_000.0),
                market_value_sf: 0,
            }],
        };
        let h = compute_obligation_health_live_with_elevation(&pos, &reserves, Some(&elev)).unwrap();
        assert!(!h.is_liquidatable, "elev thr 92% should keep LTV 0.87 healthy: {:?}", h);
        assert!((h.ltv - 0.87).abs() < 0.01, "ltv={}", h.ltv);
        // Without elevation map → false CRITICAL via 55% thr
        pos.header.elevation_group = 0;
        let h2 = compute_obligation_health_live(&pos, &reserves).unwrap();
        assert!(h2.is_liquidatable);
    }

    #[test]
    fn amount_to_usd_micro_matches_tokens_helper() {
        let px = PriceFx::from_f64(149.558);
        let a = amount_to_usd_micro(68_785_363, 9, px);
        let b = tokens_to_usd_micro(68_785_363.0, 9, f64_to_sf(149.558));
        assert!((a as i128 - b as i128).abs() < 50, "a={a} b={b}");
    }
}
