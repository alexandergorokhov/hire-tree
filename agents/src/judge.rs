//! The judge's playground: `hiretree mcp --local`. A stdio MCP server that
//! starts its own local NEAR node on the first tool call, deploys the
//! contracts, creates and funds the demo accounts, and lets any MCP client
//! (a judge's Claude, for example) act as any of them with `as`. Nothing
//! leaves the machine: the node is local, the model is the scripted test
//! model signing with a test key (pinned by this local factory), and all
//! money is fake.

use crate::agents::{b64, json_of, mock_script, story, team, Team, CHECK_WINDOW_SECS, PICKER};
use crate::chain::{num, Acc, Chain, USD};
use crate::llm::{chat_body, Llm, Mock};
use crate::mcp::PROTOCOL;
use crate::tools::{list, Tools};
use crate::verifier::{self, nep141, Verifier};
use base64::Engine;
use near_api::types::NearToken;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub const MODEL: &str = "scripted-test-model";
pub const WORKER: &str = crate::agents::WORKER;

pub struct Playground {
    pub chain: Chain,
    pub llm: Llm,
    pub team: Team,
    /// The exact bytes each job's last delivery carried (the contract
    /// checks the judge question against their hash).
    pub delivered: std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>,
    /// Guest agents: (name, account, skills).
    pub guests: std::sync::Mutex<Vec<(String, Acc, Vec<String>)>>,
    pub rounds: std::sync::Mutex<std::collections::HashMap<String, Round>>,
    /// NEAR Intents: the Verifier (its own release wasm) and the house
    /// solver that quotes fUSD for wNEAR.
    pub intents: Verifier,
    pub solver: Acc,
    /// Fresh NEP-413 nonces.
    pub nonces: std::sync::atomic::AtomicU64,
}

/// The house solver's fixed test rate: wNEAR (yocto) per fUSD unit (1e-6
/// fUSD). 1 fUSD = 0.2 wNEAR. Fake money on both sides.
pub const SOLVER_YOCTO_PER_UNIT: u128 = 200_000_000_000_000_000;
/// wNEAR the house solver brings into the Verifier.
const SOLVER_WNEAR: u128 = 20 * 10u128.pow(24);

/// One open-market round: its parts (skill, task, slot) and, after the
/// awards, the sub-jobs (account, task).
#[derive(Clone, Default)]
pub struct Round {
    pub parts: Vec<(String, String, u64)>,
    pub hired: Vec<(String, String)>,
    /// Sub-parts a guest contractor bought out of its own pay:
    /// (its sub-job, the slot on its job, task, house winner or None).
    pub subs: Vec<(String, u64, String, Option<String>)>,
}

pub const SKILLS: [&str; 3] = ["research", "translate", "summary"];
const TOPICS: [&str; 6] = [
    "NEAR sub-accounts",
    "NEAR global contracts",
    "NEAR AI Cloud receipts",
    "fungible tokens on NEAR",
    "storage staking on NEAR",
    "function-call access keys",
];
const ROUND_BID_SECS: u64 = 60;
const ROUND_WINDOW_SECS: u64 = 900;
/// House workers bid their rate (percent of max) times this margin (/100).
const HOUSE_MARGIN: u128 = 95;

pub const GUIDE: &str = "Hire Tree playground on a local NEAR node (fake money, scripted test model signing with a test key).

Every job is its own NEAR account holding exactly its budget. Pass `as` on any tool to act as one of: client, client2, client3, lead, treasury, keeper, researcher, translator, cheapbot (default client). Amounts are fUSD with 6 decimals, as strings (\"30000000\" = 30 fUSD).

Try this (about 15 calls):
1. post_job as client: budget \"50000000\", manager = the lead's account (see `accounts`), task \"Explain NEAR sub-accounts\", window_secs 3600, lead_fee \"5000000\", treasury = the treasury's account (it can pay workers early). The root job is job-<n>.<factory> (get_job shows it).
2. open_bids as lead on that job: task \"Explain NEAR sub-accounts in 3 sentences\", max \"30000000\", secs 60. It returns the slot.
3. bid as researcher (price \"25000000\") and as cheapbot (\"10000000\"); try a bid above max and see it refused.
4. wait 61, then get_bids and award as lead (worker = the researcher's account). The award creates t1.<job> holding exactly 25 fUSD.
5. do_work as researcher on t1 with the same task: the test model answers with the job's HT1 tag and signs; the contract checks the signature.
6. judge_check on t1 with the same task (as keeper): the contract builds the judge question, the model's signed answer is verified on-chain, the score is read (8/10; pass text containing \"lorem\" to do_work to get 2/10 and see the job reopen).
7. wait 301, finalize_check, then quote_note and buy_note as treasury: the worker is paid now.
8. approve as lead on t1, settle t1 (as keeper), approve as client and settle the root: each account pays out, checks its balance is zero and deletes itself. get_record shows the worker's record.
Open market (compete yourself): join with a name and skills, open_round, bid on the matching slot as your guest (undercut the house, or not), close_round, do_work + judge_check if you won, finish_round. The lead prefers the lowest bid unless the bidder's on-chain record shows unpaid jobs, so deliver junk once (do_work with text \"lorem ipsum\") and watch your next bid lose.
Become a contractor: after you win a part, subcontract a piece of it (subcontract: task, skill, max below your price), the house bids at once; hire_sub awards a bidder, creating a sub-sub-account paid out of your own pay; you keep the margin. Then do_work on your own part, judge_check, finish_round.
Get paid in another token (NEAR Intents): before your job settles, call payout_via as its worker with via = intents_verifier (see accounts); the pay then lands as your balance in the official Intents Verifier contract, and cash_out swaps it for wNEAR with the house solver and withdraws it.
Keys: an agent should not hand its full-access key to its runtime. Deliveries in the swarm use a function-call key that can call only `deliver` on one job account (no deposit, no transfers, no deletion); bids attach NEAR, so they stay on the owner's key. See docs/THREATS.md, Agent keys.\nOr call run_story once to watch the whole scripted three-job story (bids, a junk bot losing work by its record, three clients pooling money); it takes a few minutes.";

impl Playground {
    pub async fn boot() -> Playground {
        let llm = Llm::Mock(Mock::new(MODEL, Box::new(mock_script)));
        let chain = Chain::sandbox(&llm.signer().unwrap(), Some(MODEL)).await;
        let team = team(&chain).await;
        let (intents, solver) = house_solver(&chain).await;
        Playground {
            chain,
            llm,
            team,
            delivered: Default::default(),
            guests: Default::default(),
            rounds: Default::default(),
            intents,
            solver,
            nonces: Default::default(),
        }
    }

    fn named(&self) -> Vec<(String, Acc)> {
        let t = &self.team;
        let mut v = vec![
            ("client".to_string(), t.client.clone()),
            ("client2".to_string(), t.members[0].clone()),
            ("client3".to_string(), t.members[1].clone()),
            ("lead".to_string(), t.lead.clone()),
            ("treasury".to_string(), t.treasury.clone()),
            ("keeper".to_string(), t.keeper.clone()),
        ];
        for w in &t.workers {
            v.push((w.name.clone(), w.acc.clone()));
        }
        for g in self.guests.lock().unwrap().iter() {
            v.push((g.0.clone(), g.1.clone()));
        }
        v
    }

    fn skills_of(&self, id: &str) -> Vec<String> {
        if let Some(w) = self.team.workers.iter().find(|w| w.acc.id.as_str() == id) {
            return w.skills.clone();
        }
        self.guests
            .lock()
            .unwrap()
            .iter()
            .find(|g| g.1.id.as_str() == id)
            .map(|g| g.2.clone())
            .unwrap_or_default()
    }

    /// A guest agent (the judge's own) joins the market with its own
    /// funded account and skills; it then acts with `as` = its name.
    async fn join(&self, a: &Value) -> Result<Value, String> {
        let name: String = a["name"]
            .as_str()
            .ok_or("missing name")?
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(16)
            .collect();
        if name.is_empty() || self.named().iter().any(|n| n.0 == name) {
            return Err(format!("pick another name ({name} is taken or empty)"));
        }
        let skills: Vec<String> = a["skills"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
            .filter(|s| SKILLS.contains(&s.as_str()))
            .collect();
        if skills.is_empty() {
            return Err(format!("skills: one or more of {SKILLS:?}"));
        }
        let acc = self.chain.account(&format!("guest{name}"), 5).await;
        self.chain.register(&acc).await;
        self.guests
            .lock()
            .unwrap()
            .push((name.clone(), acc.clone(), skills.clone()));
        Ok(json!({ "as": name, "account": acc.id, "skills": skills,
            "next": "call open_round (or wait for one) and bid on the slots that match your skills; the lead picks by price and on-chain record" }))
    }

    /// The house client posts a job; the house lead puts three parts out
    /// for bids; the house workers bid by their own strategy. Guests bid
    /// next with `bid`, then `close_round`.
    async fn open_round(&self) -> Result<Value, String> {
        let c = &self.chain;
        let t = &self.team;
        let tools = Tools { chain: c };
        let next = c.view(&c.hub, "get_settings", json!({})).await?["next"]
            .as_u64()
            .ok_or("factory: no next")?;
        let root = format!("job-{next}.{}", c.hub);
        let topic = TOPICS[(next as usize) % TOPICS.len()];
        let task = format!(
            "Write a short explainer of {topic} in English and Spanish, plus a one-line summary."
        );
        c.call(
            &t.client,
            &c.hub,
            "prepay_post",
            json!({}),
            near_api::types::NearToken::from_millinear(60),
        )
        .await?;
        tools.call(&t.client, "post_job", &json!({ "budget": (70 * USD).to_string(), "manager": t.lead.id, "task": task,
            "window_secs": ROUND_WINDOW_SECS, "lead_fee": (7 * USD).to_string(), "treasury": t.treasury.id })).await?;
        let parts = [
            (
                "research",
                format!("Explain {topic} in 3 sentences."),
                30u128,
            ),
            (
                "translate",
                format!("Translate the explanation of {topic} into Spanish."),
                20,
            ),
            (
                "summary",
                format!("Write a one-line summary of {topic}."),
                10,
            ),
        ];
        let mut slots = vec![];
        for (name, ptask, max) in &parts {
            let slot = tools
                .call(
                    &t.lead,
                    "open_bids",
                    &json!({ "job": root, "task": ptask,
                "max": (max * USD).to_string(), "secs": ROUND_BID_SECS }),
                )
                .await?;
            let slot = slot.as_u64().ok_or("open_bids: no slot")?;
            for w in &t.workers {
                if w.skills.iter().any(|s| s == name) {
                    let price = max * USD * w.rate * HOUSE_MARGIN / 10_000;
                    let _ = tools
                        .call(
                            &w.acc,
                            "bid",
                            &json!({ "job": root, "slot": slot, "price": price.to_string() }),
                        )
                        .await;
                }
            }
            slots.push(json!({ "slot": slot, "skill": name, "task": ptask, "max": (max * USD).to_string(),
                "bids": tools.call(&t.lead, "get_bids", &json!({ "job": root, "slot": slot })).await?["bids"] }));
        }
        self.rounds.lock().unwrap().insert(
            root.clone(),
            Round {
                parts: parts
                    .iter()
                    .zip(&slots)
                    .map(|((n, t, _), s)| (n.to_string(), t.clone(), s["slot"].as_u64().unwrap()))
                    .collect(),
                hired: vec![],
                subs: vec![],
            },
        );
        Ok(json!({ "job": root, "task": task, "slots": slots,
            "note": format!("bidding closes in {ROUND_BID_SECS} s of chain time; bid with `bid` as your guest (job, slot, price), then close_round") }))
    }

    /// Bidding closes; the lead picks each slot's winner by price and
    /// on-chain record and awards it; the house winners work at once.
    async fn close_round(&self, a: &Value) -> Result<Value, String> {
        let c = &self.chain;
        let t = &self.team;
        let tools = Tools { chain: c };
        let root = a["job"].as_str().ok_or("missing job")?.to_string();
        let parts = self
            .rounds
            .lock()
            .unwrap()
            .get(&root)
            .map(|r| r.parts.clone())
            .ok_or("not an open round: call open_round")?;
        c.fast_forward(ROUND_BID_SECS + 1).await;
        let mut out = vec![];
        let mut hired = vec![];
        for (name, ptask, slot) in &parts {
            let b = tools
                .call(&t.lead, "get_bids", &json!({ "job": root, "slot": slot }))
                .await?;
            let mut cands = vec![];
            for bid in b["bids"].as_array().into_iter().flatten() {
                let who = bid["worker"].as_str().unwrap_or_default().to_string();
                let rec = tools
                    .call(&t.lead, "get_record", &json!({ "account": who }))
                    .await?;
                cands.push(json!({ "account": who, "skills": self.skills_of(&who), "bid": num(&bid["price"]) / USD, "record": rec }));
            }
            if cands.is_empty() {
                out.push(json!({ "skill": name, "award": null, "reason": "no bids" }));
                continue;
            }
            let body = chat_body(
                MODEL,
                PICKER,
                &json!({ "subtask": { "name": name, "task": ptask }, "candidates": cands })
                    .to_string(),
                200,
            );
            let pick = json_of(&self.llm.chat(&body).await?.content()).ok_or("picker: not JSON")?;
            let who = pick["worker"].as_str().unwrap_or_default().to_string();
            if who.is_empty() {
                out.push(
                    json!({ "skill": name, "award": null, "reason": "no bidder has this skill" }),
                );
                continue;
            }
            tools
                .call(
                    &t.lead,
                    "award",
                    &json!({ "job": root, "slot": slot, "worker": who }),
                )
                .await?;
            let sub = format!("t{}.{root}", hired.len() + 1);
            let price = cands
                .iter()
                .find(|x| x["account"] == who.as_str())
                .map(|x| x["bid"].clone())
                .unwrap_or(json!(null));
            let guest = self
                .guests
                .lock()
                .unwrap()
                .iter()
                .find(|g| g.1.id.as_str() == who)
                .map(|g| g.0.clone());
            match &guest {
                Some(g) => out.push(json!({ "skill": name, "award": who, "price_fusd": price, "sub_job": sub, "reason": pick["reason"],
                    "you_won": format!("call do_work as {g} with job {sub} and task {ptask:?}, then judge_check, then finish_round") })),
                None => {
                    let w = t.workers.iter().find(|w| w.acc.id.as_str() == who).ok_or("unknown winner")?;
                    let wname = w.name.clone();
                    if w.junk {
                        let bytes = b"lorem ipsum dolor sit amet, lorem ipsum dolor sit amet".to_vec();
                        tools.call(&w.acc, "deliver", &json!({ "job": sub, "resp": b64(&bytes) })).await?;
                        self.delivered.lock().unwrap().insert(sub.clone(), bytes);
                    } else {
                        self.do_work(&json!({ "as": wname, "job": sub, "task": ptask })).await?;
                    }
                    self.judge_check(&json!({ "job": sub, "task": ptask })).await?;
                    out.push(json!({ "skill": name, "award": who, "price_fusd": price, "sub_job": sub, "reason": pick["reason"], "worked": "delivered and judged" }));
                }
            }
            hired.push((sub, ptask.clone()));
        }
        if let Some(r) = self.rounds.lock().unwrap().get_mut(&root) {
            r.hired = hired;
        }
        Ok(json!({ "job": root, "awards": out }))
    }

    /// Close the check rounds, pay good work early, fold the tree, and show
    /// who earned what and every record.
    async fn finish_round(&self, a: &Value) -> Result<Value, String> {
        let c = &self.chain;
        let t = &self.team;
        let tools = Tools { chain: c };
        let root = a["job"].as_str().ok_or("missing job")?.to_string();
        let hired = self
            .rounds
            .lock()
            .unwrap()
            .get(&root)
            .map(|r| r.hired.clone())
            .ok_or("not a closed round")?;
        let subs = self
            .rounds
            .lock()
            .unwrap()
            .get(&root)
            .map(|r| r.subs.clone())
            .unwrap_or_default();
        c.fast_forward(CHECK_WINDOW_SECS + 1).await;
        let mut lines = vec![];
        // Sub-jobs a guest contractor bought settle first: its buyer accepts
        // passed work; failed work waits for the window and is not paid.
        let mut sub_wait = false;
        for (job, _, _, sub) in &subs {
            let Some(sub) = sub else { continue };
            let _ = tools
                .call(&t.keeper, "finalize_check", &json!({ "job": sub }))
                .await;
            let buyer = tools
                .call(&t.lead, "get_job", &json!({ "job": job }))
                .await?["worker"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let buyer = self.who(&json!({ "as": buyer }))?;
            let j = tools
                .call(&t.lead, "get_job", &json!({ "job": sub }))
                .await?;
            if j["status"] == "Checked" {
                tools
                    .call(&buyer, "approve", &json!({ "job": sub }))
                    .await?;
                lines.push(format!(
                    "{sub}: score {}, accepted by its contractor",
                    j["quality"]
                ));
            } else {
                sub_wait = true;
                lines.push(format!("{sub}: score {}, not accepted", j["quality"]));
            }
        }
        if sub_wait {
            c.fast_forward(ROUND_WINDOW_SECS + 1).await;
        }
        for (_, _, _, sub) in &subs {
            if let Some(sub) = sub {
                tools
                    .call(&t.keeper, "settle", &json!({ "job": sub }))
                    .await?;
            }
        }
        for (sub, _) in &hired {
            let _ = tools
                .call(&t.keeper, "finalize_check", &json!({ "job": sub }))
                .await;
            let j = tools
                .call(&t.lead, "get_job", &json!({ "job": sub }))
                .await?;
            let q = tools
                .call(&t.lead, "quote_note", &json!({ "job": sub }))
                .await?;
            if !q.is_null() && j["quality"].as_u64().unwrap_or(0) >= 7 {
                let price = q
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| q.to_string());
                tools
                    .call(
                        &t.treasury,
                        "buy_note",
                        &json!({ "job": sub, "price": price }),
                    )
                    .await?;
                lines.push(format!("{sub}: score {}, paid now", j["quality"]));
            } else {
                lines.push(format!(
                    "{sub}: status {}, score {}: not paid early",
                    j["status"], j["quality"]
                ));
            }
        }
        let mut wait = false;
        for (sub, _) in &hired {
            let j = tools
                .call(&t.lead, "get_job", &json!({ "job": sub }))
                .await?;
            if !matches!(j["status"].as_str(), Some("Checked") | Some("Refunded"))
                || tools
                    .call(&t.lead, "approve", &json!({ "job": sub }))
                    .await
                    .is_err()
            {
                wait = true;
            }
        }
        if wait {
            c.fast_forward(ROUND_WINDOW_SECS + 1).await;
        }
        for (sub, _) in &hired {
            tools
                .call(&t.keeper, "settle", &json!({ "job": sub }))
                .await?;
        }
        let _ = tools
            .call(&t.client, "approve", &json!({ "job": root }))
            .await;
        tools
            .call(&t.keeper, "settle", &json!({ "job": root }))
            .await?;
        self.rounds.lock().unwrap().remove(&root);
        let mut table = vec![];
        for (n, acc) in self.named() {
            if self.skills_of(acc.id.as_str()).is_empty() {
                continue;
            }
            let rec = tools
                .call(&t.lead, "get_record", &json!({ "account": acc.id }))
                .await?;
            table.push(json!({ "as": n, "fUSD": format!("{:.2}", c.ft(&acc.id).await as f64 / 1e6), "record": rec }));
        }
        Ok(json!({ "job": root, "outcome": lines, "workers": table,
            "note": "the job tree folded: every account paid out and deleted itself; records feed the next round's picks" }))
    }

    fn who(&self, a: &Value) -> Result<Acc, String> {
        let name = a["as"].as_str().unwrap_or("client");
        self.named()
            .into_iter()
            .find(|(n, acc)| n == name || acc.id.as_str() == name)
            .map(|(_, acc)| acc)
            .ok_or(format!(
                "unknown `as` {name}: use one of {}",
                self.named()
                    .iter()
                    .map(|n| n.0.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
    }

    pub async fn call(&self, name: &str, a: &Value) -> Result<Value, String> {
        let c = &self.chain;
        match name {
            "guide" => Ok(json!(GUIDE)),
            "accounts" => {
                let mut out = vec![];
                for (n, acc) in self.named() {
                    out.push(json!({ "as": n, "account": acc.id, "fUSD": format!("{:.2}", c.ft(&acc.id).await as f64 / 1e6) }));
                }
                Ok(
                    json!({ "factory": c.hub, "token": c.usdc, "intents_verifier": self.intents.id,
                    "wnear": self.intents.wnear, "accounts": out }),
                )
            }
            "wait" => {
                let secs = a["secs"].as_u64().ok_or("missing secs")?.min(7200);
                c.fast_forward(secs).await;
                Ok(json!(format!("{secs} s of chain time passed")))
            }
            "do_work" => self.do_work(a).await,
            "join" => self.join(a).await,
            "cash_out" => self.cash_out(a).await,
            "open_round" => self.open_round().await,
            "close_round" => self.close_round(a).await,
            "finish_round" => self.finish_round(a).await,
            "subcontract" => self.subcontract(a).await,
            "hire_sub" => self.hire_sub(a).await,
            "judge_check" => self.judge_check(a).await,
            // The playground's catalog: with no `agents`, every demo account.
            "find_agents" if !a["agents"].is_array() => {
                let all: Vec<String> = self
                    .named()
                    .into_iter()
                    .map(|(_, acc)| acc.id.to_string())
                    .collect();
                let mut args = a.clone();
                args["agents"] = json!(all
                    .into_iter()
                    .take(crate::tools::MAX_FIND)
                    .collect::<Vec<_>>());
                Box::pin(self.call("find_agents", &args)).await
            }
            "run_story" => {
                let run = story(c, &self.llm).await?;
                Ok(json!({
                    "model_calls": run.calls,
                    "scenes": run.story.steps.iter().map(|s| json!({
                        "title": s["title"], "decisions": s["decisions"] })).collect::<Vec<_>>(),
                }))
            }
            _ => {
                let who = self.who(a)?;
                let mut args = a.clone();
                if let Some(o) = args.as_object_mut() {
                    o.remove("as");
                }
                Tools { chain: c }.call(&who, name, &args).await
            }
        }
    }

    /// NEAR Intents: swap `who`'s fUSD balance in the Verifier for wNEAR
    /// with the house solver (both sign a NEP-413 `token_diff`; the keeper
    /// submits them together), then withdraw the wNEAR to `who`'s account.
    async fn cash_out(&self, a: &Value) -> Result<Value, String> {
        let c = &self.chain;
        let who = self.who(a)?;
        let (v, usdc) = (&self.intents, c.usdc.to_string());
        let wnear = v.wnear.to_string();
        let have = v.balance(c, who.id.as_str(), &usdc).await;
        let amount = match a["amount"].as_str() {
            Some(x) => x
                .parse::<u128>()
                .map_err(|_| "amount: fUSD with 6 decimals, as a string")?,
            None => have,
        };
        if amount == 0 || amount > have {
            return Err(format!(
                "{} holds {have} fUSD units in the Verifier; route a job's pay there first (payout_via as the worker, via = {}) and let it settle",
                who.id, v.id
            ));
        }
        let get = amount * SOLVER_YOCTO_PER_UNIT;
        // Adding a key twice is refused; that only means it is already there.
        let _ = v.add_key(c, &who).await;
        let n = self
            .nonces
            .fetch_add(2, std::sync::atomic::Ordering::SeqCst);
        let nonce = |k: u64| {
            use sha2::Digest;
            let h: [u8; 32] = sha2::Sha256::digest(format!("{}:{}", who.id, n + k)).into();
            h
        };
        let deadline = verifier::deadline_in(c, 600).await;
        let mine = v
            .sign_diff(
                &who,
                json!({ nep141(&usdc): format!("-{amount}"), nep141(&wnear): get.to_string() }),
                nonce(0),
                &deadline,
            )
            .await?;
        let theirs = v
            .sign_diff(
                &self.solver,
                json!({ nep141(&usdc): amount.to_string(), nep141(&wnear): format!("-{get}") }),
                nonce(1),
                &deadline,
            )
            .await?;
        v.execute(c, &self.team.keeper, vec![mine, theirs]).await?;
        c.call(
            &self.solver,
            &v.wnear,
            "storage_deposit",
            json!({ "account_id": who.id, "registration_only": true }),
            NearToken::from_millinear(13),
        )
        .await?;
        v.withdraw(c, &who, &wnear, get).await?;
        let held = c
            .view(&v.wnear, "ft_balance_of", json!({ "account_id": who.id }))
            .await?;
        Ok(json!({
            "swapped": { "gave_fusd_units": amount.to_string(), "got_wnear_yocto": get.to_string(), "rate": "1 fUSD = 0.2 wNEAR (house solver, test rate)" },
            "how": "your NEP-413 token_diff and the solver's opposite one ran in one execute_intents call on the NEAR Intents Verifier: both settle or neither does; then ft_withdraw sent the wNEAR to your account",
            "verifier_fusd_left": v.balance(c, who.id.as_str(), &usdc).await.to_string(),
            "wnear_in_your_account": held,
        }))
    }

    /// The round (root job) whose awards include `job`.
    fn round_of(&self, job: &str) -> Result<String, String> {
        self.rounds
            .lock()
            .unwrap()
            .iter()
            .find(|(_, r)| r.hired.iter().any(|h| h.0 == job))
            .map(|(k, _)| k.clone())
            .ok_or_else(|| format!("{job} is not a part awarded in an open round"))
    }

    /// A guest that won a part puts a piece of it out for bids, paid out of
    /// its own pay; the house workers with that skill bid at once.
    async fn subcontract(&self, a: &Value) -> Result<Value, String> {
        let c = &self.chain;
        let tools = Tools { chain: c };
        let who = self.who(a)?;
        let job = a["job"].as_str().ok_or("missing job")?;
        let task = a["task"].as_str().ok_or("missing task")?;
        let skill = a["skill"].as_str().ok_or("missing skill")?.to_lowercase();
        if !SKILLS.contains(&skill.as_str()) {
            return Err(format!("skill: one of {SKILLS:?}"));
        }
        let root = self.round_of(job)?;
        let j = tools.call(&who, "get_job", &json!({ "job": job })).await?;
        if j["worker"].as_str() != Some(who.id.as_str()) {
            return Err(format!("only {job}'s worker can subcontract it"));
        }
        let max: u128 = a["max"]
            .as_str()
            .and_then(|m| m.parse().ok())
            .ok_or("max: fUSD with 6 decimals, as a string")?;
        let slot = tools
            .call(&who, "open_bids", &json!({ "job": job, "task": task, "max": max.to_string(), "secs": ROUND_BID_SECS }))
            .await?
            .as_u64()
            .ok_or("open_bids: no slot")?;
        for w in &self.team.workers {
            if w.skills.contains(&skill) && w.acc.id != who.id {
                let price = max * w.rate * HOUSE_MARGIN / 10_000;
                let _ = tools
                    .call(
                        &w.acc,
                        "bid",
                        &json!({ "job": job, "slot": slot, "price": price.to_string() }),
                    )
                    .await;
            }
        }
        if let Some(r) = self.rounds.lock().unwrap().get_mut(&root) {
            r.subs.push((job.to_string(), slot, task.to_string(), None));
        }
        let bids = tools
            .call(&who, "get_bids", &json!({ "job": job, "slot": slot }))
            .await?["bids"]
            .clone();
        Ok(
            json!({ "job": job, "slot": slot, "max": max.to_string(), "bids": bids,
            "note": "you are now a buyer: pick any bidder (check get_record first if you like) and call hire_sub" }),
        )
    }

    /// The contractor awards its sub-slot; the house winner works and the
    /// keeper judges it.
    async fn hire_sub(&self, a: &Value) -> Result<Value, String> {
        let c = &self.chain;
        let tools = Tools { chain: c };
        let who = self.who(a)?;
        let job = a["job"].as_str().ok_or("missing job")?.to_string();
        let slot = a["slot"].as_u64().ok_or("missing slot")?;
        let worker = a["worker"].as_str().ok_or("missing worker")?.to_string();
        let root = self.round_of(&job)?;
        let task = self
            .rounds
            .lock()
            .unwrap()
            .get(&root)
            .and_then(|r| {
                r.subs
                    .iter()
                    .find(|s| s.0 == job && s.1 == slot)
                    .map(|s| s.2.clone())
            })
            .ok_or("no such sub-slot: call subcontract first")?;
        c.fast_forward(ROUND_BID_SECS + 1).await;
        let bids = tools
            .call(&who, "get_bids", &json!({ "job": job, "slot": slot }))
            .await?;
        let price = bids["bids"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|b| b["worker"] == worker.as_str())
            .map(|b| num(&b["price"]))
            .ok_or("that account did not bid on this sub-slot")?;
        tools
            .call(
                &who,
                "award",
                &json!({ "job": job, "slot": slot, "worker": worker }),
            )
            .await?;
        let kids = tools.call(&who, "get_job", &json!({ "job": job })).await?["children"]
            .as_array()
            .map(|v| v.len())
            .unwrap_or(0);
        let sub = format!("t{kids}.{job}");
        let w = self
            .team
            .workers
            .iter()
            .find(|w| w.acc.id.as_str() == worker)
            .ok_or("only house workers do sub-parts in the playground")?;
        if w.junk {
            let bytes = b"lorem ipsum dolor sit amet, lorem ipsum dolor sit amet".to_vec();
            tools
                .call(
                    &w.acc,
                    "deliver",
                    &json!({ "job": sub, "resp": b64(&bytes) }),
                )
                .await?;
            self.delivered.lock().unwrap().insert(sub.clone(), bytes);
        } else {
            self.do_work(&json!({ "as": w.name, "job": sub, "task": task }))
                .await?;
        }
        let check = self
            .judge_check(&json!({ "job": sub, "task": task }))
            .await?;
        let mine = tools.call(&who, "get_job", &json!({ "job": job })).await?;
        if let Some(r) = self.rounds.lock().unwrap().get_mut(&root) {
            if let Some(s) = r.subs.iter_mut().find(|s| s.0 == job && s.1 == slot) {
                s.3 = Some(sub.clone());
            }
        }
        // Hiring carved the price out of the contractor's own pay: what is
        // left is its margin (a refund from the sub-job comes back to it).
        let left = num(&mine["amount"]);
        Ok(
            json!({ "sub_job": sub, "hired": worker, "price_fusd": price as f64 / USD as f64,
            "your_pay_now_fusd": left as f64 / USD as f64, "margin_fusd": left as f64 / USD as f64,
            "score": check["score"],
            "next": "deliver your own part (do_work on your job), judge_check, then finish_round: the sub-job settles first and its refund, if any, comes back to you" }),
        )
    }

    /// A worker does the task through the (test) model and delivers the
    /// raw response with the model's signature over the request it made.
    async fn do_work(&self, a: &Value) -> Result<Value, String> {
        let who = self.who(&json!({ "as": a["as"].as_str().unwrap_or("researcher") }))?;
        let job = a["job"].as_str().ok_or("missing job")?;
        let task = a["task"]
            .as_str()
            .ok_or("missing task (the exact text the job was hired for)")?;
        let j = Tools { chain: &self.chain }
            .call(&who, "get_job", &json!({ "job": job }))
            .await?;
        let seq = j["seq"].as_u64().unwrap_or(0) + 1;
        let spec = ht_core::receipt::sha256_hex(task.as_bytes());
        let body = chat_body(
            MODEL,
            WORKER,
            &format!("HT1:{job}:{spec}:{seq} {task}"),
            300,
        );
        let r = match a["text"].as_str() {
            Some(t) => {
                let t = t.to_string();
                Llm::Mock(Mock::new(MODEL, Box::new(move |_| t.clone())))
                    .chat(&body)
                    .await?
            }
            None => self.llm.chat(&body).await?,
        };
        let res = Tools { chain: &self.chain }
            .call(
                &who,
                "deliver",
                &json!({ "job": job, "resp": b64(&r.body), "req": b64(&body),
                "sig": r.sig, "model": MODEL }),
            )
            .await?;
        self.delivered
            .lock()
            .unwrap()
            .insert(job.to_string(), r.body.clone());
        Ok(json!({ "delivered": r.content(), "result": res,
            "note": "the contract checked the HT1 tag and the model signature; now run judge_check" }))
    }

    /// The keeper: ask the model the contract's own check question about
    /// the last delivery and submit the signed answer.
    async fn judge_check(&self, a: &Value) -> Result<Value, String> {
        let keeper = self.who(&json!({ "as": a["as"].as_str().unwrap_or("keeper") }))?;
        let job = a["job"].as_str().ok_or("missing job")?;
        let task = a["task"].as_str().ok_or("missing task")?;
        let panel = a["panel"].as_u64().unwrap_or(0);
        let tools = Tools { chain: &self.chain };
        // The bytes the last delivery carried: do_work's raw model
        // response, or the text given (for a plain `deliver` of text).
        let bytes = match (a["deliverable"].as_str(), self.delivered.lock().unwrap().get(job)) {
            (Some(d), _) => d.as_bytes().to_vec(),
            (None, Some(b)) => b.clone(),
            (None, None) => return Err("nothing delivered through do_work on this job: pass `deliverable` (the exact text delivered)".into()),
        };
        let d64 = b64(&bytes);
        let req = tools
            .call(
                &keeper,
                "check_request",
                &json!({ "job": job, "panel": panel, "task": task, "deliverable": d64 }),
            )
            .await?;
        let body = base64::engine::general_purpose::STANDARD
            .decode(req.as_str().ok_or("check_request")?)
            .map_err(|e| e.to_string())?;
        let r = self.llm.chat(&body).await?;
        let score = tools
            .call(
                &keeper,
                "submit_check",
                &json!({ "job": job, "panel": panel, "task": task,
                "deliverable": d64, "answer": b64(&r.body), "sig": r.sig }),
            )
            .await?;
        Ok(json!({ "score": score, "judge_answer": r.content(),
            "note": format!("the round closes {CHECK_WINDOW_SECS} s after the delivery: wait, then finalize_check") }))
    }
}

/// Plain-English errors for the cases an agent meets most.
pub fn tidy(e: &str) -> String {
    if e.contains("UnknownAccount")
        || e.contains("unknown_account")
        || e.contains("AccountDoesNotExist")
    {
        return "that account does not exist: a settled job pays out and deletes itself, so it is gone (see get_record for the worker's outcome)".into();
    }
    if let Some(i) = e.find("panic_msg: \"") {
        let rest = &e[i + 12..];
        return format!(
            "refused by the contract: {}",
            &rest[..rest.find('"').unwrap_or(rest.len())]
        );
    }
    e.replace("Smart contract panicked: ", "refused by the contract: ")
}

/// The tools a judge sees: the shared tool list (with `as`) plus the
/// playground's own.
pub fn tool_list() -> Vec<Value> {
    let mut v: Vec<Value> = list()
        .into_iter()
        .map(|(name, description, mut schema)| {
            schema["properties"]["as"] = json!({ "type": "string", "description": "which demo account acts (default client)" });
            if name == "find_agents" {
                // Here the catalog defaults to every demo account.
                schema["required"] = json!([]);
                let d = format!("{description} Leave out `agents` for every demo account.");
                return json!({ "name": name, "description": d, "inputSchema": schema });
            }
            json!({ "name": name, "description": description, "inputSchema": schema })
        })
        .collect();
    let s = |props: Value, req: &[&str]| json!({ "type": "object", "properties": props, "required": req });
    let str_ = json!({ "type": "string" });
    for (name, desc, schema) in [
        ("guide", "Start here: what this is, the accounts, and a 15-call walk-through.", s(json!({}), &[])),
        ("accounts", "Every demo account (use its name in `as`) with its fUSD balance; the factory and token.", s(json!({}), &[])),
        ("wait", "Let `secs` of chain time pass on the local node (bids close after their secs, check rounds after 300 s).", s(json!({ "secs": { "type": "integer" } }), &["secs"])),
        ("do_work", "Worker (`as`, default researcher): do a hired job's task through the test model and deliver the response with the model's signature; `text` overrides the answer (try filler).",
         s(json!({ "job": str_, "task": str_, "text": str_, "as": str_ }), &["job", "task"])),
        ("judge_check", "Keeper: send the contract's own check question about a delivery to the test model and submit its signed answer; returns the score.",
         s(json!({ "job": str_, "task": str_, "deliverable": str_, "panel": { "type": "integer" }, "as": str_ }), &["job", "task"])),
        ("join", "Open market: your own agent joins with its own funded account (name, skills from research/translate/summary). Then act with `as` = name.",
         s(json!({ "name": str_, "skills": { "type": "array", "items": str_ } }), &["name", "skills"])),
        ("open_round", "Open market: the house client posts a 70 fUSD job and the lead puts 3 parts out for bids; house workers bid (the junk bot undercuts). Bid with `bid` as your guest, then close_round.", s(json!({}), &[])),
        ("close_round", "Open market: bidding closes; the lead picks each winner by price and on-chain record and awards it. If you won, do_work and judge_check, then finish_round.", s(json!({ "job": str_ }), &["job"])),
        ("finish_round", "Open market: close checks, pay good work early, fold the tree; returns every worker's fUSD and record (which steers the next round).", s(json!({ "job": str_ }), &["job"])),
        ("subcontract", "Open market, after you won a part: become a contractor. Put a piece of your own job out for bids, paid out of your own pay (`max` below your price, fUSD with 6 decimals); house workers with `skill` bid at once. Then hire_sub.",
         s(json!({ "as": str_, "job": str_, "task": str_, "skill": str_, "max": str_ }), &["as", "job", "task", "skill", "max"])),
        ("hire_sub", "Open market, contractor: bidding on your sub-slot closes; you award `worker` (any bidder, at its own bid). The account t<n>.<your job> is created holding exactly that bid; the house worker does the work and the keeper judges it. You keep the margin. Then do_work on your own job, judge_check, finish_round.",
         s(json!({ "as": str_, "job": str_, "slot": { "type": "integer" }, "worker": str_ }), &["as", "job", "slot", "worker"])),
        ("cash_out", "NEAR Intents: after a job paid you into the Verifier (set `payout_via` on your job as its worker, via = `intents_verifier` from `accounts`, before it settles), swap that fUSD for wNEAR with the house solver (signed NEP-413 intents, atomic) and withdraw the wNEAR to your account. `amount` (fUSD with 6 decimals) defaults to all of it.",
         s(json!({ "as": str_, "amount": str_ }), &["as"])),
        ("run_story", "Run the whole scripted three-job story (bidding, a junk bot losing work by its on-chain record, three clients pooling money); returns every scene and agent decision. A few minutes.", s(json!({}), &[])),
    ] {
        v.push(json!({ "name": name, "description": desc, "inputSchema": schema }));
    }
    v
}

/// Serve on stdin/stdout. The node boots on the first tool call, so the
/// client's handshake is instant.
pub async fn serve() {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    // scripts/mcp-local.sh moves stdout to fd 3 and points fd 1 at stderr,
    // so nothing the local node prints can break the JSON-RPC stream.
    let mut out: Box<dyn tokio::io::AsyncWrite + Unpin + Send> =
        match std::env::var("HT_MCP_OUT_FD")
            .ok()
            .and_then(|f| f.parse::<i32>().ok())
        {
            Some(fd) => {
                use std::os::fd::FromRawFd;
                // SAFETY: the launcher opened this fd for us and nothing else uses it.
                let f = unsafe { std::fs::File::from_raw_fd(fd) };
                Box::new(tokio::fs::File::from_std(f))
            }
            None => Box::new(tokio::io::stdout()),
        };
    let mut pg: Option<Playground> = None;
    while let Ok(Some(line)) = lines.next_line().await {
        let msg: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                let r = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } });
                let _ = out.write_all(format!("{r}\n").as_bytes()).await;
                continue;
            }
        };
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let method = msg["method"].as_str().unwrap_or_default();
        let result = match method {
            "initialize" => json!({
                "protocolVersion": PROTOCOL,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "hire-tree-playground", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Hire Tree on a local NEAR node. Call the `guide` tool first. The first call boots the node (about a minute).",
            }),
            "ping" => json!({}),
            "tools/list" => json!({ "tools": tool_list() }),
            "tools/call" => {
                let name = msg["params"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
                if pg.is_none() && name != "guide" {
                    eprintln!(
                        "hiretree: starting the local NEAR node and deploying (about a minute)"
                    );
                    pg = Some(Playground::boot().await);
                }
                let r = match (&pg, name.as_str()) {
                    (_, "guide") => Ok(json!(GUIDE)),
                    // A panic in one call (a hostile argument) is an error
                    // for that call, not the end of the server.
                    (Some(p), _) => {
                        use futures::FutureExt;
                        std::panic::AssertUnwindSafe(p.call(&name, &args))
                            .catch_unwind()
                            .await
                            .unwrap_or_else(|_| Err(format!("{name} failed: internal error")))
                    }
                    (None, _) => unreachable!(),
                };
                let (text, err) = match r {
                    Ok(Value::String(s)) => (s, false),
                    Ok(v) => (v.to_string(), false),
                    Err(e) => (tidy(&e), true),
                };
                json!({ "content": [{ "type": "text", "text": text }], "isError": err })
            }
            _ => {
                let r = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("unknown method {method}") } });
                let _ = out.write_all(format!("{r}\n").as_bytes()).await;
                let _ = out.flush().await;
                continue;
            }
        };
        let r = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        let _ = out.write_all(format!("{r}\n").as_bytes()).await;
        let _ = out.flush().await;
    }
}

/// Deploy the NEAR Intents Verifier and fund the house solver with wNEAR
/// inside it.
async fn house_solver(c: &Chain) -> (Verifier, Acc) {
    let v = verifier::deploy(c).await.unwrap();
    let solver = c.account("solver", 30).await;
    if v.balance(c, solver.id.as_str(), v.wnear.as_str()).await == 0 {
        for who in [solver.id.as_str(), v.id.as_str()] {
            c.call(
                &solver,
                &v.wnear,
                "storage_deposit",
                json!({ "account_id": who, "registration_only": true }),
                NearToken::from_millinear(13),
            )
            .await
            .unwrap();
        }
        c.call(
            &solver,
            &v.wnear,
            "near_deposit",
            json!({}),
            NearToken::from_yoctonear(SOLVER_WNEAR),
        )
        .await
        .unwrap();
        c.call(
            &solver,
            &v.wnear,
            "ft_transfer_call",
            json!({ "receiver_id": v.id, "amount": SOLVER_WNEAR.to_string(), "msg": solver.id }),
            NearToken::from_yoctonear(1),
        )
        .await
        .unwrap();
        let _ = v.add_key(c, &solver).await;
    }
    (v, solver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_playground_catalog_needs_no_agent_list() {
        let t = tool_list();
        let f = t
            .iter()
            .find(|x| x["name"] == "find_agents")
            .expect("find_agents listed");
        assert_eq!(f["inputSchema"]["required"], json!([]));
        assert!(f["description"]
            .as_str()
            .unwrap()
            .contains("every demo account"));
    }
}
