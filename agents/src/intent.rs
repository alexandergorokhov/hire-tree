//! Signed job intents (intent-based settlement). A client signs "get this
//! spec done for `budget`; a lead may keep up to `max_lead_fee`" off-chain;
//! any lead agent fills it on the factory (`fill_intent`), and the lead fee
//! rises from 0 to the max over the intent's life, so leads compete on how
//! soon they fill.
//!
//! Demo keys: each account's intent key is derived from its account id and
//! `HT_INTENT_SEED` (default "local"). A real client signs with its own key
//! and registers the public half with `set_intent_key`.

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;

pub fn key_for(account: &str) -> SigningKey {
    let seed = std::env::var("HT_INTENT_SEED").unwrap_or_else(|_| "local".into());
    let h = ht_core::receipt::sha256(format!("hiretree-intent-key:{seed}:{account}").as_bytes());
    SigningKey::from_bytes(&h)
}

pub fn public_key(account: &str) -> String {
    format!(
        "ed25519:{}",
        bs58::encode(key_for(account).verifying_key().as_bytes()).into_string()
    )
}

/// The intent JSON, exactly as signed and as `fill_intent` takes it.
#[allow(clippy::too_many_arguments)]
pub fn build(
    factory: &str,
    client: &str,
    budget: u128,
    spec_hash: &str,
    window_secs: u64,
    max_lead_fee: u128,
    starts_ns: u64,
    ends_ns: u64,
    nonce: u128,
) -> String {
    json!({ "factory": factory, "client": client, "budget": budget.to_string(),
            "spec_hash": spec_hash, "window_secs": window_secs, "max_lead_fee": max_lead_fee.to_string(),
            "starts_ns": starts_ns.to_string(), "ends_ns": ends_ns.to_string(), "nonce": nonce.to_string() })
    .to_string()
}

/// The same intent, fillable only by `leads`. Whoever fills an intent
/// becomes the job's manager and spends its budget, so a client that does
/// not want to trust the first filler names the leads it does trust.
pub fn name_leads(intent: &str, leads: &[&str]) -> String {
    let mut v: serde_json::Value = serde_json::from_str(intent).unwrap_or_default();
    v["leads"] = json!(leads);
    v.to_string()
}

/// Base64 ed25519 signature over "hiretree-intent:" + the intent JSON.
pub fn sign(account: &str, intent: &str) -> String {
    let mut m = b"hiretree-intent:".to_vec();
    m.extend_from_slice(intent.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(key_for(account).sign(&m).to_bytes())
}

/// The fee a lead gets if it fills at `now` (mirrors the factory's `dutch_fee`).
pub fn dutch_fee(max: u128, start: u64, end: u64, now: u64) -> u128 {
    let (now, start, end) = (now as u128, start as u128, end as u128);
    if now <= start || end <= start {
        return 0;
    }
    if now >= end {
        return max;
    }
    let (e, s) = (now - start, end - start);
    max / s * e + max % s * e / s
}

/// When a lead with this reservation fee is willing to fill.
pub fn fill_time(reservation: u128, max: u128, start: u64, end: u64) -> u64 {
    if reservation >= max {
        return end;
    }
    start + ((end - start) as u128 * reservation / max.max(1)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_intent_signature_verifies_and_binds_every_field() {
        let i = build("hub.x", "c.x", 70, &"ab".repeat(32), 3600, 8, 1, 601, 3);
        let sig = base64::engine::general_purpose::STANDARD
            .decode(sign("c.x", &i))
            .unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&sig).unwrap();
        let vk = key_for("c.x").verifying_key();
        let mut m = b"hiretree-intent:".to_vec();
        m.extend_from_slice(i.as_bytes());
        assert!(vk.verify_strict(&m, &sig).is_ok());
        let other = build("hub.x", "c.x", 71, &"ab".repeat(32), 3600, 8, 1, 601, 3);
        let mut m2 = b"hiretree-intent:".to_vec();
        m2.extend_from_slice(other.as_bytes());
        assert!(vk.verify_strict(&m2, &sig).is_err());
        assert!(public_key("c.x").starts_with("ed25519:"));
    }

    #[test]
    fn an_intent_can_name_its_leads() {
        let i = build("hub.x", "c.x", 70, &"ab".repeat(32), 3600, 8, 1, 601, 3);
        let named = name_leads(&i, &["lead.x"]);
        let v: serde_json::Value = serde_json::from_str(&named).unwrap();
        assert_eq!(v["leads"], json!(["lead.x"]));
        assert_eq!(v["budget"], "70");
        // A different text: a signature for the open intent does not cover it.
        assert_ne!(sign("c.x", &i), sign("c.x", &named));
    }

    #[test]
    fn a_cheaper_lead_fills_sooner() {
        assert_eq!(fill_time(2, 8, 0, 800), 200);
        assert_eq!(fill_time(6, 8, 0, 800), 600);
        assert_eq!(fill_time(9, 8, 0, 800), 800);
        assert_eq!(dutch_fee(8, 0, 800, 200), 2);
    }
}
