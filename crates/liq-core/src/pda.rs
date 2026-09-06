//! Program-derived address and Associated Token Account helpers (no solana-sdk).

use crate::instruction::programs;
use crate::types::Pubkey;
use curve25519_dalek::edwards::CompressedEdwardsY;
use sha2::{Digest, Sha256};

/// `find_program_address` — off-curve SHA256 PDA with bump (Solana convention).
pub fn find_program_address(seeds: &[&[u8]], program_id: &Pubkey) -> (Pubkey, u8) {
    for bump in (0u8..=255).rev() {
        let bump_arr = [bump];
        let mut hasher = Sha256::new();
        for s in seeds {
            hasher.update(s);
        }
        hasher.update(bump_arr);
        hasher.update(program_id.0);
        hasher.update(b"ProgramDerivedAddress");
        let hash = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&hash);
        if !is_on_curve(&bytes) {
            return (Pubkey::new(bytes), bump);
        }
    }
    // Solana panics if no bump found; return a deterministic fallback for compile safety.
    (Pubkey::default(), 0)
}

fn is_on_curve(bytes: &[u8; 32]) -> bool {
    CompressedEdwardsY(*bytes).decompress().is_some()
}

/// Associated Token Account address (classic ATA program derivation).
/// Seeds: `[owner, token_program, mint]` under Associated Token Program.
pub fn get_associated_token_address(
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    find_program_address(
        &[owner.0.as_ref(), token_program.0.as_ref(), mint.0.as_ref()],
        &programs::associated_token(),
    )
    .0
}

/// Convenience: ATA under SPL Token program.
pub fn get_associated_token_address_token(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    get_associated_token_address(owner, mint, &programs::token())
}

/// Klend referrer token state PDA: `["referrer_acc", referrer, reserve]`.
pub fn klend_referrer_token_state(referrer: &Pubkey, reserve: &Pubkey) -> Pubkey {
    find_program_address(
        &[b"referrer_acc", referrer.0.as_ref(), reserve.0.as_ref()],
        &programs::klend(),
    )
    .0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ata_usdc_known_vector() {
        // Well-known: System Program owner is invalid for ATA ownership in practice,
        // but derivation is deterministic. Use a documented wallet + USDC mint.
        let owner = Pubkey::from_base58("5pHk2TmnqQzRF9L6egy5FfiyBgS7G9cMZ5RFaJAvghzw").unwrap();
        let usdc = Pubkey::from_base58("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let ata = get_associated_token_address_token(&owner, &usdc);
        // Re-derive must be stable
        let ata2 = get_associated_token_address(&owner, &usdc, &programs::token());
        assert_eq!(ata, ata2);
        assert_ne!(ata.0, [0u8; 32]);
        assert_ne!(ata, owner);
        assert_ne!(ata, usdc);
        // Cross-checked vs solders Pubkey.find_program_address (2026-09-05).
        assert_eq!(
            ata.to_base58(),
            "FfqKAY2NEH7pofsVU1zELif6awbvtiXs2DJPbpWumW5w"
        );
    }

    #[test]
    fn associated_token_program_id_matches_base58() {
        let pk = programs::associated_token();
        assert_eq!(
            pk.to_base58(),
            "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"
        );
    }

    #[test]
    fn klend_referrer_pda_deterministic() {
        let r = Pubkey::test(1, 1);
        let res = Pubkey::test(1, 2);
        let a = klend_referrer_token_state(&r, &res);
        let b = klend_referrer_token_state(&r, &res);
        assert_eq!(a, b);
        assert_ne!(a, r);
    }

    #[test]
    fn find_program_address_system_nonce_style() {
        // Empty seeds under system program should produce a valid off-curve PDA.
        let (pda, bump) = find_program_address(&[], &programs::system());
        assert!(bump > 0 || pda.0 != [0u8; 32]);
        assert!(!is_on_curve(&pda.0));
    }
}
