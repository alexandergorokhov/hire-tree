//! The NEAR AI Cloud receipt text and its EIP-191 digest.
//!
//! NEAR AI Cloud signs `"{model}:{sha256_hex(request)}:{sha256_hex(response)}"`
//! as an Ethereum personal message (EIP-191, secp256k1). This module builds
//! that text and digest and parses the 65-byte signature (rejecting bad `v`
//! and high `s`). Recovery itself runs in the contract (`env::ecrecover`).
//! The text format is checked against a receipt recorded on testnet.

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

/// Longest model name accepted in a receipt.
pub const MAX_MODEL_LEN: usize = 128;

/// secp256k1 group order n / 2 (big-endian). An `s` above it is the
/// malleable twin of a valid signature and is refused.
const HALF_N: [u8; 32] = [
    0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b, 0x20, 0xa0,
];

/// A parsed ECDSA signature: `rs` = r || s, `v` = recovery id 0 or 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub rs: [u8; 64],
    pub v: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigError {
    NotHex,
    WrongLength(usize),
    BadV(u8),
    HighS,
    ZeroRorS,
}

impl core::fmt::Display for SigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SigError::NotHex => write!(f, "receipt: bad signature (not hex)"),
            SigError::WrongLength(n) => write!(f, "receipt: bad signature ({n} bytes, want 65)"),
            SigError::BadV(v) => write!(f, "receipt: bad signature (v = {v})"),
            SigError::HighS => write!(f, "receipt: bad signature (high s)"),
            SigError::ZeroRorS => write!(f, "receipt: bad signature (zero r or s)"),
        }
    }
}

/// Parse `r || s || v` hex (optional `0x`). `v` may be 0, 1, 27 or 28.
pub fn parse_signature(hex_sig: &str) -> Result<Signature, SigError> {
    let h = hex_sig.strip_prefix("0x").unwrap_or(hex_sig);
    let bytes = hex::decode(h).map_err(|_| SigError::NotHex)?;
    if bytes.len() != 65 {
        return Err(SigError::WrongLength(bytes.len()));
    }
    let v = match bytes[64] {
        0 | 27 => 0,
        1 | 28 => 1,
        other => return Err(SigError::BadV(other)),
    };
    let mut rs = [0u8; 64];
    rs.copy_from_slice(&bytes[..64]);
    if rs[..32].iter().all(|b| *b == 0) || rs[32..].iter().all(|b| *b == 0) {
        return Err(SigError::ZeroRorS);
    }
    // Big-endian compare of s with n/2.
    if rs[32..] > HALF_N[..] {
        return Err(SigError::HighS);
    }
    Ok(Signature { rs, v })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Public data from a NEAR AI Cloud receipt recorded on testnet on
    /// 2026-11-03 (Agent Exit gate G6, `chatcmpl-a701338b-...`): the text the
    /// service returned and the hashes it was made from.
    #[test]
    fn recorded_text_matches_near_ai_cloud() {
        let model = "deepseek-ai/DeepSeek-V4.1-Flash-1017";
        let req: [u8; 32] =
            hex::decode("74c84a9739582d7f1590b7b360d1766bf608bd70f4ad518fe9a352d638363fb7")
                .unwrap()
                .try_into()
                .unwrap();
        let resp: [u8; 32] =
            hex::decode("de751b3e23ab042406eec7f3314c41bb0c6e196eb46d131427284f0730b96a03")
                .unwrap()
                .try_into()
                .unwrap();
        assert_eq!(
            signed_text_from_hashes(model, &req, &resp),
            "deepseek-ai/DeepSeek-V4.1-Flash-1017:74c84a9739582d7f1590b7b360d1766bf608bd70f4ad518fe9a352d638363fb7:de751b3e23ab042406eec7f3314c41bb0c6e196eb46d131427284f0730b96a03"
        );
    }

    fn sig_with(s0: u8, v: u8) -> String {
        let mut b = vec![1u8; 32];
        b.extend(std::iter::repeat_n(s0, 32));
        b.push(v);
        hex::encode(b)
    }

    #[test]
    fn signature_v_forms() {
        assert_eq!(parse_signature(&sig_with(1, 27)).unwrap().v, 0);
        assert_eq!(parse_signature(&sig_with(1, 28)).unwrap().v, 1);
        assert_eq!(parse_signature(&sig_with(1, 0)).unwrap().v, 0);
        assert_eq!(
            parse_signature(&format!("0x{}", sig_with(1, 1))).unwrap().v,
            1
        );
        assert_eq!(parse_signature(&sig_with(1, 29)), Err(SigError::BadV(29)));
    }

    #[test]
    fn signature_shape_errors() {
        assert_eq!(parse_signature("zz"), Err(SigError::NotHex));
        assert_eq!(
            parse_signature(&"00".repeat(64)),
            Err(SigError::WrongLength(64))
        );
        assert_eq!(parse_signature(&sig_with(0, 27)), Err(SigError::ZeroRorS));
    }

    #[test]
    fn high_s_refused_half_n_accepted() {
        assert_eq!(parse_signature(&sig_with(0x80, 27)), Err(SigError::HighS));
        let mut b = vec![1u8; 32];
        b.extend_from_slice(&HALF_N);
        b.push(27);
        assert!(parse_signature(&hex::encode(&b)).is_ok());
        let last = b.len() - 2;
        b[last] += 1;
        assert_eq!(parse_signature(&hex::encode(&b)), Err(SigError::HighS));
    }

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
