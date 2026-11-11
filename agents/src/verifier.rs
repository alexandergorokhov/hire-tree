//! NEAR Intents: the official Verifier contract (`intents.near` on mainnet)
//! as the place where a worker's pay lands and is swapped.
//!
//! The local node runs the Verifier's own release wasm (defuse 0.2.10 and
//! wNEAR from github.com/near/intents, vendored in `vendor/near-intents/`).
//! A job pays a worker with `ft_transfer_call(verifier, pay, msg = worker)`
//! (`set_payout_via` on the job), so the pay is the worker's balance in the
//! Verifier. The worker then signs a NEP-413 `token_diff` intent ("give X
//! fUSD, want Y wNEAR"), a solver signs the opposite one, and anyone submits
//! both with `execute_intents`: the swap is atomic or nothing moves. The
//! worker can `ft_withdraw` either token out.

use crate::chain::{Acc, Chain};
use near_api::signer::NEP413Payload;
use near_api::types::NearToken;
use serde_json::{json, Value};

fn vendored(name: &str) -> Vec<u8> {
    let dir = std::env::var("INTENTS_WASM_DIR")
        .unwrap_or_else(|_| format!("{}/../vendor/near-intents", env!("CARGO_MANIFEST_DIR")));
    let path = format!("{dir}/{name}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The NEAR Intents token id of a NEP-141 token.
pub fn nep141(token: &str) -> String {
    format!("nep141:{token}")
}

/// The Verifier and its wNEAR, deployed on this chain.
pub struct Verifier {
    pub id: near_api::types::AccountId,
    pub wnear: near_api::types::AccountId,
}

/// Deploy wNEAR and the Verifier (no fee, no roles), or find them.
pub async fn deploy(c: &Chain) -> Result<Verifier, String> {
    let wnear = c.account_with("wnear", NearToken::from_near(5)).await;
    if !c.has_code(&wnear.id).await {
        near_api::Contract::deploy(wnear.id.clone())
            .use_code(vendored("wnear.wasm"))
            .with_init_call("new", json!({}))
            .map_err(|e| format!("{e:?}"))?
            .with_signer(wnear.signer.clone())
            .send_to(&c.net)
            .await
            .map_err(|e| format!("deploy wnear: {e:?}"))?
            .assert_success();
    }
    let v = c.account_with("intents", NearToken::from_near(20)).await;
    if !c.has_code(&v.id).await {
        near_api::Contract::deploy(v.id.clone())
            .use_code(vendored("defuse-0.2.10.wasm"))
            .with_init_call(
                "new",
                json!({ "config": {
                    "wnear_id": wnear.id,
                    "fees": { "fee": 0, "fee_collector": v.id },
                    "roles": { "super_admins": [], "admins": {}, "grantees": {} }
                }}),
            )
            .map_err(|e| format!("{e:?}"))?
            .with_signer(v.signer.clone())
            .send_to(&c.net)
            .await
            .map_err(|e| format!("deploy verifier: {e:?}"))?
            .assert_success();
    }
    Ok(Verifier {
        id: v.id,
        wnear: wnear.id,
    })
}

impl Verifier {
    /// Let `who`'s own key sign intents for it in the Verifier.
    pub async fn add_key(&self, c: &Chain, who: &Acc) -> Result<(), String> {
        let pk = who
            .signer
            .get_public_key()
            .await
            .map_err(|e| format!("{e:?}"))?;
        c.call(
            who,
            &self.id,
            "add_public_key",
            json!({ "public_key": pk.to_string() }),
            NearToken::from_yoctonear(1),
        )
        .await
        .map(|_| ())
    }

    /// `who`'s balance of `token` (a NEP-141 account) inside the Verifier.
    pub async fn balance(&self, c: &Chain, who: &str, token: &str) -> u128 {
        c.view(
            &self.id,
            "mt_balance_of",
            json!({ "account_id": who, "token_id": nep141(token) }),
        )
        .await
        .ok()
        .as_ref()
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
    }

    /// A NEP-413 signed `token_diff` intent for the Verifier: `diff` maps
    /// token ids to signed amounts (negative = give). `nonce` must be fresh
    /// per signer; `deadline` is RFC 3339.
    pub async fn sign_diff(
        &self,
        who: &Acc,
        diff: Value,
        nonce: [u8; 32],
        deadline: &str,
    ) -> Result<Value, String> {
        let message = json!({
            "signer_id": who.id,
            "deadline": deadline,
            "intents": [{ "intent": "token_diff", "diff": diff }],
        })
        .to_string();
        let payload = NEP413Payload {
            message,
            nonce,
            recipient: self.id.to_string(),
            callback_url: None,
        };
        let pk = who
            .signer
            .get_public_key()
            .await
            .map_err(|e| format!("{e:?}"))?;
        let sig = who
            .signer
            .sign_message_nep413(who.id.clone(), pk, &payload)
            .await
            .map_err(|e| format!("{e:?}"))?;
        use base64::Engine;
        Ok(json!({
            "standard": "nep413",
            "payload": {
                "message": payload.message,
                "nonce": base64::engine::general_purpose::STANDARD.encode(nonce),
                "recipient": payload.recipient,
            },
            "public_key": pk.to_string(),
            "signature": sig.to_string(),
        }))
    }

    /// Submit signed intents; all of them settle or none.
    pub async fn execute(&self, c: &Chain, by: &Acc, signed: Vec<Value>) -> Result<Value, String> {
        c.call(
            by,
            &self.id,
            "execute_intents",
            json!({ "signed": signed }),
            NearToken::from_yoctonear(0),
        )
        .await
    }

    /// Take `amount` of `token` out of the Verifier to `who`'s own account.
    pub async fn withdraw(
        &self,
        c: &Chain,
        who: &Acc,
        token: &str,
        amount: u128,
    ) -> Result<Value, String> {
        c.call(
            who,
            &self.id,
            "ft_withdraw",
            json!({ "token": token, "receiver_id": who.id, "amount": amount.to_string() }),
            NearToken::from_yoctonear(1),
        )
        .await
    }
}

/// A deadline `secs` after the chain's current block time, RFC 3339.
pub async fn deadline_in(c: &Chain, secs: u64) -> String {
    let ns = c.block_time_ns().await + secs * 1_000_000_000;
    let s = (ns / 1_000_000_000) as i64;
    // Days from civil: 1970-01-01 + s seconds, formatted without a date crate.
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_403), (2025, 11, 11));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn token_ids() {
        assert_eq!(nep141("usdc.test.near"), "nep141:usdc.test.near");
    }
}
