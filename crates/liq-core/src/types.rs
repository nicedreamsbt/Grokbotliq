use serde::{Deserialize, Serialize};
use std::fmt;

/// 32-byte Solana pubkey (no solana-* dependency in core).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct Pubkey(pub [u8; 32]);

impl Pubkey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut arr = [0u8; 32];
        if bytes.len() != 32 {
            return None;
        }
        arr.copy_from_slice(bytes);
        Some(Self(arr))
    }

    /// Deterministic test helper: fill with `tag` then index bytes.
    pub fn test(tag: u8, index: u64) -> Self {
        let mut b = [tag; 32];
        b[24..32].copy_from_slice(&index.to_le_bytes());
        Self(b)
    }

    /// Base58 Solana address (for JSON-RPC keys).
    pub fn to_base58(&self) -> String {
        bs58::encode(&self.0).into_string()
    }

    pub fn from_base58(s: &str) -> Option<Self> {
        let bytes = bs58::decode(s).into_vec().ok()?;
        Self::from_bytes(&bytes)
    }
}

impl fmt::Debug for Pubkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pubkey({:02x}..{:02x})", self.0[0], self.0[31])
    }
}

impl fmt::Display for Pubkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // hex short form for logs (base58 optional later)
        for b in &self.0[..4] {
            write!(f, "{:02x}", b)?;
        }
        write!(f, "…")?;
        for b in &self.0[28..] {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Protocol {
    Kamino,
    Project0,
    Save,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CandidateBand {
    Critical = 0,
    Hot = 1,
    Warm = 2,
    Cold = 3,
}

impl CandidateBand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::Hot => "HOT",
            Self::Warm => "WARM",
            Self::Cold => "COLD",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateSource {
    Geyser,
    Rpc,
    Replay,
    Mock,
    Computed,
}

/// Fixed-point USD price with 9 decimal places (1e9 = $1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct PriceFx(pub u128);

impl PriceFx {
    pub const SCALE: u128 = 1_000_000_000;

    pub fn from_f64(v: f64) -> Self {
        Self((v.max(0.0) * Self::SCALE as f64).round() as u128)
    }

    pub fn to_f64(self) -> f64 {
        self.0 as f64 / Self::SCALE as f64
    }
}

/// Health factor with 6 decimal places (1e6 = 1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct HealthFx(pub i128);

impl HealthFx {
    pub const SCALE: i128 = 1_000_000;

    pub fn from_f64(v: f64) -> Self {
        Self((v * Self::SCALE as f64).round() as i128)
    }

    pub fn to_f64(self) -> f64 {
        self.0 as f64 / Self::SCALE as f64
    }

    pub fn is_liquidatable(self) -> bool {
        self.0 < Self::SCALE // HF < 1.0
    }
}
