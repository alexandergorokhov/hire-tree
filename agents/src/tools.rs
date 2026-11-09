//! The tool surface: every on-chain action an agent takes goes through one
//! of these, and the MCP server (`hiretree mcp`) exposes the same list, so
//! any MCP-speaking agent can join a tree with the same powers and limits.

use crate::chain::{Acc, Chain};
use near_api::types::{AccountId, NearToken};
use serde_json::{json, Value};

/// (name, description, JSON schema of the arguments).
/// The optional extra bond a bidder stakes on top of the slot's (yoctoNEAR).
/// The most extra bond a bidder may stake (5 NEAR, the contract's MAX_BOND).
pub const MAX_STAKE: u128 = 5_000_000_000_000_000_000_000_000;

fn stake(a: &Value) -> Result<u128, String> {
    let x: u128 = match a["stake"].as_str() {
        None => return Ok(0),
        Some(x) => x
            .parse()
            .map_err(|_| "stake must be yoctoNEAR as a string".to_string())?,
    };
    if x > MAX_STAKE {
        return Err("stake is at most 5 NEAR".into());
    }
    Ok(x)
}

/// The deposit a bid attaches: storage plus the slot's bond plus the stake.
fn bid_deposit(storage_millinear: u128, bond: u128, stake: u128) -> Result<NearToken, String> {
    NearToken::from_millinear(storage_millinear)
        .as_yoctonear()
        .checked_add(bond)
        .and_then(|x| x.checked_add(stake))
        .map(NearToken::from_yoctonear)
        .ok_or_else(|| "bid deposit overflows".to_string())
}

pub fn list() -> Vec<(&'static str, &'static str, Value)> {
    let s = |props: Value, req: &[&str]| json!({ "type": "object", "properties": props, "required": req });
    let str_ = json!({ "type": "string" });
    vec![
        ("post_job", "Client: post and fund a root job through the factory (needs 0.06 NEAR prepaid with prepay_post).",
         s(json!({ "budget": str_, "manager": str_, "task": str_, "window_secs": { "type": "integer" }, "lead_fee": str_, "treasury": str_ }), &["budget", "manager", "task", "window_secs"])),
        ("deposit", "Client: keep `amount` fUSD at the factory for signed job intents (an ft_transfer_call with msg \"deposit\").",
         s(json!({ "amount": str_ }), &["amount"])),
        ("sign_intent", "Client: register your intent key (once) and sign a job intent off-chain: get `task` done for `budget`; a lead may keep up to `max_lead_fee`, rising from 0 now to the max after `secs`. Returns {intent, signature} for any lead to fill. No transaction but the key registration.",
         s(json!({ "budget": str_, "task": str_, "window_secs": { "type": "integer" }, "max_lead_fee": str_, "secs": { "type": "integer" }, "nonce": str_ }), &["budget", "task", "window_secs", "max_lead_fee", "secs", "nonce"])),
        ("quote_intent", "Anyone: the lead fee a fill would get right now, and whether the intent is still open.",
         s(json!({ "intent": str_ }), &["intent"])),
        ("fill_intent", "Lead: fill a signed job intent. The factory checks the client's signature, spends its deposit and posts the job with you as manager and the current lead fee (it rises over time: fill early to win, late to earn more).",
         s(json!({ "intent": str_, "signature": str_, "treasury": str_ }), &["intent", "signature"])),
        ("hire", "Manager (or a sub-job's own worker, paying out of its own pay): hire a worker as a new sub-account t<n>.<job> holding exactly `amount` (+ optional `sub` budget). Attaches 0.035 NEAR.",
         s(json!({ "job": str_, "worker": str_, "amount": str_, "task": str_, "sub": str_ }), &["job", "worker", "amount", "task"])),
        ("open_bids", "Manager (or a sub-job's own worker, as a contractor): put a subtask out for bids on the job account (max = the most it will pay, fUSD units as a string; secs 10..3600; optional bond = yoctoNEAR each bidder stakes, at most 5 NEAR: losers get it back at the award, the winner when paid, the buyer keeps it on junk). Returns the slot number.",
         s(json!({ "job": str_, "task": str_, "max": str_, "secs": { "type": "integer" }, "bond": str_, "sealed": { "type": "boolean" } }), &["job", "task", "max", "secs"])),
        ("bid_sealed", "Worker, sealed slot: commit to a price without showing it (the tool hashes `price:salt:you`; keep the salt). Attaches 0.002 NEAR for storage plus the slot's bond, plus an optional `stake` (yoctoNEAR) on top: a bigger bond vouches for a bidder with no record yet.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "price": str_, "salt": str_, "stake": str_ }), &["job", "slot", "price", "salt"])),
        ("reveal", "Worker, sealed slot, after bidding closes: reveal the committed price with its salt. An unrevealed bid cannot win and loses its bond.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "price": str_, "salt": str_ }), &["job", "slot", "price", "salt"])),
        ("bid", "Worker: bid `price` (at most the slot's max) on an open slot; a second bid replaces the first. Attaches 0.001 NEAR (the bid's storage) plus the slot's bond, if any, plus an optional `stake` (yoctoNEAR) on top: a bigger bond vouches for a bidder with no record yet.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "price": str_, "stake": str_ }), &["job", "slot", "price"])),
        ("get_bids", "Anyone: a bid slot with its bids and award.", s(json!({ "job": str_, "slot": { "type": "integer" } }), &["job", "slot"])),
        ("award", "Manager: after bidding closes, hire one bidder at its own bid (a new sub-account). Attaches 0.035 NEAR.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "worker": str_, "sub": str_ }), &["job", "slot", "worker"])),
        ("deliver", "Worker: deliver the response bytes (base64) with an optional NEAR AI receipt (req base64, sig hex, model).",
         s(json!({ "job": str_, "resp": str_, "req": str_, "sig": str_, "model": str_ }), &["job", "resp"])),
        ("check_request", "Anyone: the exact judge request body for a delivery (base64).",
         s(json!({ "job": str_, "panel": { "type": "integer" }, "task": str_, "deliverable": str_ }), &["job", "panel", "task", "deliverable"])),
        ("submit_check", "Anyone: submit a NEAR AI judge answer (base64) and its signature for an open check round.",
         s(json!({ "job": str_, "panel": { "type": "integer" }, "task": str_, "deliverable": str_, "answer": str_, "sig": str_ }), &["job", "panel", "task", "deliverable", "answer", "sig"])),
        ("finalize_check", "Anyone: close a delivery's check round after 5 minutes.", s(json!({ "job": str_ }), &["job"])),
        ("quote_note", "Anyone: the price of a job's note for the lead's treasury, or null.", s(json!({ "job": str_ }), &["job"])),
        ("buy_note", "Treasury: buy a job's note at its quote; the worker is paid now.", s(json!({ "job": str_, "price": str_ }), &["job", "price"])),
        ("dispute", "Manager: dispute a sub-job's delivery (resp base64 = the delivered bytes).", s(json!({ "job": str_, "resp": str_ }), &["job", "resp"])),
        ("finalize_dispute", "Anyone: decide a dispute after its rounds close.", s(json!({ "job": str_ }), &["job"])),
        ("approve", "Client (root) or manager (sub-job): approve, so settle may run before the window ends.", s(json!({ "job": str_ }), &["job"])),
        ("fire", "Manager (sub-job): after the signed check failed the delivery, end the job now: the worker is not paid, its bond goes to you, and the pay comes back so you can hire the runner-up.", s(json!({ "job": str_ }), &["job"])),
        ("settle", "Anyone: settle a job (pay, return the rest, delete at zero).", s(json!({ "job": str_ }), &["job"])),
        ("get_job", "Anyone: a job's state.", s(json!({ "job": str_ }), &["job"])),
        ("get_record", "Anyone: a worker's track record kept by the factory.", s(json!({ "account": str_ }), &["account"])),
        ("join_pool", "Client: put `amount` into a collective-purchase pool; the pool keeps at most what its target still needs.",
         s(json!({ "pool": str_, "amount": str_ }), &["pool", "amount"])),
        ("launch_pool", "Anyone: the target is raised; the pool posts its job through the factory as the client. Attaches 0.06 NEAR.",
         s(json!({ "pool": str_ }), &["pool"])),
        ("bind_pool", "Anyone: tell the pool which factory job number is its own (checked against the factory).",
         s(json!({ "pool": str_, "n": { "type": "integer" } }), &["pool", "n"])),
        ("vote_approve", "Pool member: vote to approve the pool's job early; more than half the stake approves it.",
         s(json!({ "pool": str_ }), &["pool"])),
        ("finish_pool", "Anyone: after the job deleted itself (or the pool failed), the pool counts what came back.",
         s(json!({ "pool": str_ }), &["pool"])),
        ("claim_pool", "Pool member: take your pro-rata share of what came back.", s(json!({ "pool": str_ }), &["pool"])),
        ("get_pool", "Anyone: a pool's state.", s(json!({ "pool": str_ }), &["pool"])),
    ]
}

pub struct Tools<'a> {
    pub chain: &'a Chain,
}

fn id(v: &Value, k: &str) -> Result<AccountId, String> {
    v[k].as_str()
        .ok_or(format!("missing {k}"))?
        .parse()
        .map_err(|e| format!("{k}: {e}"))
}

fn s<'v>(v: &'v Value, k: &str) -> Result<&'v str, String> {
    v[k].as_str().ok_or(format!("missing {k}"))
}

impl Tools<'_> {
    /// Run tool `name` as account `who`.
    pub async fn call(&self, who: &Acc, name: &str, a: &Value) -> Result<Value, String> {
        let c = self.chain;
        // Job ids must be jobs of this factory: the tools never send money
        // or NEAR to an arbitrary contract.
        if a.get("job").is_some() {
            let j = s(a, "job")?;
            if !j.ends_with(&format!(".{}", c.hub))
                || !j
                    .split('.')
                    .next()
                    .is_some_and(|p| p.starts_with("job-") || p.starts_with('t'))
            {
                return Err(format!("{j} is not a job of {}", c.hub));
            }
        }
        if a.get("pool").is_some() {
            let p = s(a, "pool")?;
            if !p.starts_with("pool-")
                || !p.ends_with(&format!(".{}", c.funder.id))
                || p.matches('.').count() != c.funder.id.as_str().matches('.').count() + 1
            {
                return Err(format!("{p} is not a pool of {}", c.funder.id));
            }
        }
        let zero = NearToken::from_yoctonear(0);
        match name {
            "join_pool" => {
                let amount: u128 = s(a, "amount")?.parse().map_err(|_| "amount")?;
                c.send(who, &id(a, "pool")?, amount, Some("join")).await
            }
            "launch_pool" => {
                c.call(who, &id(a, "pool")?, "launch", json!({}), NearToken::from_millinear(60))
                    .await
            }
            "bind_pool" => {
                c.call(who, &id(a, "pool")?, "bind", json!({ "n": a["n"].as_u64().ok_or("missing n")? }), zero)
                    .await
            }
            "vote_approve" | "claim_pool" | "finish_pool" => {
                let m = match name {
                    "claim_pool" => "claim",
                    "finish_pool" => "finish",
                    m => m,
                };
                c.call(who, &id(a, "pool")?, m, json!({}), zero).await
            }
            "get_pool" => c.view(&id(a, "pool")?, "get_pool", json!({})).await,
            "deposit" => {
                let amount: u128 = s(a, "amount")?.parse().map_err(|_| "amount")?;
                c.send(who, &c.hub, amount, Some("deposit")).await
            }
            "sign_intent" => {
                let me = who.id.to_string();
                let key = crate::intent::public_key(&me);
                let have = c.view(&c.hub, "get_intent_key", json!({ "account_id": me })).await?;
                if have.as_str() != Some(key.as_str()) {
                    c.call(who, &c.hub, "set_intent_key", json!({ "public_key": key }), zero).await?;
                }
                let now = c.block_time_ns().await;
                let secs = a["secs"].as_u64().ok_or("missing secs")?.clamp(10, 86_400);
                let intent = crate::intent::build(
                    c.hub.as_str(),
                    &me,
                    s(a, "budget")?.parse().map_err(|_| "budget")?,
                    &ht_core::receipt::sha256_hex(s(a, "task")?.as_bytes()),
                    a["window_secs"].as_u64().ok_or("missing window_secs")?,
                    s(a, "max_lead_fee")?.parse().map_err(|_| "max_lead_fee")?,
                    now,
                    now + secs * 1_000_000_000,
                    s(a, "nonce")?.parse().map_err(|_| "nonce")?,
                );
                Ok(json!({ "signature": crate::intent::sign(&me, &intent), "intent": intent }))
            }
            "quote_intent" => c.view(&c.hub, "quote_intent", json!({ "intent": s(a, "intent")? })).await,
            "fill_intent" => {
                c.call(
                    who,
                    &c.hub,
                    "fill_intent",
                    json!({ "intent": s(a, "intent")?, "signature": s(a, "signature")?, "treasury": a["treasury"].as_str() }),
                    zero,
                )
                .await
            }
            "post_job" => {
                let task = s(a, "task")?;
                let msg = json!({
                    "manager": s(a, "manager")?,
                    "spec_hash": ht_core::receipt::sha256_hex(task.as_bytes()),
                    "window_secs": a["window_secs"].as_u64().ok_or("missing window_secs")?,
                    "treasury": a["treasury"].as_str(),
                    "lead_fee": a["lead_fee"].as_str().unwrap_or("0"),
                })
                .to_string();
                let budget: u128 = s(a, "budget")?.parse().map_err(|_| "budget")?;
                c.send(who, &c.hub, budget, Some(&msg)).await
            }
            "hire" => {
                let task = s(a, "task")?;
                c.call(
                    who,
                    &id(a, "job")?,
                    "hire",
                    json!({ "worker": s(a, "worker")?, "amount": s(a, "amount")?,
                            "spec_hash": ht_core::receipt::sha256_hex(task.as_bytes()),
                            "sub": a["sub"].as_str() }),
                    NearToken::from_millinear(35),
                )
                .await
            }
            "open_bids" => {
                let task = s(a, "task")?;
                c.call(
                    who,
                    &id(a, "job")?,
                    "open_bids",
                    json!({ "spec_hash": ht_core::receipt::sha256_hex(task.as_bytes()), "max": s(a, "max")?,
                            "secs": a["secs"].as_u64().ok_or("missing secs")?, "bond": a["bond"].as_str(),
                            "sealed": a["sealed"].as_bool() }),
                    zero,
                )
                .await
            }
            "bid" => {
                // A slot may ask for a NEAR bond: attach it with the bid.
                let slot = a["slot"].as_u64().ok_or("missing slot")?;
                let job = id(a, "job")?;
                let info = c.view(&job, "get_bids", json!({ "slot": slot })).await?;
                let deposit = bid_deposit(1, crate::chain::num(&info["bond"]), stake(a)?)?;
                c.call(
                    who,
                    &job,
                    "bid",
                    json!({ "slot": slot, "price": s(a, "price")? }),
                    deposit,
                )
                .await
            }
            "bid_sealed" => {
                let slot = a["slot"].as_u64().ok_or("missing slot")?;
                let job = id(a, "job")?;
                let info = c.view(&job, "get_bids", json!({ "slot": slot })).await?;
                let deposit = bid_deposit(2, crate::chain::num(&info["bond"]), stake(a)?)?;
                let commit = ht_core::receipt::sha256_hex(
                    format!("{}:{}:{}", s(a, "price")?, s(a, "salt")?, who.id).as_bytes(),
                );
                c.call(
                    who,
                    &job,
                    "bid_sealed",
                    json!({ "slot": slot, "commit": commit }),
                    deposit,
                )
                .await
            }
            "reveal" => {
                c.call(
                    who,
                    &id(a, "job")?,
                    "reveal",
                    json!({ "slot": a["slot"].as_u64().ok_or("missing slot")?, "price": s(a, "price")?, "salt": s(a, "salt")? }),
                    zero,
                )
                .await
            }
            "get_bids" => {
                c.view(&id(a, "job")?, "get_bids", json!({ "slot": a["slot"].as_u64().ok_or("missing slot")? }))
                    .await
            }
            "award" => {
                c.call(
                    who,
                    &id(a, "job")?,
                    "award",
                    json!({ "slot": a["slot"].as_u64().ok_or("missing slot")?, "worker": s(a, "worker")?,
                            "sub": a["sub"].as_str() }),
                    NearToken::from_millinear(35),
                )
                .await
            }
            "deliver" => {
                use base64::Engine;
                let resp = base64::engine::general_purpose::STANDARD
                    .decode(s(a, "resp")?)
                    .map_err(|_| "resp is not base64")?;
                c.call(
                    who,
                    &id(a, "job")?,
                    "deliver",
                    json!({ "resp_hash": ht_core::receipt::sha256_hex(&resp), "resp": s(a, "resp")?,
                            "req": a["req"].as_str(), "sig": a["sig"].as_str(), "model": a["model"].as_str() }),
                    zero,
                )
                .await
            }
            "check_request" => {
                c.view(
                    &id(a, "job")?,
                    "check_request",
                    json!({ "panel": a["panel"], "task": s(a, "task")?, "deliverable": s(a, "deliverable")? }),
                )
                .await
            }
            "submit_check" => {
                c.call(
                    who,
                    &id(a, "job")?,
                    "submit_check",
                    json!({ "panel": a["panel"], "task": s(a, "task")?, "deliverable": s(a, "deliverable")?,
                            "answer": s(a, "answer")?, "sig": s(a, "sig")? }),
                    zero,
                )
                .await
            }
            "finalize_check" | "finalize_dispute" | "approve" | "fire" | "settle" => {
                c.call(who, &id(a, "job")?, name, json!({}), zero).await
            }
            "quote_note" => c.view(&id(a, "job")?, "quote_note", json!({})).await,
            "buy_note" => {
                let price: u128 = s(a, "price")?.parse().map_err(|_| "price")?;
                let quote = crate::chain::num(&c.view(&id(a, "job")?, "quote_note", json!({})).await?);
                if quote == 0 || price != quote {
                    return Err(format!("price must equal the current quote ({quote})"));
                }
                c.send(who, &id(a, "job")?, price, Some("buy_note")).await
            }
            "dispute" => {
                c.call(who, &id(a, "job")?, "dispute", json!({ "resp": s(a, "resp")? }), zero)
                    .await
            }
            "get_job" => c.view(&id(a, "job")?, "get_job", json!({})).await,
            "get_record" => {
                c.view(&c.hub, "get_record", json!({ "account_id": s(a, "account")? }))
                    .await
            }
            other => Err(format!("unknown tool {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostile_stake_is_refused_not_a_crash() {
        // u128::MAX used to overflow the deposit sum and panic the server.
        let max = json!({ "stake": u128::MAX.to_string() });
        assert_eq!(stake(&max), Err("stake is at most 5 NEAR".into()));
        assert!(stake(&json!({ "stake": "-1" })).is_err());
        assert_eq!(stake(&json!({})), Ok(0));
        assert_eq!(
            stake(&json!({ "stake": MAX_STAKE.to_string() })),
            Ok(MAX_STAKE)
        );
        // Even a slot bond near u128::MAX cannot overflow the deposit.
        assert!(bid_deposit(1, u128::MAX - 5, MAX_STAKE).is_err());
        assert_eq!(
            bid_deposit(1, 7, 3).unwrap().as_yoctonear(),
            NearToken::from_millinear(1).as_yoctonear() + 10
        );
    }
}
