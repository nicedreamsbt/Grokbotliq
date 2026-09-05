//! Load oracle tick + borrower snapshot fixtures for replay/shadow.

use crate::{StreamError, StreamEvent};
use liq_core::{
    CandidateBand, CandidateMeta, HealthFx, PriceFx, PriceTrigger, Protocol, Pubkey, TriggerSide,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct FixturePubkey {
    pub tag: u8,
    pub index: u64,
}

impl FixturePubkey {
    pub fn to_pubkey(&self) -> Pubkey {
        Pubkey::test(self.tag, self.index)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct OracleTickFixture {
    pub asset: FixturePubkey,
    #[serde(default)]
    pub label: Option<String>,
    pub price_usd: f64,
    pub slot: u64,
    pub write_version: u64,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OracleTicksFile {
    #[serde(default)]
    pub description: Option<String>,
    pub ticks: Vec<OracleTickFixture>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TriggerFixture {
    pub asset: FixturePubkey,
    pub side: String,
    pub trigger_price_usd: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanFixture {
    pub ix_order: String,
    pub repay_amount: u64,
    pub gross_profit_usd: f64,
    pub notional_usd: f64,
    pub capital_usd: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BorrowerFixture {
    pub account: FixturePubkey,
    pub protocol: String,
    pub band: String,
    pub health: f64,
    pub assets: Vec<FixturePubkey>,
    pub triggers: Vec<TriggerFixture>,
    pub plan: PlanFixture,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BorrowersFile {
    #[serde(default)]
    pub description: Option<String>,
    pub borrowers: Vec<BorrowerFixture>,
}

fn parse_protocol(s: &str) -> Result<Protocol, StreamError> {
    match s.to_ascii_lowercase().as_str() {
        "kamino" => Ok(Protocol::Kamino),
        "project0" | "p0" | "marginfi" => Ok(Protocol::Project0),
        "save" | "solend" => Ok(Protocol::Save),
        other => Err(StreamError::Fixture(format!("unknown protocol: {other}"))),
    }
}

fn parse_band(s: &str) -> Result<CandidateBand, StreamError> {
    match s.to_ascii_lowercase().as_str() {
        "critical" => Ok(CandidateBand::Critical),
        "hot" => Ok(CandidateBand::Hot),
        "warm" => Ok(CandidateBand::Warm),
        "cold" => Ok(CandidateBand::Cold),
        other => Err(StreamError::Fixture(format!("unknown band: {other}"))),
    }
}

fn parse_side(s: &str) -> Result<TriggerSide, StreamError> {
    match s.to_ascii_lowercase().as_str() {
        "collateraldown" | "collateral_down" => Ok(TriggerSide::CollateralDown),
        "debtup" | "debt_up" => Ok(TriggerSide::DebtUp),
        other => Err(StreamError::Fixture(format!("unknown side: {other}"))),
    }
}

pub fn load_oracle_ticks(path: impl AsRef<Path>) -> Result<OracleTicksFile, StreamError> {
    let raw = std::fs::read_to_string(path.as_ref())
        .map_err(|e| StreamError::Fixture(format!("read {}: {e}", path.as_ref().display())))?;
    serde_json::from_str(&raw).map_err(|e| StreamError::Fixture(format!("oracle json: {e}")))
}

pub fn load_borrowers(path: impl AsRef<Path>) -> Result<BorrowersFile, StreamError> {
    let raw = std::fs::read_to_string(path.as_ref())
        .map_err(|e| StreamError::Fixture(format!("read {}: {e}", path.as_ref().display())))?;
    serde_json::from_str(&raw).map_err(|e| StreamError::Fixture(format!("borrowers json: {e}")))
}

pub fn resolve_fixtures_dir(explicit: Option<&str>) -> PathBuf {
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("LIQ_FIXTURES") {
        return PathBuf::from(p);
    }
    for candidate in ["fixtures", "./fixtures", "../fixtures"] {
        let p = PathBuf::from(candidate);
        if p.join("oracle_ticks.json").exists() {
            return p;
        }
    }
    PathBuf::from("fixtures")
}

pub fn ticks_to_events(file: &OracleTicksFile) -> Vec<StreamEvent> {
    file.ticks
        .iter()
        .map(|t| StreamEvent::Price {
            asset: t.asset.to_pubkey(),
            price_fx: PriceFx::from_f64(t.price_usd).0,
            slot: t.slot,
            write_version: t.write_version,
        })
        .collect()
}

pub fn borrower_to_meta(b: &BorrowerFixture) -> Result<CandidateMeta, StreamError> {
    Ok(CandidateMeta {
        account: b.account.to_pubkey(),
        protocol: parse_protocol(&b.protocol)?,
        band: parse_band(&b.band)?,
        health: HealthFx::from_f64(b.health),
        assets: b.assets.iter().map(|a| a.to_pubkey()).collect(),
    })
}

pub fn borrower_triggers(b: &BorrowerFixture) -> Result<Vec<PriceTrigger>, StreamError> {
    let account = b.account.to_pubkey();
    b.triggers
        .iter()
        .map(|t| {
            Ok(PriceTrigger {
                account,
                asset: t.asset.to_pubkey(),
                side: parse_side(&t.side)?,
                trigger_price: PriceFx::from_f64(t.trigger_price_usd),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_repo_fixtures() {
        let dir = resolve_fixtures_dir(Some("fixtures"));
        // When cwd is crate during unit test, fixtures may be at ../../fixtures
        let dir = if dir.join("oracle_ticks.json").exists() {
            dir
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
        };
        let ticks = load_oracle_ticks(dir.join("oracle_ticks.json")).unwrap();
        assert!(ticks.ticks.len() >= 4);
        let borrowers = load_borrowers(dir.join("borrowers.json")).unwrap();
        assert_eq!(borrowers.borrowers.len(), 3);
        let meta = borrower_to_meta(&borrowers.borrowers[0]).unwrap();
        assert_eq!(meta.protocol, Protocol::Kamino);
        let events = ticks_to_events(&ticks);
        assert_eq!(events.len(), ticks.ticks.len());
    }
}
