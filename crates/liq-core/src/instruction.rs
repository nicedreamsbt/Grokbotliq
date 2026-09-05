//! Wire-ready Solana instruction types without solana-* deps.

use crate::types::Pubkey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl AccountMeta {
    pub fn new(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: true,
        }
    }

    pub fn new_readonly(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: false,
        }
    }
}

/// VersionedTransaction-ready instruction (program_id + metas + data bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instruction {
    pub program_id: Pubkey,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

impl Instruction {
    pub fn new(program_id: Pubkey, accounts: Vec<AccountMeta>, data: Vec<u8>) -> Self {
        Self {
            program_id,
            accounts,
            data,
        }
    }

    pub fn is_wire_ready(&self) -> bool {
        !self.data.is_empty() || !self.accounts.is_empty()
    }
}

/// Named instruction for logging / opportunity JSON (keeps wire ixs separate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabeledIx {
    pub label: String,
    pub ix: Instruction,
}

/// Well-known mainnet program IDs (bytes verified via base58 decode).
pub mod programs {
    use crate::types::Pubkey;

    pub fn system() -> Pubkey {
        Pubkey::new([0u8; 32])
    }

    pub fn token() -> Pubkey {
        Pubkey::new([
            6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180,
            133, 237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
        ])
    }

    pub fn compute_budget() -> Pubkey {
        Pubkey::new([
            3, 6, 70, 111, 229, 33, 23, 50, 255, 236, 173, 186, 114, 195, 155, 231, 188, 140, 229,
            187, 197, 247, 18, 107, 44, 67, 155, 58, 64, 0, 0, 0,
        ])
    }

    pub fn sysvar_instructions() -> Pubkey {
        Pubkey::new([
            6, 167, 213, 23, 24, 123, 209, 102, 53, 218, 212, 4, 85, 253, 194, 192, 193, 36, 198,
            143, 33, 86, 117, 165, 219, 186, 203, 95, 8, 0, 0, 0,
        ])
    }

    pub fn klend() -> Pubkey {
        Pubkey::new([
            4, 178, 172, 177, 18, 88, 204, 227, 104, 44, 65, 139, 168, 114, 255, 61, 249, 17, 2,
            113, 47, 21, 175, 18, 182, 190, 105, 179, 67, 91, 0, 8,
        ])
    }

    pub fn save() -> Pubkey {
        Pubkey::new([
            6, 155, 139, 152, 90, 171, 83, 42, 69, 9, 13, 232, 85, 127, 205, 220, 190, 108, 183,
            239, 199, 58, 10, 101, 176, 111, 146, 3, 93, 183, 62, 236,
        ])
    }

    pub fn marginfi() -> Pubkey {
        Pubkey::new([
            5, 48, 122, 214, 69, 75, 188, 94, 30, 78, 146, 5, 146, 83, 161, 139, 184, 200, 134,
            140, 88, 166, 49, 46, 200, 106, 57, 230, 34, 78, 55, 59,
        ])
    }
}

/// ComputeBudget: SetComputeUnitLimit (tag 2) + units le u32.
pub fn compute_unit_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction::new(programs::compute_budget(), vec![], data)
}

/// ComputeBudget: SetComputeUnitPrice (tag 3) + micro_lamports le u64.
pub fn compute_unit_price(micro_lamports: u64) -> Instruction {
    let mut data = vec![3u8];
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    Instruction::new(programs::compute_budget(), vec![], data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_budget_data_nonempty() {
        let ix = compute_unit_limit(200_000);
        assert_eq!(ix.data[0], 2);
        assert_eq!(ix.program_id, programs::compute_budget());
        assert!(ix.is_wire_ready());
    }

    #[test]
    fn wellknown_program_ids_nonzero() {
        assert_ne!(programs::token().0, [0u8; 32]);
        assert_ne!(programs::klend().0, [0u8; 32]);
        assert_ne!(programs::save().0, [0u8; 32]);
        assert_ne!(programs::marginfi().0, [0u8; 32]);
    }
}
