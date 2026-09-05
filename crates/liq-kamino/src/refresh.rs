//! Refresh reserve / obligation ordering helpers.

use crate::{encode_refresh_obligation, encode_refresh_reserve};
use liq_core::Pubkey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshPlan {
    pub reserve_order: Vec<Pubkey>,
    pub obligation: Pubkey,
    pub datas: Vec<Vec<u8>>,
    pub labels: Vec<String>,
}

/// Build refresh ix data sequence: each unique reserve once, then obligation.
/// Order matches terminator/klend pattern: refresh reserves involved, then obligation.
pub fn build_refresh_plan(
    obligation: Pubkey,
    deposit_reserves: &[Pubkey],
    borrow_reserves: &[Pubkey],
) -> RefreshPlan {
    let mut seen = Vec::new();
    let mut reserve_order = Vec::new();
    for r in deposit_reserves.iter().chain(borrow_reserves.iter()) {
        if !seen.contains(r) {
            seen.push(*r);
            reserve_order.push(*r);
        }
    }
    let mut datas = Vec::new();
    let mut labels = Vec::new();
    for _ in &reserve_order {
        datas.push(encode_refresh_reserve());
        labels.push("refresh_reserve".into());
    }
    datas.push(encode_refresh_obligation());
    labels.push("refresh_obligation".into());
    RefreshPlan {
        reserve_order,
        obligation,
        datas,
        labels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc;

    #[test]
    fn dedups_reserves_then_obligation() {
        let a = Pubkey::test(1, 1);
        let b = Pubkey::test(1, 2);
        let obl = Pubkey::test(2, 1);
        let plan = build_refresh_plan(obl, &[a, b], &[b]);
        assert_eq!(plan.reserve_order, vec![a, b]);
        assert_eq!(plan.datas.len(), 3);
        assert_eq!(&plan.datas[0][..8], &disc::REFRESH_RESERVE);
        assert_eq!(&plan.datas[2][..8], &disc::REFRESH_OBLIGATION);
    }
}
