//! Model calls. Every call returns the raw response bytes and the NEAR AI
//! Cloud-style signature over `model:sha256(request):sha256(response)`, so
//! any answer an agent acts on can be checked later, and the ones that go
//! on-chain (worker receipts, judge answers) are checked by the contract.
//!
//! - `Live`: NEAR AI Cloud (`NEARAI_API_KEY`); the only paid mode.
//! - `Record`: Live, and every call is saved to a cassette file.
//! - `Replay`: answers from a cassette, offline and free; the signatures in
//!   it are the original NEAR AI ones.
//! - `Mock`: a scripted model that signs with a test key (tests only).

use ht_core::receipt::{eip191_digest, eth_address, sha256_hex, signed_text};
use secp256k1::{ecdsa::RecoverableSignature, ecdsa::RecoveryId, Message, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;

pub const NEARAI_BASE: &str = "https://cloud-api.near.ai";

#[derive(Clone, Debug)]
pub struct Reply {
    pub body: Vec<u8>,
    /// 65-byte r||s||v hex.
    pub sig: String,
}

impl Reply {
    /// `choices[0].message.content`, or "" if absent.
    pub fn content(&self) -> String {
        serde_json::from_slice::<Value>(&self.body)
            .ok()
            .and_then(|v| {
                v["choices"][0]["message"]["content"]
                    .as_str()
                    .map(String::from)
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub req: String,
    pub resp: String,
    pub sig: String,
}

/// A recorded run: every model call with its signature.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cassette {
    pub model: String,
    /// 20-byte hex address that signed every entry.
    pub signer: String,
    /// Where `signer` came from (e.g. NEAR AI's attestation report URL).
    pub signer_source: String,
    pub entries: Vec<Entry>,
}

impl Cassette {
    pub fn load(path: &str) -> Result<Cassette, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        serde_json::from_str(&s).map_err(|e| format!("{path}: {e}"))
    }
    pub fn save(&self, path: &str) -> Result<(), String> {
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
            .map_err(|e| format!("{path}: {e}"))
    }
    /// Re-check every entry's signature against `signer`. Returns the count.
    pub fn verify(&self) -> Result<usize, String> {
        for (i, e) in self.entries.iter().enumerate() {
            let addr = recover(&self.model, e.req.as_bytes(), e.resp.as_bytes(), &e.sig)
                .map_err(|x| format!("entry {i}: {x}"))?;
            if !addr.eq_ignore_ascii_case(self.signer.trim_start_matches("0x")) {
                return Err(format!("entry {i}: signed by {addr}, not {}", self.signer));
            }
        }
        Ok(self.entries.len())
    }
}

/// The address that signed `model:sha256(req):sha256(resp)` (EIP-191).
pub fn recover(model: &str, req: &[u8], resp: &[u8], sig_hex: &str) -> Result<String, String> {
    let sig = ht_core::receipt::parse_signature(sig_hex).map_err(|e| e.to_string())?;
    let text = signed_text(model, req, resp);
    let digest = eip191_digest(text.as_bytes());
    let rs =
        RecoverableSignature::from_compact(&sig.rs, RecoveryId::from_i32(sig.v as i32).unwrap())
            .map_err(|e| e.to_string())?;
    let pk = Secp256k1::new()
        .recover_ecdsa(&Message::from_slice(&digest).unwrap(), &rs)
        .map_err(|e| e.to_string())?;
    let pk = pk.serialize_uncompressed();
    Ok(hex::encode(eth_address(&pk[1..].try_into().unwrap())))
}

pub fn sign_text(k: &SecretKey, text: &str) -> String {
    let (rid, rs) = Secp256k1::new()
        .sign_ecdsa_recoverable(
            &Message::from_slice(&eip191_digest(text.as_bytes())).unwrap(),
            k,
        )
        .serialize_compact();
    let mut out = rs.to_vec();
    out.push(27 + rid.to_i32() as u8);
    hex::encode(out)
}

pub fn addr_of(k: &SecretKey) -> String {
    let pk = k.public_key(&Secp256k1::new()).serialize_uncompressed();
    hex::encode(eth_address(&pk[1..].try_into().unwrap()))
}

/// NEAR AI Cloud client (OpenAI-compatible chat + signature endpoint).
pub struct NearAi {
    pub base: String,
    pub key: String,
    pub model: String,
    http: reqwest::Client,
}

impl NearAi {
    pub fn from_env(model: &str) -> Result<NearAi, String> {
        let key = std::env::var("NEARAI_API_KEY").map_err(|_| "set NEARAI_API_KEY".to_string())?;
        Ok(NearAi {
            base: std::env::var("NEARAI_BASE").unwrap_or_else(|_| NEARAI_BASE.into()),
            key,
            model: model.into(),
            http: reqwest::Client::new(),
        })
    }

    /// The model's signing address from NEAR AI's attestation report (no
    /// tokens spent). The report itself is what proves the key lives in a TEE.
    pub async fn signing_address(&self) -> Result<String, String> {
        if let Ok(a) = std::env::var("HT_SIGNER") {
            return Ok(a.trim_start_matches("0x").to_lowercase());
        }
        let v: Value = self
            .http
            .get(format!("{}/v1/attestation/report", self.base))
            .query(&[("model", self.model.as_str()), ("signing_algo", "ecdsa")])
            .bearer_auth(&self.key)
            .send()
            .await
            .map_err(|e| format!("attestation: {e}"))?
            .json()
            .await
            .map_err(|e| format!("attestation json: {e}"))?;
        let a = v["signing_address"]
            .as_str()
            .or_else(|| v["all_attestations"][0]["signing_address"].as_str())
            .ok_or(format!(
                "attestation: no signing_address in {}",
                v.to_string().chars().take(300).collect::<String>()
            ))?;
        Ok(a.trim_start_matches("0x").to_lowercase())
    }

    /// Model ids NEAR AI serves.
    pub async fn models(&self) -> Result<Vec<String>, String> {
        let v: Value = self
            .http
            .get(format!("{}/v1/models", self.base))
            .bearer_auth(&self.key)
            .send()
            .await
            .map_err(|e| format!("models: {e}"))?
            .json()
            .await
            .map_err(|e| format!("models json: {e}"))?;
        Ok(v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str().map(String::from))
            .collect())
    }

    /// POST the exact body bytes, then fetch the signature for that chat.
    /// Fails if NEAR AI's signed text is not `model:sha256(our request
    /// bytes):sha256(the response bytes we got)`: that is the format the
    /// contract checks, so a mismatch is a no-go.
    pub async fn chat(&self, body: &[u8]) -> Result<Reply, String> {
        let r = self
            .http
            .post(format!("{}/v1/chat/completions", self.base))
            .bearer_auth(&self.key)
            .header("content-type", "application/json")
            .body(body.to_vec())
            .send()
            .await
            .map_err(|e| format!("chat: {e}"))?;
        let status = r.status();
        let resp = r
            .bytes()
            .await
            .map_err(|e| format!("chat body: {e}"))?
            .to_vec();
        if !status.is_success() {
            return Err(format!(
                "chat: HTTP {status}: {}",
                String::from_utf8_lossy(&resp)
            ));
        }
        let id = serde_json::from_slice::<Value>(&resp)
            .ok()
            .and_then(|v| v["id"].as_str().map(String::from))
            .ok_or("chat: no id in response")?;
        // The signature can lag the answer by a moment: retry briefly.
        let mut s = Value::Null;
        for attempt in 0..5 {
            let r = self
                .http
                .get(format!("{}/v1/signature/{id}", self.base))
                .query(&[("model", self.model.as_str()), ("signing_algo", "ecdsa")])
                .bearer_auth(&self.key)
                .send()
                .await
                .map_err(|e| format!("signature: {e}"))?;
            if r.status().is_success() {
                s = r.json().await.map_err(|e| format!("signature json: {e}"))?;
                if s["signature"].is_string() {
                    break;
                }
            } else if attempt == 4 {
                return Err(format!("signature: HTTP {} for chat {id}", r.status()));
            }
            tokio::time::sleep(std::time::Duration::from_millis(800 * (attempt + 1))).await;
        }
        let text = s["text"].as_str().unwrap_or_default();
        let want = signed_text(&self.model, body, &resp);
        if text != want {
            return Err(format!(
                "NO-GO: NEAR AI signed text differs from what the contract checks.\n  signed: {text}\n  ours:   {want}\n  (req sha256 {}, resp sha256 {})",
                sha256_hex(body),
                sha256_hex(&resp)
            ));
        }
        let sig = s["signature"]
            .as_str()
            .ok_or("signature: missing")?
            .trim_start_matches("0x")
            .to_string();
        let addr = recover(&self.model, body, &resp, &sig)?;
        let claimed = s["signing_address"]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches("0x");
        if !addr.eq_ignore_ascii_case(claimed) {
            return Err(format!(
                "NO-GO: signature recovers to {addr}, NEAR AI says {claimed}"
            ));
        }
        Ok(Reply { body: resp, sig })
    }
}

/// Scripted model for tests: `script` maps the request JSON to the reply
/// text; the reply is signed with a test key, exactly like NEAR AI.
pub struct Mock {
    pub key: SecretKey,
    pub model: String,
    pub script: Box<dyn Fn(&Value) -> String + Send + Sync>,
    n: Mutex<u32>,
}

impl Mock {
    pub fn new(model: &str, script: Box<dyn Fn(&Value) -> String + Send + Sync>) -> Mock {
        Mock {
            key: SecretKey::from_slice(&[0x42; 32]).unwrap(),
            model: model.into(),
            script,
            n: Mutex::new(0),
        }
    }
    fn chat(&self, body: &[u8]) -> Reply {
        let req: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let content = (self.script)(&req);
        let mut n = self.n.lock().unwrap();
        *n += 1;
        let resp = json!({ "id": format!("mock-{n}"), "object": "chat.completion", "model": self.model,
                           "choices": [{ "index": 0, "message": { "role": "assistant", "content": content } }] })
        .to_string()
        .into_bytes();
        let sig = sign_text(&self.key, &signed_text(&self.model, body, &resp));
        Reply { body: resp, sig }
    }
}

/// Where a recording's answers come from.
pub enum Source {
    Live(NearAi),
    Mock(Mock),
}

impl Source {
    fn model(&self) -> String {
        match self {
            Source::Live(n) => n.model.clone(),
            Source::Mock(m) => m.model.clone(),
        }
    }
    async fn chat(&self, body: &[u8]) -> Result<Reply, String> {
        match self {
            Source::Live(n) => n.chat(body).await,
            Source::Mock(m) => Ok(m.chat(body)),
        }
    }
}

pub enum Llm {
    Mock(Mock),
    Live(NearAi),
    Record(Source, Mutex<Cassette>, String),
    Replay(Cassette),
}

impl Llm {
    pub fn model(&self) -> String {
        match self {
            Llm::Mock(m) => m.model.clone(),
            Llm::Live(n) => n.model.clone(),
            Llm::Record(src, _, _) => src.model(),
            Llm::Replay(c) => c.model.clone(),
        }
    }

    /// The address the contracts must pin as signer.
    pub fn signer(&self) -> Option<String> {
        match self {
            Llm::Mock(m) => Some(addr_of(&m.key)),
            Llm::Replay(c) => Some(c.signer.clone()),
            Llm::Record(_, c, _) => {
                let s = c.lock().unwrap().signer.clone();
                (!s.is_empty()).then_some(s)
            }
            Llm::Live(_) => None,
        }
    }

    pub async fn chat(&self, body: &[u8]) -> Result<Reply, String> {
        match self {
            Llm::Mock(m) => Ok(m.chat(body)),
            Llm::Live(n) => n.chat(body).await,
            Llm::Record(src, c, path) => {
                let r = src.chat(body).await?;
                let mut c = c.lock().unwrap();
                if c.signer.is_empty() {
                    c.signer = recover(&src.model(), body, &r.body, &r.sig)?;
                }
                c.entries.push(Entry {
                    req: String::from_utf8_lossy(body).into(),
                    resp: String::from_utf8_lossy(&r.body).into(),
                    sig: r.sig.clone(),
                });
                c.save(path)?;
                Ok(r)
            }
            Llm::Replay(c) => {
                let want = String::from_utf8_lossy(body);
                c.entries
                    .iter()
                    .find(|e| e.req == want)
                    .map(|e| Reply {
                        body: e.resp.clone().into_bytes(),
                        sig: e.sig.clone(),
                    })
                    .ok_or_else(|| {
                        format!(
                            "replay: no recorded answer for request sha256 {} (the run diverged from the recording)",
                            sha256_hex(body)
                        )
                    })
            }
        }
    }
}

/// Chat body bytes for a system + user prompt (what every agent sends).
pub fn chat_body(model: &str, system: &str, user: &str, max_tokens: u32) -> Vec<u8> {
    json!({ "model": model, "temperature": 0, "max_tokens": max_tokens,
            "messages": [ { "role": "system", "content": system }, { "role": "user", "content": user } ] })
    .to_string()
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cassette_verifies_and_catches_tampering() {
        let llm = Llm::Mock(Mock::new("m", Box::new(|_| "hi".into())));
        let body = chat_body("m", "s", "u", 4);
        let r = llm.chat(&body).await.unwrap();
        assert_eq!(r.content(), "hi");
        let mut c = Cassette {
            model: "m".into(),
            signer: llm.signer().unwrap(),
            signer_source: "test".into(),
            entries: vec![Entry {
                req: String::from_utf8(body).unwrap(),
                resp: String::from_utf8(r.body).unwrap(),
                sig: r.sig,
            }],
        };
        assert_eq!(c.verify().unwrap(), 1);
        c.entries[0].resp = c.entries[0].resp.replace("hi", "ho");
        assert!(c.verify().is_err(), "an edited answer must fail");
    }

    #[tokio::test]
    async fn replay_refuses_a_request_it_never_saw() {
        let c = Llm::Replay(Cassette {
            model: "m".into(),
            ..Default::default()
        });
        let e = c.chat(b"{}").await.unwrap_err();
        assert!(e.contains("diverged"));
    }
}
