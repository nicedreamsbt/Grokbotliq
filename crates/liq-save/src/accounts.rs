//! Account metas for LiquidateObligation (tag 12) from solend-sdk.

use liq_core::Pubkey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidateObligationAccounts {
    pub source_liquidity: Pubkey,
    pub destination_collateral: Pubkey,
    pub repay_reserve: Pubkey,
    pub repay_reserve_liquidity_supply: Pubkey,
    pub withdraw_reserve: Pubkey,
    pub withdraw_reserve_collateral_supply: Pubkey,
    pub obligation: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub user_transfer_authority: Pubkey,
    pub clock_sysvar: Pubkey,
    pub token_program: Pubkey,
}

impl LiquidateObligationAccounts {
    pub fn names() -> &'static [&'static str] {
        &[
            "source_liquidity",
            "destination_collateral",
            "repay_reserve",
            "repay_reserve_liquidity_supply",
            "withdraw_reserve",
            "withdraw_reserve_collateral_supply",
            "obligation",
            "lending_market",
            "lending_market_authority",
            "user_transfer_authority",
            "clock",
            "token_program",
        ]
    }

    pub fn keys(&self) -> [Pubkey; 12] {
        [
            self.source_liquidity,
            self.destination_collateral,
            self.repay_reserve,
            self.repay_reserve_liquidity_supply,
            self.withdraw_reserve,
            self.withdraw_reserve_collateral_supply,
            self.obligation,
            self.lending_market,
            self.lending_market_authority,
            self.user_transfer_authority,
            self.clock_sysvar,
            self.token_program,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_accounts() {
        assert_eq!(LiquidateObligationAccounts::names().len(), 12);
    }
}
