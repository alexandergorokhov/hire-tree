//! Receipt binding: ties a NEAR AI Cloud receipt to one task and one delivery.
//!
//! The raw request bytes the worker sent to the model must contain exactly
//! one binding tag, and it must be this job's tag:
//!
//! ```text
//! HT1:<task_account>:<spec_hash as 64 lowercase hex>:<seq>
//! ```
//!
//! The deliverable hash is `sha256(response bytes)`.

use crate::receipt::sha256;

pub const TAG_PREFIX: &[u8] = b"HT1:";
/// Largest request the contract accepts (plan section 10).
pub const MAX_REQUEST_BYTES: usize = 8 * 1024;

const MIN_ACCOUNT_LEN: usize = 2;
const MAX_ACCOUNT_LEN: usize = 64;
const MAX_SEQ_DIGITS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingTag {
    pub task: String,
    pub spec_hash: [u8; 32],
    pub seq: u32,
}

impl BindingTag {
    pub fn render(&self) -> String {
        format!(
            "HT1:{}:{}:{}",
            self.task,
            hex::encode(self.spec_hash),
            self.seq
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    Empty,
    TooLarge {
        len: usize,
    },
    /// No well-formed tag in the request.
    Missing,
    /// The request carries more than one distinct tag, so one model call
    /// could be claimed by two tasks.
    MultipleTags,
    /// One tag, but for another task, spec or seq.
    Mismatch {
        found: String,
    },
    /// `sha256(response)` is not the claimed deliverable hash.
    DeliverableMismatch,
}

impl core::fmt::Display for BindingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BindingError::Empty => write!(f, "request is empty"),
            BindingError::TooLarge { len } => {
                write!(f, "request is {len} bytes, max {MAX_REQUEST_BYTES}")
            }
            BindingError::Missing => write!(f, "request has no HT1 binding tag"),
            BindingError::MultipleTags => write!(f, "request has more than one binding tag"),
            BindingError::Mismatch { found } => write!(f, "binding tag {found} is not this task"),
            BindingError::DeliverableMismatch => {
                write!(f, "deliverable hash is not sha256(response)")
            }
        }
    }
}

fn is_account_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'_'
}

fn is_lower_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
}

/// Parse one tag that starts at `bytes[0]` (which must be `HT1:`).
fn parse_at(bytes: &[u8]) -> Option<BindingTag> {
    let rest = bytes.strip_prefix(TAG_PREFIX)?;

    let acc_len = rest.iter().take_while(|b| is_account_byte(**b)).count();
    if !(MIN_ACCOUNT_LEN..=MAX_ACCOUNT_LEN).contains(&acc_len) {
        return None;
    }
    let task = core::str::from_utf8(&rest[..acc_len]).ok()?.to_string();
    let rest = rest[acc_len..].strip_prefix(b":")?;

    if rest.len() < 64 || !rest[..64].iter().all(|b| is_lower_hex(*b)) {
        return None;
    }
    let mut spec_hash = [0u8; 32];
    hex::decode_to_slice(&rest[..64], &mut spec_hash).ok()?;
    let rest = rest[64..].strip_prefix(b":")?;

    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > MAX_SEQ_DIGITS {
        return None;
    }
    // No leading zeros: one seq has one spelling.
    if digits > 1 && rest[0] == b'0' {
        return None;
    }
    // The tag must end here: "...:1" must not match inside "...:1x" or "...:12".
    if let Some(next) = rest.get(digits) {
        if next.is_ascii_alphanumeric() || *next == b'_' {
            return None;
        }
    }
    let seq: u32 = core::str::from_utf8(&rest[..digits]).ok()?.parse().ok()?;
    Some(BindingTag {
        task,
        spec_hash,
        seq,
    })
}

/// Every well-formed tag in the request, in order, duplicates removed.
pub fn parse_tags(req: &[u8]) -> Vec<BindingTag> {
    let mut out: Vec<BindingTag> = Vec::new();
    let mut i = 0;
    while i + TAG_PREFIX.len() <= req.len() {
        if &req[i..i + TAG_PREFIX.len()] == TAG_PREFIX {
            if let Some(tag) = parse_at(&req[i..]) {
                if !out.contains(&tag) {
                    out.push(tag);
                }
            }
        }
        i += 1;
    }
    out
}

/// Check that `req` is bound to exactly this task, spec and delivery seq.
pub fn check_binding(
    req: &[u8],
    task: &str,
    spec_hash: &[u8; 32],
    seq: u32,
) -> Result<(), BindingError> {
    if req.is_empty() {
        return Err(BindingError::Empty);
    }
    if req.len() > MAX_REQUEST_BYTES {
        return Err(BindingError::TooLarge { len: req.len() });
    }
    let tags = parse_tags(req);
    match tags.as_slice() {
        [] => Err(BindingError::Missing),
        [only] => {
            if only.task == task && &only.spec_hash == spec_hash && only.seq == seq {
                Ok(())
            } else {
                Err(BindingError::Mismatch {
                    found: only.render(),
                })
            }
        }
        _ => Err(BindingError::MultipleTags),
    }
}

/// The deliverable hash: `sha256(response)`.
pub fn deliverable_hash(response: &[u8]) -> [u8; 32] {
    sha256(response)
}

pub fn check_deliverable(response: &[u8], claimed: &[u8; 32]) -> Result<(), BindingError> {
    if &deliverable_hash(response) == claimed {
        Ok(())
    } else {
        Err(BindingError::DeliverableMismatch)
    }
}

/// Parse a 64-char hex string (lower or upper case, optional `0x`) into 32 bytes.
pub fn parse_hash32_hex(s: &str) -> Option<[u8; 32]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    hex::decode_to_slice(s, &mut out).ok()?;
    Some(out)
}

/// Parse a 40-char hex Ethereum-style address (optional `0x`) into 20 bytes.
pub fn parse_addr20_hex(s: &str) -> Option<[u8; 20]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    hex::decode_to_slice(s, &mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = "t1.job-1.hiretree.testnet";
    const SPEC: [u8; 32] = [0xab; 32];

    fn tag(seq: u32) -> String {
        BindingTag {
            task: TASK.into(),
            spec_hash: SPEC,
            seq,
        }
        .render()
    }

    fn req_with(body: &str) -> Vec<u8> {
        format!(r#"{{"model":"m","messages":[{{"role":"user","content":"{body}"}}]}}"#).into_bytes()
    }

    #[test]
    fn render_format() {
        assert_eq!(
            tag(3),
            format!("HT1:t1.job-1.hiretree.testnet:{}:3", "ab".repeat(32))
        );
    }

    #[test]
    fn tag_round_trips_through_parser() {
        let r = req_with(&format!("Translate this. {} Thanks", tag(7)));
        let tags = parse_tags(&r);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].task, TASK);
        assert_eq!(tags[0].spec_hash, SPEC);
        assert_eq!(tags[0].seq, 7);
    }

    #[test]
    fn bound_request_passes() {
        let r = req_with(&format!("do it {}", tag(1)));
        assert_eq!(check_binding(&r, TASK, &SPEC, 1), Ok(()));
    }

    #[test]
    fn request_without_tag_fails() {
        assert_eq!(
            check_binding(&req_with("no tag here"), TASK, &SPEC, 1),
            Err(BindingError::Missing)
        );
    }

    #[test]
    fn copied_receipt_from_other_task_fails() {
        let other = BindingTag {
            task: "t2.job-1.hiretree.testnet".into(),
            spec_hash: SPEC,
            seq: 1,
        }
        .render();
        let r = req_with(&other);
        assert!(matches!(
            check_binding(&r, TASK, &SPEC, 1),
            Err(BindingError::Mismatch { .. })
        ));
    }

    #[test]
    fn wrong_seq_or_spec_fails() {
        let r = req_with(&tag(1));
        assert!(check_binding(&r, TASK, &SPEC, 2).is_err());
        assert!(check_binding(&r, TASK, &[0xcd; 32], 1).is_err());
    }

    #[test]
    fn seq_prefix_does_not_match_longer_seq() {
        // The tag for seq 12 must not satisfy seq 1.
        let r = req_with(&tag(12));
        assert!(check_binding(&r, TASK, &SPEC, 1).is_err());
        assert_eq!(check_binding(&r, TASK, &SPEC, 12), Ok(()));
    }

    #[test]
    fn seq_followed_by_letter_is_not_a_tag() {
        let r = req_with(&format!("{}x", tag(1)));
        assert_eq!(
            check_binding(&r, TASK, &SPEC, 1),
            Err(BindingError::Missing)
        );
    }

    #[test]
    fn leading_zero_seq_is_not_a_tag() {
        let t = format!("HT1:{TASK}:{}:01", "ab".repeat(32));
        assert!(parse_tags(t.as_bytes()).is_empty());
    }

    #[test]
    fn seq_overflow_is_not_a_tag() {
        let t = format!("HT1:{TASK}:{}:4294967296", "ab".repeat(32));
        assert!(parse_tags(t.as_bytes()).is_empty());
        let t = format!("HT1:{TASK}:{}:4294967295", "ab".repeat(32));
        assert_eq!(parse_tags(t.as_bytes())[0].seq, u32::MAX);
    }

    #[test]
    fn uppercase_hex_is_not_a_tag() {
        let t = format!("HT1:{TASK}:{}:1", "AB".repeat(32));
        assert!(parse_tags(t.as_bytes()).is_empty());
    }

    #[test]
    fn two_distinct_tags_are_refused() {
        let other = BindingTag {
            task: "t2.job-1.hiretree.testnet".into(),
            spec_hash: SPEC,
            seq: 1,
        }
        .render();
        let r = req_with(&format!("{} and {}", tag(1), other));
        assert_eq!(
            check_binding(&r, TASK, &SPEC, 1),
            Err(BindingError::MultipleTags)
        );
    }

    #[test]
    fn same_tag_twice_is_fine() {
        let r = req_with(&format!("{} again {}", tag(1), tag(1)));
        assert_eq!(check_binding(&r, TASK, &SPEC, 1), Ok(()));
    }

    #[test]
    fn size_limits() {
        assert_eq!(check_binding(b"", TASK, &SPEC, 1), Err(BindingError::Empty));
        let mut big = req_with(&tag(1));
        big.resize(MAX_REQUEST_BYTES + 1, b' ');
        assert!(matches!(
            check_binding(&big, TASK, &SPEC, 1),
            Err(BindingError::TooLarge { .. })
        ));
        let mut max = req_with(&tag(1));
        max.resize(MAX_REQUEST_BYTES, b' ');
        assert_eq!(check_binding(&max, TASK, &SPEC, 1), Ok(()));
    }

    #[test]
    fn truncated_tags_do_not_panic() {
        let t = tag(1);
        for cut in 0..t.len() {
            let _ = parse_tags(&t.as_bytes()[..cut]);
        }
    }

    #[test]
    fn deliverable_hash_is_sha256_of_response() {
        // sha256("abc")
        let h = deliverable_hash(b"abc");
        assert_eq!(
            hex::encode(h),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(check_deliverable(b"abc", &h), Ok(()));
        assert_eq!(
            check_deliverable(b"abd", &h),
            Err(BindingError::DeliverableMismatch)
        );
    }

    #[test]
    fn hex_parsers() {
        assert_eq!(parse_hash32_hex(&"ab".repeat(32)), Some(SPEC));
        assert_eq!(
            parse_hash32_hex(&format!("0x{}", "AB".repeat(32))),
            Some(SPEC)
        );
        assert_eq!(parse_hash32_hex("abcd"), None);
        assert_eq!(parse_hash32_hex(&"zz".repeat(32)), None);
        assert_eq!(parse_addr20_hex(&"01".repeat(20)), Some([1; 20]));
        assert_eq!(parse_addr20_hex(&"01".repeat(21)), None);
    }
}
