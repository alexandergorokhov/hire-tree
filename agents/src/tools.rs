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
        ("post_job", "Client: post and fund a root job through the factory (needs 0.06 NEAR prepaid on the factory, its `prepay_post` method; the MCP playground prepays for you).",
         s(json!({ "budget": str_, "manager": str_, "task": str_, "window_secs": { "type": "integer" }, "lead_fee": str_, "treasury": str_ }), &["budget", "manager", "task", "window_secs"])),
        ("deposit", "Client: keep `amount` fUSD at the factory for signed job intents (an ft_transfer_call with msg \"deposit\").",
         s(json!({ "amount": str_ }), &["amount"])),
        ("sign_intent", "Client: register your intent key (once) and sign a job intent off-chain: get `task` done for `budget`; a lead may keep up to `max_lead_fee`, rising from 0 now to the max after `secs`. Returns {intent, signature} for any lead to fill, or only the accounts in `leads` if you name them: the filler becomes the job's manager and spends its budget, so an open intent trusts whoever fills first. No transaction but the key registration.",
         s(json!({ "budget": str_, "task": str_, "window_secs": { "type": "integer" }, "max_lead_fee": str_, "secs": { "type": "integer" }, "nonce": str_, "leads": { "type": "array", "items": str_ } }), &["budget", "task", "window_secs", "max_lead_fee", "secs", "nonce"])),
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
        ("seek_backer", "Worker, after bidding: ask an underwriter to back your bid with more bond. You offer `premium` (yoctoNEAR, at most half your own bond), paid out of your bond only if you win and your work is paid. A newcomer with no record uses it to borrow a trusted agent's vouch.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "premium": str_ }), &["job", "slot", "premium"])),
        ("back", "Underwriter: back another agent's bid that seeks a backer, attaching `amount` yoctoNEAR: at least the bidder's own bond and twice the premium, at most 5 NEAR. Buyers see the bid's bond plus your backing. Paid work: you get it back plus the premium. Junk: the buyer keeps it. A losing bid: refunded at the award.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "bidder": str_, "amount": str_ }), &["job", "slot", "bidder", "amount"])),
        ("reveal", "Worker, sealed slot, after bidding closes: reveal the committed price with its salt. An unrevealed bid cannot win and loses its bond.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "price": str_, "salt": str_ }), &["job", "slot", "price", "salt"])),
        ("bid", "Worker: bid `price` (at most the slot's max) on an open slot; a second bid replaces the first. Attaches 0.001 NEAR (the bid's storage) plus the slot's bond, if any, plus an optional `stake` (yoctoNEAR) on top: a bigger bond vouches for a bidder with no record yet.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "price": str_, "stake": str_ }), &["job", "slot", "price"])),
        ("get_bids", "Anyone: a bid slot with its bids and award.", s(json!({ "job": str_, "slot": { "type": "integer" } }), &["job", "slot"])),
        ("award", "Manager: after bidding closes, hire one bidder at its own bid (a new sub-account). Attaches 0.035 NEAR.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "worker": str_, "sub": str_ }), &["job", "slot", "worker"])),
        ("award_best", "Manager: after bidding closes, the job contract picks the winner by a public rule and hires it at its own bid. It reads each revealed bidder's on-chain record, scores bid x (1 + record_weight x unpaid share of the record) and, for a bidder with no record, x (1 + newcomer_pct / how many times the slot's bond the bid carries), and hires the lowest score (ties: more paid money, then the earlier bid). record_weight 150 = x2.5 when all pay was lost; 0 and 0 = lowest bid. `skip` lists bidders you cannot use (it goes in the event). `min_paid` (fUSD units, optional): paid money below it is no record, so a bidder that washed a cent through its own job is still a newcomer. `payers` (optional list of up to 16 manager accounts): only the pay of those managers counts as a bidder's paid money, so a record washed through accounts you do not list is no record; unpaid money counts whoever it came from; the list goes in the events. Refused in the last 60 s of the job's window (the winner needs time to deliver). Fails closed (an error, nothing awarded, slot still open) if a record cannot be read. Attaches 0.035 NEAR.",
         s(json!({ "job": str_, "slot": { "type": "integer" }, "record_weight": { "type": "integer" }, "newcomer_pct": { "type": "integer" }, "sub": str_, "skip": { "type": "array", "items": str_ }, "min_paid": str_, "payers": { "type": "array", "items": str_ } }), &["job", "slot", "record_weight", "newcomer_pct"])),
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
        ("payout_via", "Worker (the payee), before settle: have your pay land as your balance in the NEAR Intents Verifier (`via` = the Verifier's account on this network), where you can swap it for another token or withdraw it. Omit `via` for a plain transfer. If the Verifier refuses the deposit, the route is dropped and the next settle pays you plainly.",
         s(json!({ "job": str_, "via": str_ }), &["job"])),
        ("settle", "Anyone: settle a job (pay, return the rest, delete at zero).", s(json!({ "job": str_ }), &["job"])),
        ("get_job", "Anyone: a job's state.", s(json!({ "job": str_ }), &["job"])),
        ("get_record", "Anyone: a worker's track record kept by the factory. With `payers` (up to 16 manager accounts), its paid money and payer count count only what those managers paid.", s(json!({ "account": str_, "payers": { "type": "array", "items": str_ } }), &["account"])),
        ("find_agents", "Anyone, before hiring: rank candidate agents by their on-chain records (up to 32 `agents`; with `payers`, only money those managers paid counts). Best first: least share of pay lost to junk (no record counts as half), then most fUSD paid, then most different payers. `has_record: false` means the factory has nothing on that account (new, or not an agent). Then hire one with `hire` or `open_bids` + `award_best`, or name it `manager` in post_job.",
         s(json!({ "agents": { "type": "array", "items": str_ }, "payers": { "type": "array", "items": str_ } }), &["agents"])),
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
                intent_seed_ok(c.sbx.is_some(), std::env::var("HT_INTENT_SEED").is_ok())?;
                let key = crate::intent::public_key(&me);
                let have = c.view(&c.hub, "get_intent_key", json!({ "account_id": me })).await?;
                match have.as_str() {
                    Some(k) if k == key => {}
                    // Never replace a key the account registered itself.
                    Some(_) => return Err("this account already registered its own intent key: sign intents with that key, not this tool".into()),
                    None => {
                        c.call(who, &c.hub, "set_intent_key", json!({ "public_key": key }), zero).await?;
                    }
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
                let intent = match a["leads"].as_array() {
                    Some(l) => crate::intent::name_leads(
                        &intent,
                        &l.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>(),
                    ),
                    None => intent,
                };
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
            "seek_backer" => {
                c.call(
                    who,
                    &id(a, "job")?,
                    "seek_backer",
                    json!({ "slot": a["slot"].as_u64().ok_or("missing slot")?, "premium": s(a, "premium")? }),
                    zero,
                )
                .await
            }
            "back" => {
                let amount: u128 = s(a, "amount")?
                    .parse()
                    .map_err(|_| "amount must be yoctoNEAR as a string".to_string())?;
                if amount == 0 || amount > MAX_STAKE {
                    return Err("back with 1 yocto to 5 NEAR".into());
                }
                c.call(
                    who,
                    &id(a, "job")?,
                    "back",
                    json!({ "slot": a["slot"].as_u64().ok_or("missing slot")?, "bidder": id(a, "bidder")? }),
                    NearToken::from_yoctonear(amount),
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
            "award_best" => {
                let job = id(a, "job")?;
                let slot = a["slot"].as_u64().ok_or("missing slot")?;
                let num = |k: &str| a[k].as_u64().ok_or(format!("missing {k}"));
                c.call(
                    who,
                    &job,
                    "award_best",
                    json!({ "slot": slot, "record_weight": num("record_weight")?, "newcomer_pct": num("newcomer_pct")?,
                            "sub": a["sub"].as_str(), "skip": a["skip"], "min_paid": a["min_paid"].as_str(), "payers": a["payers"] }),
                    NearToken::from_millinear(35),
                )
                .await?;
                // The record reads and the hire run in a callback, which can
                // refuse without failing this transaction: read the outcome.
                let bids = c.view(&job, "get_bids", json!({ "slot": slot })).await?;
                match bids["awarded"].as_str() {
                    Some(w) => Ok(json!({ "slot": slot, "awarded": w })),
                    None => Err("award_best made no award (a record could not be read, the job could not hire, or it is too late in the window): the deposit was refunded, see the award_best_failed event".into()),
                }
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
            "payout_via" => {
                let via = a["via"].as_str().filter(|v| !v.is_empty());
                c.call(who, &id(a, "job")?, "set_payout_via", json!({ "via": via }), zero)
                    .await
            }
            "settle" => {
                let job = id(a, "job")?;
                // Many open bid bonds: refund them in batches first.
                for _ in 0..16 {
                    match c.call(who, &job, "settle", json!({}), zero).await {
                        Err(e) if e.contains("call refund_bids first") => {
                            c.call(who, &job, "refund_bids", json!({}), zero).await?;
                        }
                        r => return r,
                    }
                }
                Err("settle: open bid bonds remain after 16 refund batches".into())
            }
            "finalize_check" | "finalize_dispute" | "approve" | "fire" => {
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
            "get_record" if a["payers"].is_array() => {
                c.view(&c.hub, "get_record_from", json!({ "account_id": s(a, "account")?, "payers": a["payers"] }))
                    .await
            }
            "get_record" => {
                c.view(&c.hub, "get_record", json!({ "account_id": s(a, "account")? }))
                    .await
            }
            "find_agents" => {
                let agents = a["agents"].as_array().ok_or("missing agents")?;
                if agents.is_empty() || agents.len() > MAX_FIND {
                    return Err(format!("agents: 1 to {MAX_FIND} accounts"));
                }
                let mut rows = vec![];
                for x in agents {
                    let acct = x.as_str().ok_or("agents are account ids")?;
                    acct.parse::<AccountId>().map_err(|e| format!("agents: {e}"))?;
                    let r = if a["payers"].is_array() {
                        c.view(&c.hub, "get_record_from", json!({ "account_id": acct, "payers": a["payers"] })).await?
                    } else {
                        c.view(&c.hub, "get_record", json!({ "account_id": acct })).await?
                    };
                    rows.push((acct.to_string(), r));
                }
                Ok(json!(rank_agents(rows, a["payers"].is_array())))
            }
            other => Err(format!("unknown tool {other}")),
        }
    }
}

/// Demo intent keys come from the account id and `HT_INTENT_SEED`. With the
/// default seed anyone can compute them, which is fine on a local node only.
fn intent_seed_ok(local_node: bool, seed_set: bool) -> Result<(), String> {
    if !local_node && !seed_set {
        return Err("sign_intent off a local node needs a private HT_INTENT_SEED in the environment (the default seed is public)".into());
    }
    Ok(())
}

/// The most candidates one `find_agents` call reads.
pub const MAX_FIND: usize = 32;

fn num(r: &Value, k: &str) -> u128 {
    match &r[k] {
        Value::String(s) => s.parse().unwrap_or(0),
        v => v.as_u64().unwrap_or(0) as u128,
    }
}

/// Basis points of an agent's recorded pay that it lost (junk, fired). An
/// agent with no money on record counts as half-trusted (5000), so a clean
/// record ranks above a newcomer and a newcomer above a mostly-junk one.
fn lost_bps(r: &Value) -> u128 {
    let (paid, lost) = (num(r, "paid_value"), num(r, "unpaid_value"));
    match paid.checked_add(lost) {
        Some(0) | None => 5000,
        Some(all) => lost.saturating_mul(10_000) / all,
    }
}

/// A catalog of hireable agents from their on-chain records, best first:
/// least share of pay lost to junk, then most fUSD paid, then most different
/// payers, then name. With `filtered` (a `payers` list was given), the
/// per-delivery fields cover the whole record, so they are left out.
pub fn rank_agents(rows: Vec<(String, Value)>, filtered: bool) -> Vec<Value> {
    let mut v: Vec<(String, Value)> = rows;
    v.sort_by(|(a, x), (b, y)| {
        lost_bps(x)
            .cmp(&lost_bps(y))
            .then(num(y, "paid_value").cmp(&num(x, "paid_value")))
            .then(num(y, "payers").cmp(&num(x, "payers")))
            .then(a.cmp(b))
    });
    v.into_iter()
        .map(|(acct, r)| {
            let (delivered, paid, scored) = (num(&r, "delivered"), num(&r, "paid"), num(&r, "scored"));
            let whole = |x: Value| if filtered { Value::Null } else { x };
            json!({
                "account": acct,
                "has_record": num(&r, "jobs") > 0,
                "jobs": num(&r, "jobs"),
                "paid": whole(json!(paid)),
                "junk": num(&r, "junk"),
                "pass_rate": whole(if delivered == 0 { Value::Null } else { json!(paid as f64 / delivered as f64) }),
                "avg_score": whole(if scored == 0 { Value::Null } else { json!(num(&r, "score_sum") as f64 / scored as f64) }),
                "paid_value": num(&r, "paid_value").to_string(),
                "unpaid_value": num(&r, "unpaid_value").to_string(),
                "payers": num(&r, "payers"),
                "lost_share": lost_bps(&r) as f64 / 10_000.0,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(junk: u32, paid_value: &str, payers: u32, delivered: u32, paid: u32) -> Value {
        json!({ "jobs": delivered, "delivered": delivered, "checked": delivered, "junk": junk, "paid": paid,
                "score_sum": paid * 8, "scored": paid, "paid_value": paid_value, "unpaid_value": "0", "payers": payers })
    }

    #[test]
    fn find_agents_ranks_clean_paid_records_first() {
        let mut junk = rec(2, "90000000", 3, 5, 3);
        junk["unpaid_value"] = json!("60000000");
        let out = rank_agents(
            vec![
                ("junk.near".into(), junk),
                ("new.near".into(), rec(0, "0", 0, 0, 0)),
                ("solid.near".into(), rec(0, "50000000", 2, 4, 4)),
                ("wide.near".into(), rec(0, "50000000", 5, 4, 4)),
            ],
            false,
        );
        let order: Vec<&str> = out.iter().map(|r| r["account"].as_str().unwrap()).collect();
        // More money paid does not buy back 40% of pay lost to junk.
        assert_eq!(order, ["wide.near", "solid.near", "junk.near", "new.near"]);
        assert_eq!(out[0]["pass_rate"], json!(1.0));
        assert_eq!(out[0]["avg_score"], json!(8.0));
        assert_eq!(
            out[3]["pass_rate"],
            Value::Null,
            "no deliveries yet: no rate, not 0%"
        );
        assert_eq!(out[3]["has_record"], json!(false));
        assert_eq!(out[2]["lost_share"], json!(0.4));
    }

    #[test]
    fn find_agents_puts_newcomers_between_clean_and_mostly_junk_records() {
        let mut veteran = rec(1, "400000000", 6, 40, 39);
        veteran["unpaid_value"] = json!("10000000");
        let mut junky = rec(3, "5000000", 1, 4, 1);
        junky["unpaid_value"] = json!("30000000");
        let out = rank_agents(
            vec![
                ("junky.near".into(), junky),
                ("nobody.near".into(), rec(0, "0", 0, 0, 0)),
                ("veteran.near".into(), veteran),
            ],
            false,
        );
        let order: Vec<&str> = out.iter().map(|r| r["account"].as_str().unwrap()).collect();
        assert_eq!(
            order,
            ["veteran.near", "nobody.near", "junky.near"],
            "one junk in 40 jobs beats an unknown account"
        );
        assert_eq!(out[1]["has_record"], json!(false));
    }

    #[test]
    fn find_agents_with_payers_hides_whole_record_rates() {
        let out = rank_agents(vec![("sybil.near".into(), rec(0, "0", 0, 5, 5))], true);
        assert_eq!(
            out[0]["pass_rate"],
            Value::Null,
            "a washed record must not show pass_rate 1.0"
        );
        assert_eq!(out[0]["avg_score"], Value::Null);
        assert_eq!(out[0]["paid"], Value::Null);
    }

    #[test]
    fn demo_intent_keys_need_a_private_seed_off_the_local_node() {
        assert!(intent_seed_ok(true, false).is_ok());
        assert!(intent_seed_ok(false, false).is_err());
        assert!(intent_seed_ok(false, true).is_ok());
    }

    #[test]
    fn find_agents_is_listed_and_needs_agents() {
        assert!(list()
            .iter()
            .any(|(n, _, s)| *n == "find_agents" && s["required"] == json!(["agents"])));
    }

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
