//! Mainnet read-only discovery: known markets/groups + filtered GPA (RPS-friendly).
//! Decodes what we can; records unknowns. Never logs API keys (host-only).

use crate::bootstrap::{BootstrapError, JsonRpcBootstrap, RawAccount, RpcBootstrap};
use crate::redact::short_b58;
use crate::rpc_pool::{EndpointStats, RotatingRpcPool};
use liq_core::{CandidateBand, Protocol, Pubkey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};

/// Documented mainnet pubkeys (PROTOCOL_RESEARCH + public docs).
pub mod known {
    /// Kamino primary lending market.
    pub const KLEND_MAIN_MARKET: &str = "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF";
    /// Project 0 / marginfi-v2 main group.
    pub const MARGINFI_MAIN_GROUP: &str = "4qp6Fx6tnZkY5Wropq9wUYgtFxXKwE6viZxFHg3rdAG8";
    /// Save / Solend main lending market (classic main pool).
    pub const SAVE_MAIN_MARKET: &str = "4UpD2fh7xH3VP9QQaXtsS1YY3bxzWhtfpks7FatyKvdY";
    /// Public mainnet system accounts often used as shadow fee-payers (sigVerify=false; never signed).
    pub const SIM_FEE_PAYER_CANDIDATES: &[&str] = &[
        // Save market owner (docs.save.finance) — typically system-owned / funded
        "5pHk2TmnqQzRF9L6egy5FfiyBgS7G9cMZ5RFaJAvghzw",
        // Well-known high-lamport system account (Binance 2 cold — public address only)
        "5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9",
    ];

    /// Klend Obligation account size (8-byte disc + layout) — from klend IDL / docs.
    pub const KLEND_OBLIGATION_DATASIZE: u64 = 3344;
    /// Klend Reserve account size.
    pub const KLEND_RESERVE_DATASIZE: u64 = 8624; // 8 + 8616
    /// Marginfi account discriminator (type-crate).
    pub const MARGINFI_ACCOUNT_DISC: [u8; 8] = [67, 178, 130, 109, 126, 114, 28, 42];
    /// Marginfi bank discriminator.
    pub const MARGINFI_BANK_DISC: [u8; 8] = [142, 49, 166, 242, 50, 66, 97, 188];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredAccount {
    pub protocol: String,
    pub kind: String,
    /// Short base58 for reports.
    pub pubkey_short: String,
    pub data_len: usize,
    pub decode: String,
    pub health: Option<f64>,
    pub band: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub slot: u64,
    pub health: String,
    pub endpoint_host: String,
    pub endpoint_stats: Vec<EndpointStats>,
    pub program_ids: Value,
    pub known_markets: Value,
    pub accounts_scanned: usize,
    pub by_protocol: Value,
    pub candidates: Vec<DiscoveredAccount>,
    pub gaps: Vec<String>,
}

fn pk(s: &str) -> Result<Pubkey, BootstrapError> {
    Pubkey::from_base58(s).ok_or_else(|| BootstrapError::Decode(format!("bad pubkey {s}")))
}

fn short_pk(p: &Pubkey) -> String {
    short_b58(&p.to_base58())
}

/// Partial live Klend Obligation decode using IDL offsets (see decode module in liq-kamino).
pub fn classify_klend_obligation(addr: &Pubkey, data: &[u8]) -> DiscoveredAccount {
    let mut notes = Vec::new();
    let mut health = None;
    let mut band = None;
    let decode = if data.len() < 2260 {
        notes.push("too_short_for_value_fields".into());
        "incomplete".into()
    } else {
        match liq_kamino::decode_obligation_live_header(*addr, data) {
            Ok(h) => {
                notes.push(format!("market={}", short_b58(&h.lending_market.to_base58())));
                notes.push(format!("has_debt={}", h.has_debt));
                if h.has_debt && h.borrowed_assets_market_value_sf > 0 {
                    // Approximate HF = unhealthy / borrowed (SF units cancel).
                    let hf = if h.borrowed_assets_market_value_sf == 0 {
                        1000.0
                    } else {
                        (h.unhealthy_borrow_value_sf as f64)
                            / (h.borrowed_assets_market_value_sf as f64)
                    };
                    health = Some(hf);
                    let band_e = if hf < 1.0 {
                        CandidateBand::Critical
                    } else if hf < 1.05 {
                        CandidateBand::Hot
                    } else if hf < 1.2 {
                        CandidateBand::Warm
                    } else {
                        CandidateBand::Cold
                    };
                    band = Some(band_e.as_str().into());
                } else {
                    notes.push("no_debt_or_zero_borrow_sf".into());
                }
                "live_header".into()
            }
            Err(e) => {
                notes.push(format!("decode_err={e}"));
                "unknown".into()
            }
        }
    };
    DiscoveredAccount {
        protocol: format!("{:?}", Protocol::Kamino),
        kind: "obligation".into(),
        pubkey_short: short_pk(addr),
        data_len: data.len(),
        decode,
        health,
        band,
        notes,
    }
}

fn classify_raw(protocol: Protocol, kind: &str, a: &RawAccount) -> DiscoveredAccount {
    let mut notes = Vec::new();
    let decode = if a.data.len() >= 8 {
        notes.push(format!(
            "disc={:02x?}",
            &a.data[..8.min(a.data.len())]
        ));
        "header_only".into()
    } else {
        "empty".into()
    };
    DiscoveredAccount {
        protocol: format!("{protocol:?}"),
        kind: kind.into(),
        pubkey_short: short_pk(&a.pubkey),
        data_len: a.data.len(),
        decode,
        health: None,
        band: None,
        notes,
    }
}

/// Run read-only mainnet discovery using a rotating RPC pool.
pub async fn discover_mainnet(pool: &RotatingRpcPool) -> Result<DiscoveryReport, BootstrapError> {
    let boot = JsonRpcBootstrap::new(pool.clone());
    let mut gaps = Vec::new();

    let slot = boot.get_slot().await?;
    let health = match boot.get_health().await {
        Ok(h) => h,
        Err(e) => {
            gaps.push(format!("getHealth failed: {e}"));
            "unknown".into()
        }
    };
    info!(slot, health = %health, host = %pool.current_host(), "mainnet RPC live");

    let klend_market = pk(known::KLEND_MAIN_MARKET)?;
    let mfi_group = pk(known::MARGINFI_MAIN_GROUP)?;
    let save_market = pk(known::SAVE_MAIN_MARKET)?;

    let known_keys = [klend_market, mfi_group, save_market];
    let known_accts = boot.get_multiple_accounts(&known_keys).await?;
    let mut known_markets = json!({});
    let labels = ["klend_main_market", "marginfi_main_group", "save_main_market"];
    for (i, label) in labels.iter().enumerate() {
        let present = known_accts.get(i).and_then(|x| x.as_ref());
        known_markets[label] = match present {
            Some(a) => json!({
                "pubkey_short": short_pk(&a.pubkey),
                "lamports": a.lamports,
                "data_len": a.data.len(),
                "owner_short": short_b58(&a.owner.to_base58()),
            }),
            None => {
                gaps.push(format!("{label} missing on-chain"));
                json!(null)
            }
        };
    }

    let mut scanned: Vec<DiscoveredAccount> = Vec::new();
    let mut counts = json!({
        "kamino_obligations": 0,
        "kamino_reserves": 0,
        "marginfi_accounts": 0,
        "marginfi_banks": 0,
        "save_accounts": 0,
    });

    // --- Klend obligations: dataSize + memcmp lending_market @ 32 ---
    let obl_filters = vec![
        json!({ "dataSize": known::KLEND_OBLIGATION_DATASIZE }),
        json!({
            "memcmp": {
                "offset": 32,
                "bytes": known::KLEND_MAIN_MARKET
            }
        }),
    ];
    match boot
        .get_program_accounts_filtered(&liq_core::programs::klend(), &obl_filters)
        .await
    {
        Ok(accts) => {
            counts["kamino_obligations"] = json!(accts.len());
            info!(n = accts.len(), "klend obligations (filtered GPA)");
            // Cap detailed decode for report / candidate scan
            for a in accts.iter().take(64) {
                scanned.push(classify_klend_obligation(&a.pubkey, &a.data));
            }
            if accts.len() > 64 {
                gaps.push(format!(
                    "klend obligations truncated in report: {} total, decoded first 64",
                    accts.len()
                ));
            }
        }
        Err(e) => {
            warn!(error = %e, "klend obligation GPA failed — falling back to reserves-only");
            gaps.push(format!("klend obligation GPA: {e}"));
            // Fallback: smaller scoped reserves fetch
            let res_filters = vec![
                json!({ "dataSize": known::KLEND_RESERVE_DATASIZE }),
                json!({
                    "memcmp": {
                        "offset": 32,
                        "bytes": known::KLEND_MAIN_MARKET
                    }
                }),
            ];
            match boot
                .get_program_accounts_filtered(&liq_core::programs::klend(), &res_filters)
                .await
            {
                Ok(accts) => {
                    counts["kamino_reserves"] = json!(accts.len());
                    for a in accts.iter().take(32) {
                        scanned.push(classify_raw(Protocol::Kamino, "reserve", a));
                    }
                }
                Err(e2) => gaps.push(format!("klend reserve GPA: {e2}")),
            }
        }
    }

    // If obligations GPA succeeded but we want reserve count too (cheap-ish).
    if counts["kamino_reserves"] == 0 {
        let res_filters = vec![
            json!({ "dataSize": known::KLEND_RESERVE_DATASIZE }),
            json!({
                "memcmp": {
                    "offset": 32,
                    "bytes": known::KLEND_MAIN_MARKET
                }
            }),
        ];
        match boot
            .get_program_accounts_filtered(&liq_core::programs::klend(), &res_filters)
            .await
        {
            Ok(accts) => {
                counts["kamino_reserves"] = json!(accts.len());
                info!(n = accts.len(), "klend reserves (filtered GPA)");
            }
            Err(e) => gaps.push(format!("klend reserve GPA: {e}")),
        }
    }

    // --- Marginfi: memcmp ACCOUNT disc @ 0 + group pubkey (group often at offset 8+…) ---
    // Group field typically follows 8-byte disc in MarginfiAccount.
    let group_b58 = known::MARGINFI_MAIN_GROUP;
    let mfi_acct_filters = vec![
        json!({
            "memcmp": {
                "offset": 0,
                "bytes": bs58::encode(known::MARGINFI_ACCOUNT_DISC).into_string()
            }
        }),
        json!({
            "memcmp": {
                "offset": 8,
                "bytes": group_b58
            }
        }),
    ];
    match boot
        .get_program_accounts_filtered(&liq_core::programs::marginfi(), &mfi_acct_filters)
        .await
    {
        Ok(accts) => {
            counts["marginfi_accounts"] = json!(accts.len());
            info!(n = accts.len(), "marginfi accounts (filtered GPA)");
            for a in accts.iter().take(32) {
                let mut d = classify_raw(Protocol::Project0, "marginfi_account", a);
                d.notes
                    .push("full_health_decode_pending_zero_copy".into());
                scanned.push(d);
            }
        }
        Err(e) => {
            gaps.push(format!("marginfi account GPA: {e}"));
            // Banks only
            let bank_filters = vec![json!({
                "memcmp": {
                    "offset": 0,
                    "bytes": bs58::encode(known::MARGINFI_BANK_DISC).into_string()
                }
            })];
            match boot
                .get_program_accounts_filtered(&liq_core::programs::marginfi(), &bank_filters)
                .await
            {
                Ok(accts) => {
                    counts["marginfi_banks"] = json!(accts.len());
                    for a in accts.iter().take(16) {
                        scanned.push(classify_raw(Protocol::Project0, "bank", a));
                    }
                }
                Err(e2) => gaps.push(format!("marginfi bank GPA: {e2}")),
            }
        }
    }

    // --- Save: getAccountInfo on known market; optional GPA with dataSize for obligations (~1300) ---
    // Solend Obligation size historically ~916–1300; use market getAccountInfo proof + small GPA attempt.
    if let Some(Some(mkt)) = known_accts.get(2) {
        scanned.push(classify_raw(Protocol::Save, "lending_market", mkt));
    }
    // Save obligation layout/size varies by market version; unscoped GPA is RPS-hostile.
    // Prefer market getAccountInfo proof + optional tiny memcmp sample (owner byte shard).
    gaps.push(
        "Save: skipped full GPA — obligation dataSize/market offset not yet pinned; market account fetched via getMultipleAccounts"
            .into(),
    );
    let save_sample_filters = vec![
        json!({
            "memcmp": {
                "offset": 10,
                "bytes": known::SAVE_MAIN_MARKET
            }
        }),
        // shard: first byte of pubkey path not available in filters; use dataSize common classic size 916
        json!({ "dataSize": 916 }),
    ];
    match boot
        .get_program_accounts_filtered(&liq_core::programs::save(), &save_sample_filters)
        .await
    {
        Ok(accts) => {
            counts["save_accounts"] = json!(accts.len());
            info!(n = accts.len(), "save accounts sample (dataSize=916 + market@10)");
            for a in accts.iter().take(16) {
                let mut d = classify_raw(Protocol::Save, "obligation?", a);
                d.notes.push("save_partial_sample".into());
                scanned.push(d);
            }
        }
        Err(e) => {
            gaps.push(format!("save sample GPA failed (market still fetched): {e}"));
        }
    }

    gaps.push(
        "Full Anchor zero-copy (Kamino deposits/borrows arrays, marginfi balances, Save reserves) still incomplete — live_header SF health used where available"
            .into(),
    );

    let candidates: Vec<_> = scanned
        .iter()
        .filter(|c| {
            matches!(
                c.band.as_deref(),
                Some("CRITICAL") | Some("HOT") | Some("WARM")
            ) || c.health.map(|h| h < 1.05).unwrap_or(false)
        })
        .cloned()
        .collect();

    let accounts_scanned = {
        let mut n = 0usize;
        for k in [
            "kamino_obligations",
            "kamino_reserves",
            "marginfi_accounts",
            "marginfi_banks",
            "save_accounts",
        ] {
            n += counts[k].as_u64().unwrap_or(0) as usize;
        }
        n += known_accts.iter().filter(|x| x.is_some()).count();
        n
    };

    Ok(DiscoveryReport {
        slot,
        health,
        endpoint_host: pool.current_host(),
        endpoint_stats: pool.stats_snapshot(),
        program_ids: json!({
            "klend": short_b58(liq_kamino::KLEND_PROGRAM_ID_MAINNET),
            "marginfi": short_b58(liq_project0::MARGINFI_PROGRAM_ID_MAINNET),
            "save": short_b58(liq_save::SAVE_PROGRAM_ID_MAINNET),
        }),
        known_markets,
        accounts_scanned,
        by_protocol: counts,
        candidates,
        gaps,
    })
}

/// Convenience: build pool from env URLs.
pub fn pool_from_env() -> Result<RotatingRpcPool, BootstrapError> {
    let urls = crate::local_env::rpc_urls_from_env();
    RotatingRpcPool::from_urls(urls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_pubkeys_decode() {
        assert!(Pubkey::from_base58(known::KLEND_MAIN_MARKET).is_some());
        assert!(Pubkey::from_base58(known::MARGINFI_MAIN_GROUP).is_some());
        assert!(Pubkey::from_base58(known::SAVE_MAIN_MARKET).is_some());
    }

    #[test]
    fn classify_short_obligation() {
        let pk = Pubkey::test(1, 1);
        let d = classify_klend_obligation(&pk, &[0u8; 10]);
        assert_eq!(d.decode, "incomplete");
    }
}
