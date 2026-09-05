//! RefreshReserve → RefreshObligation → Liquidate ordering.

use crate::{encode_liquidate_and_redeem, encode_refresh_obligation, encode_refresh_reserve};
use liq_core::Pubkey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveLiquidationPlan {
    pub labels: Vec<String>,
    pub datas: Vec<Vec<u8>>,
    pub reserves_refreshed: Vec<Pubkey>,
    pub obligation: Pubkey,
    pub liquidity_amount: u64,
}

/// Build the canonical Save liquidation ix data sequence.
pub fn build_liquidation_plan(
    obligation: Pubkey,
    deposit_reserves: &[Pubkey],
    borrow_reserves: &[Pubkey],
    liquidity_amount: u64,
) -> SaveLiquidationPlan {
    let mut seen = Vec::new();
    let mut reserves = Vec::new();
    for r in deposit_reserves.iter().chain(borrow_reserves.iter()) {
        if !seen.contains(r) {
            seen.push(*r);
            reserves.push(*r);
        }
    }
    let mut labels = Vec::new();
    let mut datas = Vec::new();
    for _ in &reserves {
        labels.push("RefreshReserve".into());
        datas.push(encode_refresh_reserve());
    }
    labels.push("RefreshObligation".into());
    datas.push(encode_refresh_obligation());
    labels.push("LiquidateObligationAndRedeemReserveCollateral".into());
    datas.push(encode_liquidate_and_redeem(liquidity_amount));
    SaveLiquidationPlan {
        labels,
        datas,
        reserves_refreshed: reserves,
        obligation,
        liquidity_amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SaveIx;

    #[test]
    fn order_refresh_then_liquidate() {
        let obl = Pubkey::test(1, 1);
        let r1 = Pubkey::test(2, 1);
        let r2 = Pubkey::test(2, 2);
        let plan = build_liquidation_plan(obl, &[r1], &[r2], 42);
        assert_eq!(plan.labels, [
            "RefreshReserve",
            "RefreshReserve",
            "RefreshObligation",
            "LiquidateObligationAndRedeemReserveCollateral"
        ]);
        assert_eq!(plan.datas[0][0], SaveIx::RefreshReserve as u8);
        assert_eq!(plan.datas[2][0], SaveIx::RefreshObligation as u8);
        assert_eq!(plan.datas[3][0], SaveIx::LiquidateObligationAndRedeemReserveCollateral as u8);
    }
}
