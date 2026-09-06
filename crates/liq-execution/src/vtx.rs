//! Minimal Solana VersionedTransaction (v0) encoder for shadow simulateTransaction.
//! No solana-sdk dependency — produces RPC-acceptable base64 with dummy signatures
//! for `sigVerify: false` (+ typically `replaceRecentBlockhash: true`).

use base64::Engine;
use liq_core::{Instruction, Pubkey};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VtxError {
    #[error("no instructions")]
    EmptyInstructions,
    #[error("too many account keys ({0}) for u8 indices")]
    TooManyKeys(usize),
    #[error("account key missing from message table")]
    MissingKey,
}

fn shortvec_encode(n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut v = n;
    while v >= 0x80 {
        out.push(((v & 0x7f) as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyFlags {
    is_signer: bool,
    is_writable: bool,
}

/// Build account key table in Solana message order:
/// writable signers → readonly signers → writable nonsigners → readonly nonsigners.
fn build_account_keys(
    fee_payer: &Pubkey,
    ixs: &[Instruction],
) -> (Vec<Pubkey>, u8, u8, u8) {
    use std::collections::HashMap;
    let mut flags: HashMap<[u8; 32], KeyFlags> = HashMap::new();
    // Fee payer is always a writable signer.
    flags.insert(
        fee_payer.0,
        KeyFlags {
            is_signer: true,
            is_writable: true,
        },
    );
    for ix in ixs {
        let e = flags.entry(ix.program_id.0).or_insert(KeyFlags {
            is_signer: false,
            is_writable: false,
        });
        // program ids are never writable/signer via metas alone
        let _ = e;
        for m in &ix.accounts {
            let e = flags.entry(m.pubkey.0).or_insert(KeyFlags {
                is_signer: false,
                is_writable: false,
            });
            e.is_signer |= m.is_signer;
            e.is_writable |= m.is_writable;
        }
        // ensure program id present as readonly nonsigner unless already flagged
        flags.entry(ix.program_id.0).or_insert(KeyFlags {
            is_signer: false,
            is_writable: false,
        });
    }

    let mut writable_signers = Vec::new();
    let mut readonly_signers = Vec::new();
    let mut writable_nonsigners = Vec::new();
    let mut readonly_nonsigners = Vec::new();

    // Fee payer first among writable signers.
    writable_signers.push(*fee_payer);

    for (k, f) in &flags {
        if *k == fee_payer.0 {
            continue;
        }
        let pk = Pubkey::new(*k);
        match (f.is_signer, f.is_writable) {
            (true, true) => writable_signers.push(pk),
            (true, false) => readonly_signers.push(pk),
            (false, true) => writable_nonsigners.push(pk),
            (false, false) => readonly_nonsigners.push(pk),
        }
    }

    let num_required_signatures = (writable_signers.len() + readonly_signers.len()) as u8;
    let num_readonly_signed = readonly_signers.len() as u8;
    let num_readonly_unsigned = readonly_nonsigners.len() as u8;

    let mut keys = Vec::new();
    keys.extend(writable_signers);
    keys.extend(readonly_signers);
    keys.extend(writable_nonsigners);
    keys.extend(readonly_nonsigners);
    (
        keys,
        num_required_signatures,
        num_readonly_signed,
        num_readonly_unsigned,
    )
}

fn index_of(keys: &[Pubkey], pk: &Pubkey) -> Result<u8, VtxError> {
    keys.iter()
        .position(|k| k == pk)
        .map(|i| i as u8)
        .ok_or(VtxError::MissingKey)
}

/// Encode a v0 VersionedTransaction (unsigned / dummy signatures) as base64.
pub fn encode_versioned_tx_base64(
    ixs: &[Instruction],
    fee_payer: &Pubkey,
    recent_blockhash: &[u8; 32],
) -> Result<String, VtxError> {
    let raw = encode_versioned_tx_bytes(ixs, fee_payer, recent_blockhash)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(raw))
}

/// Wire bytes for a v0 VersionedTransaction with zeroed signatures.
pub fn encode_versioned_tx_bytes(
    ixs: &[Instruction],
    fee_payer: &Pubkey,
    recent_blockhash: &[u8; 32],
) -> Result<Vec<u8>, VtxError> {
    if ixs.is_empty() {
        return Err(VtxError::EmptyInstructions);
    }
    let (keys, num_required_signatures, num_readonly_signed, num_readonly_unsigned) =
        build_account_keys(fee_payer, ixs);
    if keys.len() > 256 {
        return Err(VtxError::TooManyKeys(keys.len()));
    }

    let mut message = Vec::new();
    // v0 version prefix
    message.push(0x80);
    message.push(num_required_signatures);
    message.push(num_readonly_signed);
    message.push(num_readonly_unsigned);
    message.extend_from_slice(&shortvec_encode(keys.len()));
    for k in &keys {
        message.extend_from_slice(&k.0);
    }
    message.extend_from_slice(recent_blockhash);

    // compiled instructions
    let mut compiled = Vec::new();
    compiled.extend_from_slice(&shortvec_encode(ixs.len()));
    for ix in ixs {
        let prog_idx = index_of(&keys, &ix.program_id)?;
        compiled.push(prog_idx);
        compiled.extend_from_slice(&shortvec_encode(ix.accounts.len()));
        for m in &ix.accounts {
            compiled.push(index_of(&keys, &m.pubkey)?);
        }
        compiled.extend_from_slice(&shortvec_encode(ix.data.len()));
        compiled.extend_from_slice(&ix.data);
    }
    message.extend_from_slice(&compiled);
    // address table lookups: empty
    message.extend_from_slice(&shortvec_encode(0));

    // signatures: num_required_signatures × 64 zero bytes
    let mut tx = Vec::new();
    tx.extend_from_slice(&shortvec_encode(num_required_signatures as usize));
    for _ in 0..num_required_signatures {
        tx.extend_from_slice(&[0u8; 64]);
    }
    tx.extend_from_slice(&message);
    Ok(tx)
}

/// Decode blockhash from base58 (Solana RPC `getLatestBlockhash` value).
pub fn blockhash_from_base58(s: &str) -> Option<[u8; 32]> {
    let bytes = bs58::decode(s).into_vec().ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use liq_core::{compute_unit_limit, compute_unit_price, programs, AccountMeta, Instruction};

    fn memo_like(data: &[u8]) -> Instruction {
        // Treat system program as a stand-in "memo-like" program for structure tests.
        Instruction::new(
            programs::system(),
            vec![AccountMeta::new_readonly(Pubkey::test(9, 1), false)],
            data.to_vec(),
        )
    }

    #[test]
    fn encode_cu_and_memo_nonzero_base64() {
        let payer = Pubkey::test(1, 1);
        let ixs = vec![
            compute_unit_limit(200_000),
            compute_unit_price(1_000),
            memo_like(b"shadow-vtx-test"),
        ];
        let b64 = encode_versioned_tx_base64(&ixs, &payer, &[7u8; 32]).unwrap();
        assert!(!b64.is_empty());
        let raw = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .unwrap();
        assert!(raw.len() > 100, "len={}", raw.len());
        // v0 prefix somewhere after signatures
        assert!(raw.iter().any(|&b| b == 0x80));
        // roundtrip structure: starts with shortvec sig count = 1
        assert_eq!(raw[0], 1);
        // 64 zero sig bytes
        assert!(raw[1..65].iter().all(|&b| b == 0));
        assert_eq!(raw[65], 0x80); // versioned message
    }

    #[test]
    fn empty_ixs_err() {
        let payer = Pubkey::test(1, 2);
        assert_eq!(
            encode_versioned_tx_base64(&[], &payer, &[0u8; 32]),
            Err(VtxError::EmptyInstructions)
        );
    }

    #[test]
    fn blockhash_b58_roundtrip_len() {
        let h = [9u8; 32];
        let s = bs58::encode(&h).into_string();
        assert_eq!(blockhash_from_base58(&s), Some(h));
    }
}
