//! The NEAR AI Cloud receipt text and its EIP-191 digest.
//!
//! NEAR AI Cloud signs `"{model}:{sha256_hex(request)}:{sha256_hex(response)}"`
//! as an Ethereum personal message (EIP-191, secp256k1). This module builds
//! that text and digest. Signature recovery runs in the contract
//! (`env::ecrecover`); it is not built yet (day S+2, gate G2), which will also
//! confirm the exact text format against a real recorded receipt.

use sha2::{Digest, Sha256};
use sha3::Keccak256;

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(sha256(data))
}

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

/// The text NEAR AI Cloud signs, from the raw request and response bytes.
pub fn signed_text(model: &str, request: &[u8], response: &[u8]) -> String {
    signed_text_from_hashes(model, &sha256(request), &sha256(response))
}

/// The same text when only the hashes are known (the contract has the raw
/// request but only the response hash).
pub fn signed_text_from_hashes(model: &str, req_hash: &[u8; 32], resp_hash: &[u8; 32]) -> String {
    format!(
        "{}:{}:{}",
        model,
        hex::encode(req_hash),
        hex::encode(resp_hash)
    )
}

/// EIP-191 personal-message digest:
/// `keccak256("\x19Ethereum Signed Message:\n" ++ len(msg) ++ msg)`.
pub fn eip191_digest(msg: &[u8]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(msg.len() + 32);
    buf.extend_from_slice(b"\x19Ethereum Signed Message:\n");
    buf.extend_from_slice(msg.len().to_string().as_bytes());
    buf.extend_from_slice(msg);
    keccak256(&buf)
}

/// Ethereum address of an uncompressed secp256k1 public key (64 bytes, x||y):
/// the last 20 bytes of `keccak256(pk)`.
pub fn eth_address(pubkey: &[u8; 64]) -> [u8; 20] {
    let h = keccak256(pubkey);
    let mut out = [0u8; 20];
    out.copy_from_slice(&h[12..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_text_shape() {
        let t = signed_text("deepseek-v3", b"req", b"resp");
        assert_eq!(
            t,
            format!("deepseek-v3:{}:{}", sha256_hex(b"req"), sha256_hex(b"resp"))
        );
        assert_eq!(t.len(), "deepseek-v3".len() + 2 + 128);
    }

    #[test]
    fn signed_text_from_hashes_matches() {
        assert_eq!(
            signed_text("m", b"a", b"b"),
            signed_text_from_hashes("m", &sha256(b"a"), &sha256(b"b"))
        );
    }

    #[test]
    fn keccak_known_vector() {
        // keccak256("") (Ethereum's empty hash).
        assert_eq!(
            hex::encode(keccak256(b"")),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn eip191_known_vector() {
        // ethers.js hashMessage("hello world").
        assert_eq!(
            hex::encode(eip191_digest(b"hello world")),
            "d9eba16ed0ecae432b71fe008c98cc872bb4cc214d3220a36f365326cf807d68"
        );
    }

    #[test]
    fn eth_address_of_generator_point() {
        // Private key 1 -> public key G -> 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf.
        let pk = hex::decode(concat!(
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            "483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8"
        ))
        .unwrap();
        let pk: [u8; 64] = pk.try_into().unwrap();
        assert_eq!(
            hex::encode(eth_address(&pk)),
            "7e5f4552091a69125d5dfcb7b8c2659029395bdf"
        );
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
