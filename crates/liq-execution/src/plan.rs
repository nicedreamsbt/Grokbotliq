//! Strategy → protocol tx builders. Accounts come from structured inputs
//! (fixtures/config), never hardcoded inside flash builders.

use liq_core::{
    programs, FundingStrategy, LabeledIx, Protocol, Pubkey,
};
use liq_kamino::{
    build_flash_tx, build_inventory_tx, EnsureAta, FlashBorrowAccounts, FlashRepayAccounts,
    KaminoTxBuildParams, LiquidateV2Accounts, RefreshReserveAccounts,
};
use liq_routing::JupiterQuoteBlob;
use liq_save::{
    build_flash_atomic_plan, FlashBorrowAccounts as SaveFlashBorrow,
    FlashRepayAccounts as SaveFlashRepay, SaveFlashPlanAccounts, SaveLiquidateAccounts,
};

/// Structured account set for building liquidation txs in shadow/fixture mode.
/// Real mainnet keys should be loaded from config/RPC decode; fixture path uses
/// deterministic `Pubkey::test` **outside** the builders.
#[derive(Debug, Clone)]
pub struct PlanAccountSet {
    pub liquidator: Pubkey,
    pub obligation: Pubkey,
    pub lending_market: Pubkey,
    pub lending_market_authority: Pubkey,
    pub repay_reserve: Pubkey,
    pub withdraw_reserve: Pubkey,
    pub user_liquidity: Pubkey,
    pub user_collateral: Pubkey,
    /// When set (live decode), used instead of synthetic mint/supply placeholders.
    pub repay_liquidity_mint: Option<Pubkey>,
    pub repay_liquidity_supply: Option<Pubkey>,
    pub repay_fee_receiver: Option<Pubkey>,
    pub withdraw_liquidity_mint: Option<Pubkey>,
    pub withdraw_collateral_mint: Option<Pubkey>,
    pub withdraw_collateral_supply: Option<Pubkey>,
    pub withdraw_liquidity_supply: Option<Pubkey>,
    pub withdraw_fee_receiver: Option<Pubkey>,
    pub repay_token_program: Option<Pubkey>,
    pub withdraw_token_program: Option<Pubkey>,
    /// Extra deposit reserves to refresh (from live obligation deposits).
    pub deposit_reserves_extra: Vec<Pubkey>,
    pub borrow_reserves_extra: Vec<Pubkey>,
    /// True when core keys came from live Klend decode.
    pub from_live_decode: bool,
    /// Live refresh_reserve metas (market + oracles) when decoded.
    pub refresh_reserve_metas: Vec<RefreshReserveAccounts>,
    /// Optional destination liquidity ATA (withdraw liquidity mint) — defaults to user_liquidity.
    pub user_destination_liquidity: Option<Pubkey>,
    /// Farm accounts from reserves (None → KLend program id placeholder in liquidate_v2).
    pub collateral_obligation_farm_user_state: Option<Pubkey>,
    pub collateral_reserve_farm_state: Option<Pubkey>,
    pub debt_obligation_farm_user_state: Option<Pubkey>,
    pub debt_reserve_farm_state: Option<Pubkey>,
    /// Obligation referrer wallet when non-default.
    pub referrer: Option<Pubkey>,
    /// ReferrerTokenState PDAs aligned with borrow_reserves_extra (when referrer set).
    pub referrer_token_states: Vec<Pubkey>,
    /// When `Some`, only emit CreateIdempotent for these ATA addresses (missing on-chain).
    /// When `None`, emit CreateIdempotent for all liquidator ATAs (idempotent).
    pub missing_ata_filter: Option<Vec<Pubkey>>,
}

impl PlanAccountSet {
    /// Fixture-shaped accounts derived from a borrower pubkey (tag/index stable).
    pub fn from_seed(seed: Pubkey) -> Self {
        let tag = seed.0[0];
        let base = u64::from_le_bytes(seed.0[24..32].try_into().unwrap_or([0; 8]));
        let pk = |i| Pubkey::test(tag.wrapping_add(20), base.wrapping_add(i));
        Self {
            liquidator: pk(1),
            obligation: seed,
            lending_market: pk(3),
            lending_market_authority: pk(4),
            repay_reserve: pk(5),
            withdraw_reserve: pk(6),
            user_liquidity: pk(7),
            user_collateral: pk(8),
            repay_liquidity_mint: None,
            repay_liquidity_supply: None,
            repay_fee_receiver: None,
            withdraw_liquidity_mint: None,
            withdraw_collateral_mint: None,
            withdraw_collateral_supply: None,
            withdraw_liquidity_supply: None,
            withdraw_fee_receiver: None,
            repay_token_program: None,
            withdraw_token_program: None,
            deposit_reserves_extra: vec![],
            borrow_reserves_extra: vec![],
            from_live_decode: false,
            refresh_reserve_metas: vec![],
            user_destination_liquidity: None,
            collateral_obligation_farm_user_state: None,
            collateral_reserve_farm_state: None,
            debt_obligation_farm_user_state: None,
            debt_reserve_farm_state: None,
            referrer: None,
            referrer_token_states: vec![],
            missing_ata_filter: None,
        }
    }

    /// Build from live Klend obligation positions + reserve vault metas.
    pub fn from_kamino_live(
        obligation: Pubkey,
        liquidator: Pubkey,
        positions: &liq_kamino::LiveObligationPositions,
        repay: &liq_kamino::LiveReserveVaults,
        withdraw: &liq_kamino::LiveReserveVaults,
        market_authority: Pubkey,
    ) -> Self {
        let deposit_reserves_extra: Vec<_> = positions.deposits.iter().map(|d| d.reserve).collect();
        let borrow_reserves_extra: Vec<_> = positions.borrows.iter().map(|b| b.reserve).collect();
        // Real ATAs for fee payer / liquidator (derivation only — no private key).
        let repay_tp = if repay.token_program.0 == [0u8; 32] {
            programs::token()
        } else {
            repay.token_program
        };
        let withdraw_tp = if withdraw.token_program.0 == [0u8; 32] {
            programs::token()
        } else {
            withdraw.token_program
        };
        let user_liquidity = liq_core::get_associated_token_address(
            &liquidator,
            &repay.liquidity_mint,
            &repay_tp,
        );
        let user_collateral = liq_core::get_associated_token_address(
            &liquidator,
            &withdraw.collateral_mint,
            &withdraw_tp,
        );
        let user_dest_liq = liq_core::get_associated_token_address(
            &liquidator,
            &withdraw.liquidity_mint,
            &withdraw_tp,
        );
        let nonzero = |pk: Pubkey| (pk.0 != [0u8; 32]).then_some(pk);
        let referrer = nonzero(positions.header.referrer);
        let referrer_token_states = if let Some(ref_pk) = referrer {
            borrow_reserves_extra
                .iter()
                .map(|r| liq_core::klend_referrer_token_state(&ref_pk, r))
                .collect()
        } else {
            vec![]
        };
        // Farm reserve states from live reserve decode; obligation farm user
        // states left None → KLend program id readonly placeholder (sdk pattern).
        Self {
            liquidator,
            obligation,
            lending_market: positions.header.lending_market,
            lending_market_authority: market_authority,
            repay_reserve: repay.address,
            withdraw_reserve: withdraw.address,
            user_liquidity,
            user_collateral,
            repay_liquidity_mint: Some(repay.liquidity_mint),
            repay_liquidity_supply: Some(repay.liquidity_supply),
            repay_fee_receiver: Some(repay.fee_vault),
            withdraw_liquidity_mint: Some(withdraw.liquidity_mint),
            withdraw_collateral_mint: Some(withdraw.collateral_mint),
            withdraw_collateral_supply: Some(withdraw.collateral_supply),
            withdraw_liquidity_supply: Some(withdraw.liquidity_supply),
            withdraw_fee_receiver: Some(withdraw.fee_vault),
            repay_token_program: Some(repay_tp),
            withdraw_token_program: Some(withdraw_tp),
            deposit_reserves_extra,
            borrow_reserves_extra,
            from_live_decode: true,
            refresh_reserve_metas: {
                // Deposit then borrow order for stable refresh_ix sequencing.
                let mut v = Vec::new();
                let mut seen = Vec::new();
                for r in [withdraw, repay] {
                    if seen.contains(&r.address) {
                        continue;
                    }
                    seen.push(r.address);
                    v.push(RefreshReserveAccounts {
                        reserve: r.address,
                        lending_market: r.lending_market,
                        pyth_oracle: r.pyth_oracle,
                        switchboard_price: r.switchboard_price,
                        switchboard_twap: r.switchboard_twap,
                        scope_prices: r.scope_prices,
                    });
                }
                v
            },
            user_destination_liquidity: Some(user_dest_liq),
            // Unused farms: both sides None → KLend program id readonly (sdk codegen).
            // Do not pass reserve farm_state alone without obligation farm user state.
            collateral_obligation_farm_user_state: None,
            collateral_reserve_farm_state: None,
            debt_obligation_farm_user_state: None,
            debt_reserve_farm_state: None,
            referrer,
            referrer_token_states,
            missing_ata_filter: None,
        }
    }

    /// Build CreateIdempotent specs for liquidator repay/collateral/dest ATAs.
    /// Uses mint + token_program from live decode when present; Tokenkeg otherwise.
    /// Honors `missing_ata_filter` when set (shadow prefetch of absent accounts).
    pub fn liquidator_ensure_atas(&self) -> Vec<EnsureAta> {
        let repay_tp = self.repay_token_program.unwrap_or_else(programs::token);
        let withdraw_tp = self.withdraw_token_program.unwrap_or_else(programs::token);
        let repay_mint = self
            .repay_liquidity_mint
            .unwrap_or_else(|| Pubkey::test(self.repay_reserve.0[0], 100));
        let coll_mint = self.withdraw_collateral_mint.unwrap_or_else(|| {
            Pubkey::test(self.withdraw_reserve.0[0], 103)
        });
        let withdraw_liq_mint = self.withdraw_liquidity_mint.unwrap_or_else(|| {
            Pubkey::test(self.withdraw_reserve.0[0], 102)
        });
        let specs = [
            EnsureAta {
                payer: self.liquidator,
                owner: self.liquidator,
                mint: repay_mint,
                token_program: repay_tp,
            },
            EnsureAta {
                payer: self.liquidator,
                owner: self.liquidator,
                mint: coll_mint,
                token_program: withdraw_tp,
            },
            EnsureAta {
                payer: self.liquidator,
                owner: self.liquidator,
                mint: withdraw_liq_mint,
                token_program: withdraw_tp,
            },
        ];
        let mut out = Vec::new();
        let mut seen = Vec::new();
        for spec in specs {
            let ata = spec.ata_address();
            if seen.contains(&ata) {
                continue;
            }
            if let Some(filter) = &self.missing_ata_filter {
                if !filter.iter().any(|k| *k == ata) {
                    continue;
                }
            }
            seen.push(ata);
            out.push(spec);
        }
        out
    }
}

fn kamino_liquidate_accounts(a: &PlanAccountSet) -> LiquidateV2Accounts {
    let synth = |tag: u64| Pubkey::test(a.repay_reserve.0[0], tag);
    LiquidateV2Accounts {
        liquidator: a.liquidator,
        obligation: a.obligation,
        lending_market: a.lending_market,
        lending_market_authority: a.lending_market_authority,
        repay_reserve: a.repay_reserve,
        repay_reserve_liquidity_mint: a.repay_liquidity_mint.unwrap_or_else(|| synth(100)),
        repay_reserve_liquidity_supply: a.repay_liquidity_supply.unwrap_or_else(|| synth(101)),
        withdraw_reserve: a.withdraw_reserve,
        withdraw_reserve_liquidity_mint: a
            .withdraw_liquidity_mint
            .unwrap_or_else(|| Pubkey::test(a.withdraw_reserve.0[0], 102)),
        withdraw_reserve_collateral_mint: a
            .withdraw_collateral_mint
            .unwrap_or_else(|| Pubkey::test(a.withdraw_reserve.0[0], 103)),
        withdraw_reserve_collateral_supply: a
            .withdraw_collateral_supply
            .unwrap_or_else(|| Pubkey::test(a.withdraw_reserve.0[0], 104)),
        withdraw_reserve_liquidity_supply: a
            .withdraw_liquidity_supply
            .unwrap_or_else(|| Pubkey::test(a.withdraw_reserve.0[0], 105)),
        withdraw_reserve_liquidity_fee_receiver: a
            .withdraw_fee_receiver
            .unwrap_or_else(|| Pubkey::test(a.withdraw_reserve.0[0], 106)),
        user_source_liquidity: a.user_liquidity,
        user_destination_collateral: a.user_collateral,
        user_destination_liquidity: a
            .user_destination_liquidity
            .unwrap_or(a.user_liquidity),
        collateral_token_program: a.withdraw_token_program.unwrap_or_else(programs::token),
        repay_liquidity_token_program: a.repay_token_program.unwrap_or_else(programs::token),
        withdraw_liquidity_token_program: a.withdraw_token_program.unwrap_or_else(programs::token),
        instruction_sysvar_account: programs::sysvar_instructions(),
        collateral_obligation_farm_user_state: a.collateral_obligation_farm_user_state,
        collateral_reserve_farm_state: a.collateral_reserve_farm_state,
        debt_obligation_farm_user_state: a.debt_obligation_farm_user_state,
        debt_reserve_farm_state: a.debt_reserve_farm_state,
        farms_program: programs::kfarms(),
        deposit_reserves: a.deposit_reserves_extra.clone(),
    }
}

fn kamino_flash_pair(a: &PlanAccountSet) -> (FlashBorrowAccounts, FlashRepayAccounts) {
    let mint = a
        .repay_liquidity_mint
        .unwrap_or_else(|| Pubkey::test(a.repay_reserve.0[0], 100));
    let supply = a
        .repay_liquidity_supply
        .unwrap_or_else(|| Pubkey::test(a.repay_reserve.0[0], 101));
    let fee = a
        .repay_fee_receiver
        .or(a.withdraw_fee_receiver)
        .unwrap_or_else(|| Pubkey::test(a.withdraw_reserve.0[0], 106));
    let borrow = FlashBorrowAccounts {
        user_transfer_authority: a.liquidator,
        lending_market_authority: a.lending_market_authority,
        lending_market: a.lending_market,
        reserve: a.repay_reserve,
        reserve_liquidity_mint: mint,
        reserve_source_liquidity: supply,
        user_destination_liquidity: a.user_liquidity,
        reserve_liquidity_fee_receiver: fee,
        referrer_token_state: None, // → KLend program id readonly
        referrer_account: None,
    };
    let repay = FlashRepayAccounts {
        user_transfer_authority: a.liquidator,
        lending_market_authority: a.lending_market_authority,
        lending_market: a.lending_market,
        reserve: a.repay_reserve,
        reserve_liquidity_mint: mint,
        reserve_destination_liquidity: supply,
        user_source_liquidity: a.user_liquidity,
        reserve_liquidity_fee_receiver: fee,
        referrer_token_state: None,
        referrer_account: None,
    };
    (borrow, repay)
}

fn save_flash_accounts(a: &PlanAccountSet) -> SaveFlashPlanAccounts {
    SaveFlashPlanAccounts {
        flash_borrow: SaveFlashBorrow {
            source_liquidity: Pubkey::test(a.repay_reserve.0[0], 101),
            destination_liquidity: a.user_liquidity,
            reserve: a.repay_reserve,
            lending_market: a.lending_market,
            lending_market_authority: a.lending_market_authority,
        },
        liquidate: SaveLiquidateAccounts {
            source_liquidity: a.user_liquidity,
            destination_collateral: a.user_collateral,
            destination_liquidity: a.user_liquidity,
            repay_reserve: a.repay_reserve,
            repay_reserve_liquidity_supply: Pubkey::test(a.repay_reserve.0[0], 101),
            withdraw_reserve: a.withdraw_reserve,
            withdraw_reserve_collateral_mint: Pubkey::test(a.withdraw_reserve.0[0], 103),
            withdraw_reserve_collateral_supply: Pubkey::test(a.withdraw_reserve.0[0], 104),
            withdraw_reserve_liquidity_supply: Pubkey::test(a.withdraw_reserve.0[0], 105),
            withdraw_reserve_fee_receiver: Pubkey::test(a.withdraw_reserve.0[0], 106),
            obligation: a.obligation,
            lending_market: a.lending_market,
            lending_market_authority: a.lending_market_authority,
            user_transfer_authority: a.liquidator,
        },
        flash_repay: SaveFlashRepay {
            source_liquidity: a.user_liquidity,
            destination_liquidity: Pubkey::test(a.repay_reserve.0[0], 101),
            fee_receiver: Pubkey::test(a.withdraw_reserve.0[0], 106),
            host_fee_receiver: Pubkey::test(a.withdraw_reserve.0[0], 107),
            reserve: a.repay_reserve,
            lending_market: a.lending_market,
            user_transfer_authority: a.liquidator,
        },
        refresh_reserves: vec![a.repay_reserve, a.withdraw_reserve],
        obligation: a.obligation,
    }
}

/// Result of planning wire ixs for a funding strategy.
#[derive(Debug, Clone)]
pub struct PlannedIxs {
    pub labeled: Vec<LabeledIx>,
    pub swap_incomplete: bool,
    pub used_flash_builder: bool,
}

/// Build protocol-exact instruction lists for the chosen funding strategy.
pub fn build_strategy_ixs(
    protocol: Protocol,
    strategy: FundingStrategy,
    accounts: &PlanAccountSet,
    liquidity_amount: u64,
    swap_blob: &JupiterQuoteBlob,
) -> PlannedIxs {
    let (swaps, swap_incomplete) = swap_blob.attach_or_omit();
    match (protocol, strategy) {
        (Protocol::Kamino, FundingStrategy::KaminoFlashBorrow) => {
            let (borrow, repay) = kamino_flash_pair(accounts);
            // Live decode: exact active deposit/borrow lists (count-sensitive for
            // refresh_obligation Custom 6006). Fixture path may pad with repay/withdraw.
            let (deposit_reserves, borrow_reserves) = if accounts.from_live_decode {
                (
                    accounts.deposit_reserves_extra.clone(),
                    accounts.borrow_reserves_extra.clone(),
                )
            } else {
                let mut deposit_reserves = if accounts.deposit_reserves_extra.is_empty() {
                    vec![accounts.withdraw_reserve]
                } else {
                    accounts.deposit_reserves_extra.clone()
                };
                let mut borrow_reserves = if accounts.borrow_reserves_extra.is_empty() {
                    vec![accounts.repay_reserve]
                } else {
                    accounts.borrow_reserves_extra.clone()
                };
                if !deposit_reserves.contains(&accounts.withdraw_reserve) {
                    deposit_reserves.push(accounts.withdraw_reserve);
                }
                if !borrow_reserves.contains(&accounts.repay_reserve) {
                    borrow_reserves.push(accounts.repay_reserve);
                }
                (deposit_reserves, borrow_reserves)
            };
            let params = KaminoTxBuildParams {
                obligation: accounts.obligation,
                deposit_reserves,
                borrow_reserves,
                liquidate: kamino_liquidate_accounts(accounts),
                liquidity_amount,
                min_acceptable_received: 0,
                max_allowed_ltv_override_percent: 0,
                cu_limit: 400_000,
                cu_price: 1_000,
                flash: Some((borrow, repay)),
                refresh_reserves: accounts.refresh_reserve_metas.clone(),
                referrer_token_states: accounts.referrer_token_states.clone(),
                ensure_atas: accounts.liquidator_ensure_atas(),
            };
            let labeled = build_flash_tx(&params, &swaps).unwrap_or_default();
            PlannedIxs {
                labeled,
                swap_incomplete,
                used_flash_builder: true,
            }
        }
        (Protocol::Kamino, FundingStrategy::Inventory) => {
            let deposit_reserves = if accounts.from_live_decode {
                accounts.deposit_reserves_extra.clone()
            } else if accounts.deposit_reserves_extra.is_empty() {
                vec![accounts.withdraw_reserve]
            } else {
                accounts.deposit_reserves_extra.clone()
            };
            let borrow_reserves = if accounts.from_live_decode {
                accounts.borrow_reserves_extra.clone()
            } else if accounts.borrow_reserves_extra.is_empty() {
                vec![accounts.repay_reserve]
            } else {
                accounts.borrow_reserves_extra.clone()
            };
            let params = KaminoTxBuildParams {
                obligation: accounts.obligation,
                deposit_reserves,
                borrow_reserves,
                liquidate: kamino_liquidate_accounts(accounts),
                liquidity_amount,
                min_acceptable_received: 0,
                max_allowed_ltv_override_percent: 0,
                cu_limit: 400_000,
                cu_price: 1_000,
                flash: None,
                refresh_reserves: accounts.refresh_reserve_metas.clone(),
                referrer_token_states: accounts.referrer_token_states.clone(),
                ensure_atas: accounts.liquidator_ensure_atas(),
            };
            PlannedIxs {
                labeled: build_inventory_tx(&params, &swaps),
                swap_incomplete,
                used_flash_builder: false,
            }
        }
        (Protocol::Save, FundingStrategy::SaveFlashLoan) => {
            let plan = build_flash_atomic_plan(
                &save_flash_accounts(accounts),
                liquidity_amount,
                &swaps,
                400_000,
                1_000,
            );
            PlannedIxs {
                labeled: plan.labeled,
                swap_incomplete,
                used_flash_builder: true,
            }
        }
        (Protocol::Project0, FundingStrategy::Project0Receivership) => {
            use liq_project0::*;
            let a = accounts;
            let params = ReceivershipBuildParams {
                start: StartLiquidationAccounts {
                    marginfi_account: a.obligation,
                    liquidation_record: Pubkey::test(a.obligation.0[0], 200),
                    group: a.lending_market,
                    liquidation_receiver: a.liquidator,
                    instruction_sysvar: programs::sysvar_instructions(),
                    remaining_writable: vec![a.repay_reserve],
                },
                withdraw: WithdrawAccounts {
                    group: a.lending_market,
                    marginfi_account: a.liquidator,
                    authority: a.liquidator,
                    bank: a.withdraw_reserve,
                    vault: Pubkey::test(a.withdraw_reserve.0[0], 201),
                    destination: a.user_liquidity,
                    bank_liquidity_vault_authority: a.lending_market_authority,
                    token_program: programs::token(),
                },
                repay: RepayAccounts {
                    group: a.lending_market,
                    marginfi_account: a.liquidator,
                    authority: a.liquidator,
                    bank: a.repay_reserve,
                    signer_token_account: a.user_liquidity,
                    vault: Pubkey::test(a.repay_reserve.0[0], 202),
                    token_program: programs::token(),
                },
                end: EndLiquidationAccounts {
                    marginfi_account: a.obligation,
                    liquidation_record: Pubkey::test(a.obligation.0[0], 200),
                    group: a.lending_market,
                    liquidation_receiver: a.liquidator,
                    fee_state: Pubkey::test(80, 1),
                    global_fee_wallet: Pubkey::test(80, 2),
                    system_program: programs::system(),
                    fee_payer: None,
                },
                withdraw_amount: liquidity_amount,
                repay_amount: liquidity_amount.saturating_mul(9) / 10,
                cu_limit: 500_000,
                cu_price: 1000,
            };
            PlannedIxs {
                labeled: build_receivership_tx(&params, &swaps),
                swap_incomplete,
                used_flash_builder: false,
            }
        }
        _ => PlannedIxs {
            labeled: vec![LabeledIx {
                label: "ComputeBudget:SetComputeUnitLimit".into(),
                ix: liq_core::compute_unit_limit(200_000),
            }],
            swap_incomplete,
            used_flash_builder: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_selects_kamino_flash_builder() {
        let accounts = PlanAccountSet::from_seed(Pubkey::test(1, 1));
        let planned = build_strategy_ixs(
            Protocol::Kamino,
            FundingStrategy::KaminoFlashBorrow,
            &accounts,
            1_000_000,
            &JupiterQuoteBlob::default(),
        );
        assert!(planned.used_flash_builder);
        assert!(planned.swap_incomplete);
        let labels: Vec<_> = planned.labeled.iter().map(|l| l.label.as_str()).collect();
        assert!(labels.iter().any(|l| l.contains("flash_borrow")));
        assert!(labels.iter().any(|l| l.contains("flash_repay")));
        let borrow = planned
            .labeled
            .iter()
            .find(|l| l.label.contains("flash_borrow"))
            .unwrap();
        // absent referrer → KLend program id readonly
        assert_eq!(borrow.ix.accounts[8].pubkey, programs::klend());
        assert!(!borrow.ix.accounts[8].is_writable);
    }

    #[test]
    fn planner_kamino_inventory_skips_flash() {
        let accounts = PlanAccountSet::from_seed(Pubkey::test(1, 2));
        let planned = build_strategy_ixs(
            Protocol::Kamino,
            FundingStrategy::Inventory,
            &accounts,
            1_000,
            &JupiterQuoteBlob::default(),
        );
        assert!(!planned.used_flash_builder);
        assert!(!planned
            .labeled
            .iter()
            .any(|l| l.label.contains("flash_borrow")));
    }

    #[test]
    fn ata_derivation_for_liquidator_is_deterministic() {
        let owner = Pubkey::from_base58("5pHk2TmnqQzRF9L6egy5FfiyBgS7G9cMZ5RFaJAvghzw").unwrap();
        let mint = Pubkey::from_base58("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let a = liq_core::get_associated_token_address_token(&owner, &mint);
        let b = liq_core::get_associated_token_address(&owner, &mint, &programs::token());
        assert_eq!(a, b);
        assert_ne!(a, owner);
    }

    #[test]
    fn farm_placeholder_metas_use_klend_not_farms_program() {
        let accounts = PlanAccountSet::from_seed(Pubkey::test(9, 9));
        let liq = super::kamino_liquidate_accounts(&accounts);
        let metas = liq.metas();
        for name in [
            "collateral_obligation_farm_user_state",
            "collateral_reserve_farm_state",
            "debt_obligation_farm_user_state",
            "debt_reserve_farm_state",
        ] {
            let m = metas.iter().find(|x| x.name == name).unwrap();
            assert_eq!(m.key, programs::klend());
            assert_ne!(m.key, programs::kfarms());
        }
        let fp = metas.iter().find(|x| x.name == "farms_program").unwrap();
        assert_eq!(fp.key, programs::kfarms());
    }

    #[test]
    fn planner_save_flash_uses_builder() {
        let accounts = PlanAccountSet::from_seed(Pubkey::test(7, 1));
        let planned = build_strategy_ixs(
            Protocol::Save,
            FundingStrategy::SaveFlashLoan,
            &accounts,
            1_000_000,
            &JupiterQuoteBlob::default(),
        );
        assert!(planned.used_flash_builder);
        assert!(planned
            .labeled
            .iter()
            .any(|l| l.label == "FlashBorrowReserveLiquidity"));
    }
    #[test]
    fn planner_emits_create_idempotent_before_flash_and_liquidate() {
        let mut accounts = PlanAccountSet::from_seed(Pubkey::test(1, 1));
        // Force known mints so CreateIdempotent is emitted (always when filter is None).
        accounts.repay_liquidity_mint = Some(Pubkey::test(1, 100));
        accounts.withdraw_collateral_mint = Some(Pubkey::test(1, 103));
        accounts.withdraw_liquidity_mint = Some(Pubkey::test(1, 102));
        accounts.repay_token_program = Some(programs::token());
        accounts.withdraw_token_program = Some(programs::token());
        let planned = build_strategy_ixs(
            Protocol::Kamino,
            FundingStrategy::KaminoFlashBorrow,
            &accounts,
            1_000_000,
            &JupiterQuoteBlob::default(),
        );
        let labels: Vec<_> = planned.labeled.iter().map(|l| l.label.as_str()).collect();
        let creates: Vec<_> = labels
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains("CreateIdempotent"))
            .map(|(i, _)| i)
            .collect();
        assert!(!creates.is_empty(), "expected CreateIdempotent ixs: {labels:?}");
        let borrow = labels
            .iter()
            .position(|l| *l == "flash_borrow_reserve_liquidity")
            .unwrap();
        let liq = labels
            .iter()
            .position(|l| l.contains("liquidate"))
            .unwrap();
        assert!(creates.iter().all(|c| *c < borrow));
        assert!(borrow < liq);
        // Encoding spot-check
        let cix = &planned.labeled[creates[0]].ix;
        assert_eq!(cix.program_id, programs::associated_token());
        assert_eq!(cix.data, vec![1u8]);
        assert_eq!(cix.accounts.len(), 6);
    }

    #[test]
    fn missing_ata_filter_limits_create_idempotent() {
        let mut accounts = PlanAccountSet::from_seed(Pubkey::test(2, 2));
        accounts.repay_liquidity_mint = Some(Pubkey::test(2, 100));
        accounts.withdraw_collateral_mint = Some(Pubkey::test(2, 103));
        accounts.withdraw_liquidity_mint = Some(Pubkey::test(2, 102));
        let all = accounts.liquidator_ensure_atas();
        assert_eq!(all.len(), 3);
        let only = all[1].ata_address();
        accounts.missing_ata_filter = Some(vec![only]);
        let filtered = accounts.liquidator_ensure_atas();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].ata_address(), only);
    }
}
