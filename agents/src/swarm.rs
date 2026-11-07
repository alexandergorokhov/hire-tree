//! `hiretree swarm`: a recursive labor market on a local node. Three leads
//! with different buying rules post jobs every round and compete for the
//! same workers (each worker takes one part per round). Worker agents with
//! different strategies bid; a contractor that wins a part becomes a buyer
//! itself: it puts a sub-part out for bids on its own job account, pays for
//! it out of its own pay and keeps the margin, so trees grow 3 levels deep.
//! Every bid stakes a small NEAR bond (a junk winner loses it to its
//! buyer); the quality lead runs sealed bids (commit, then reveal). The
//! factory keeps a 0.5% platform fee on each posted budget.
//! The contract judges every delivery and records accumulate on-chain.
//! Nobody scripts who wins. The output (`web/swarm.json`) shows what
//! emerges: prices per lead, tree depth, margins, earnings, junk priced out.

use crate::agents::{b64, WORKER};
use crate::chain::{num, Acc, Chain, USD};
use crate::judge::MODEL;
use crate::llm::{chat_body, Llm};
use crate::tools::Tools;
use base64::Engine;
use serde_json::{json, Value};

/// How an agent bids and works.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Strategy {
    /// Bids 90% of max, 5 less after every 3 straight losses; good work.
    Steady,
    /// Bids 65% of max, 5 less after every 3 straight losses; good work.
    Lean,
    /// Bids 100% of max and never cuts; leaves after 6 straight losses.
    Greedy,
    /// Starts at 80%; +5% after a win, -10% after a loss (never below cost); good work.
    Adaptive,
    /// Bids 45% of max; filler, no model call.
    Junk,
    /// Bids 60%, 5 less after every 3 straight losses; filler on every
    /// third job it wins.
    Sloppy,
    /// Bids just under the last winning price it saw (never below cost); good work.
    Undercut,
    /// Asks the model for its price each round, given the recent winning
    /// prices, its own record and its losing streak; good work.
    Model,
    /// Undercuts like `Undercut` but can go lower (45%): it only assembles,
    /// and buys the sub-part (sources, glossary, fetch) from a cheaper
    /// specialist out of its own pay, keeping the margin.
    Contractor,
}

impl Strategy {
    pub fn name(self) -> &'static str {
        match self {
            Strategy::Steady => "steady",
            Strategy::Lean => "lean",
            Strategy::Greedy => "greedy",
            Strategy::Adaptive => "adaptive",
            Strategy::Junk => "junk",
            Strategy::Sloppy => "sloppy",
            Strategy::Undercut => "undercut",
            Strategy::Model => "model",
            Strategy::Contractor => "contractor",
        }
    }
}

pub struct Agent {
    pub name: String,
    pub acc: Acc,
    pub skill: &'static str,
    pub strategy: Strategy,
    /// Current asking rate, percent of the skill's max.
    pub rate: u128,
    pub wins: u32,
    /// A junk bot that gave up (never paid, and each new account costs).
    pub gone: bool,
    pub respawned: u32,
    /// Rounds bid and lost in a row.
    pub lost: u32,
    pub won_last: bool,
    /// The model bidder's reason for its last ask.
    pub why: String,
    /// fUSD earned on accounts it no longer uses (junk respawns).
    pub earned_before: u128,
    /// Bonds it lost to buyers (junk work).
    pub bonds_lost: u32,
}

pub const MARKET_BIDDER: &str = "ROLE: market bidder. You sell one skill in an open market of AI agents. \
Reply with JSON only: {\"rate\": <your ask, percent of the part's max>, \"why\": \"<one short sentence>\"}. \
Asking below cost_floor loses money; you may do it (never below 40) only to win a first job and start a record. The buyer picks the lowest ask after a penalty for unpaid jobs on the record.";

/// The model bidder's prompt: what it knows about the market this round.
pub fn market_prompt(
    skill: &str,
    recent: &[u128],
    jobs: u64,
    paid: u64,
    last: u128,
    lost: u32,
    won_last: bool,
) -> String {
    json!({ "skill": skill, "cost_floor": COST_FLOOR, "recent_winning_rates": recent,
        "my_record": { "jobs": jobs, "paid": paid }, "my_last_ask": last,
        "lost_in_a_row": lost, "won_last_round": won_last })
    .to_string()
}

/// The model bidder may go this low (below cost) to win its first job.
pub const ENTRY_FLOOR: u128 = 40;

/// Reads the model's answer; out-of-range or unreadable answers fall back
/// to the last ask, clamped to ENTRY_FLOOR..=100.
pub fn read_rate(answer: &str, last: u128) -> (u128, String) {
    let v: Value = answer
        .find('{')
        .and_then(|i| answer.rfind('}').map(|j| &answer[i..=j]))
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or(Value::Null);
    let rate = v["rate"].as_f64().map(|r| r as u128).unwrap_or(last);
    (
        rate.clamp(ENTRY_FLOOR, 100),
        v["why"].as_str().unwrap_or("").chars().take(160).collect(),
    )
}

/// A junk bot comes back under a fresh account this many times, then quits.
pub const JUNK_RESPAWNS: u32 = 2;

/// The lowest rate (percent of max) at which an honest worker still covers
/// its model call and time. Junk does no work, so it can go lower.
pub const COST_FLOOR: u128 = 55;

/// A contractor only assembles and buys the sub-part cheaper, so its floor
/// is lower.
pub const CONTRACTOR_FLOOR: u128 = 45;

/// A contractor offers at most this share of its own price for the sub-part.
pub const SUB_SHARE: u128 = 60;

/// The newcomer premium a buyer (lead or contractor) learns from its own
/// history (percent): a failed newcomer adds 100 (cap 150), a newcomer that
/// delivered halves it, and a round with no newcomer hired lets it fade by
/// a fifth.
pub fn next_premium(p: u128, newcomer_failed: Option<bool>) -> u128 {
    match newcomer_failed {
        Some(true) => (p + 100).min(150),
        Some(false) => p / 2,
        None => p * 4 / 5,
    }
}

/// The parts a lead buys, with their max price in fUSD.
pub const SKILLS: [(&str, u128); 3] = [("research", 30), ("translate", 20), ("summary", 10)];

/// What a contractor of `skill` may buy from a specialist, and that
/// specialist's max price in fUSD. Research needs sources, sources need a
/// fetch, translation needs a glossary: trees up to 3 levels below the root.
pub const SUB_SKILLS: [(&str, &str, u128); 3] = [
    ("research", "sources", 12),
    ("translate", "glossary", 8),
    ("sources", "fetch", 5),
];

/// The nominal max (fUSD) of any skill.
pub fn nominal_max(skill: &str) -> u128 {
    SKILLS
        .iter()
        .find(|s| s.0 == skill)
        .map(|s| s.1)
        .or_else(|| SUB_SKILLS.iter().find(|s| s.1 == skill).map(|s| s.2))
        .unwrap_or(10)
}

pub fn sub_skill(skill: &str) -> Option<(&'static str, u128)> {
    SUB_SKILLS.iter().find(|s| s.0 == skill).map(|s| (s.1, s.2))
}

/// One buyer's rules. Leads differ, so prices form on both sides.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    /// Penalty for an all-unpaid record (150 = x2.5).
    pub record_weight: u128,
    /// Max offered, percent of each part's max.
    pub max_pct: u128,
}

/// Three leads: a price buyer, a quality buyer (weighs records heavily),
/// and a budget buyer (offers only 75% of max).
pub const LEADS: [(&str, Rules); 3] = [
    (
        "price",
        Rules {
            record_weight: 150,
            max_pct: 100,
        },
    ),
    (
        "quality",
        Rules {
            record_weight: 400,
            max_pct: 100,
        },
    ),
    (
        "budget",
        Rules {
            record_weight: 150,
            max_pct: 75,
        },
    ),
];

/// A contractor buys like a price lead.
pub const CONTRACTOR_RULES: Rules = Rules {
    record_weight: 150,
    max_pct: 100,
};

/// The buyer's pick: the lowest bid after a penalty for the share of unpaid
/// jobs on the bidder's on-chain record (`record_weight`: 150 = x2.5 when
/// all unpaid), and for a bidder with no record yet, a newcomer premium
/// (percent) the buyer learned. Ties go to the bidder with more paid jobs.
/// Deterministic.
pub fn pick(
    cands: &[(usize, u128, u32, u32)],
    newcomer_pct: u128,
    record_weight: u128,
) -> Option<usize> {
    cands
        .iter()
        .min_by_key(|(i, bid, jobs, paid)| {
            let unpaid = jobs.saturating_sub(*paid) as u128;
            let mut score = bid * (100 + record_weight * unpaid / (*jobs as u128).max(1)) / 100;
            if *jobs == 0 {
                score = score * (100 + newcomer_pct) / 100;
            }
            (score, std::cmp::Reverse(*paid), *i)
        })
        .map(|c| c.0)
}

pub fn roster() -> Vec<(String, &'static str, Strategy)> {
    use Strategy::*;
    let per: [(&str, &[Strategy]); 6] = [
        (
            "research",
            &[
                Steady, Lean, Greedy, Adaptive, Junk, Sloppy, Undercut, Model, Contractor,
            ],
        ),
        (
            "translate",
            &[
                Steady, Lean, Greedy, Adaptive, Junk, Undercut, Model, Contractor,
            ],
        ),
        ("summary", &[Steady, Lean, Adaptive, Junk, Undercut, Model]),
        ("sources", &[Steady, Lean, Junk, Contractor]),
        ("glossary", &[Steady, Lean, Junk]),
        ("fetch", &[Steady, Lean, Junk]),
    ];
    let mut v = vec![];
    for (skill, sts) in per {
        for st in sts {
            v.push((format!("{}-{}", &skill[..3], st.name()), skill, *st));
        }
    }
    v
}

fn initial_rate(s: Strategy) -> u128 {
    match s {
        Strategy::Steady => 90,
        Strategy::Lean => 65,
        Strategy::Greedy => 100,
        Strategy::Adaptive => 80,
        Strategy::Junk => 45,
        Strategy::Sloppy => 60,
        Strategy::Undercut => 75,
        Strategy::Model => 75,
        Strategy::Contractor => 70,
    }
}

/// A small, fixed per-agent spread (-3..=+3 points) so agents with the same
/// strategy do not bid in lockstep.
fn jitter(name: &str) -> i64 {
    (name.bytes().map(|b| b as i64).sum::<i64>() % 7) - 3
}

const BID_SECS: u64 = 300;
const WINDOW_SECS: u64 = 5400;
/// Platform fee the factory keeps on each posted budget in this run.
pub const SWARM_FEE_BPS: u16 = 50;
/// Every slot asks each bidder to stake 0.05 NEAR: back to losers at the
/// award and to the winner when paid; the buyer keeps a junk winner's bond.
pub const BOND_YOCTO: u128 = 50 * 10u128.pow(21);
/// The quality lead runs sealed bids (commit, then reveal).
pub fn sealed_lead(l: usize) -> bool {
    LEADS[l].0 == "quality"
}

/// Who buys a part.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Buyer {
    Lead(usize),
    Agent(usize),
}

/// One part put out for bids in a round, at any level of a tree.
struct Part {
    lead: usize,
    root: String,
    /// The job account the slot is on (root, or a contractor's job).
    job: String,
    level: u32,
    parent: Option<usize>,
    buyer: Buyer,
    skill: &'static str,
    max: u128,
    task: String,
    slot: u64,
    sealed: bool,
    bids: Vec<(usize, u128)>,
    winner: Option<usize>,
    price: u128,
    newcomer: bool,
    premium_seen: u128,
    /// The child job account the winner works in.
    child: String,
    score: u64,
    passed: bool,
    paid_now: bool,
    settled: bool,
}

fn buyer_name(b: Buyer, agents: &[Agent]) -> String {
    match b {
        Buyer::Lead(l) => format!("lead-{}", LEADS[l].0),
        Buyer::Agent(i) => agents[i].name.clone(),
    }
}

/// Runs `rounds` rounds: each round every lead posts one job.
pub async fn run(c: &Chain, llm: &Llm, rounds: u32) -> Result<Value, String> {
    use std::collections::{HashMap, HashSet};
    let tools = Tools { chain: c };
    let mk = |n: String| async move { c.account(&n, 5).await };
    let hub = Acc {
        id: c.hub.clone(),
        signer: c.derived("hub"),
    };
    c.ok(
        &hub,
        &c.hub,
        "set_settings",
        json!({ "fee_bps": SWARM_FEE_BPS }),
    )
    .await;
    let treasury = mk("streasury".into()).await;
    let keeper = mk("skeeper".into()).await;
    c.register(&treasury).await;
    c.mint(&treasury.id, 400 * USD).await;
    let mut leads = vec![];
    for (name, _) in LEADS {
        let lead = mk(format!("slead-{name}")).await;
        let client = mk(format!("sclient-{name}")).await;
        c.register(&lead).await;
        c.register(&client).await;
        c.mint(&client.id, 70 * USD * rounds as u128).await;
        c.call(
            &client,
            &c.hub,
            "prepay_post",
            json!({}),
            near_api::types::NearToken::from_millinear(60 * rounds as u128),
        )
        .await?;
        leads.push((lead, client));
    }
    let mut agents = vec![];
    for (name, skill, strategy) in roster() {
        let rate0 = (initial_rate(strategy) as i64 + jitter(&name)).clamp(40, 100) as u128;
        let acc = mk(format!("w-{name}")).await;
        c.register(&acc).await;
        agents.push(Agent {
            name,
            acc,
            skill,
            strategy,
            rate: rate0,
            wins: 0,
            gone: false,
            respawned: 0,
            lost: 0,
            won_last: false,
            why: String::new(),
            earned_before: 0,
            bonds_lost: 0,
        });
    }
    let mut premium: HashMap<String, u128> = HashMap::new();
    let mut history: HashMap<&str, Vec<u128>> = HashMap::new();
    let mut respawns = 0u32;
    let mut log = vec![];
    let mut lead_spent = [0u128; 3];
    let mut lead_unfilled = [0u32; 3];
    for r in 1..=rounds {
        let mut parts: Vec<Part> = vec![];
        // Leads take turns picking first.
        let order: Vec<usize> = (0..3).map(|k| (k + r as usize - 1) % 3).collect();
        // 1. Every lead posts a job and puts its three parts out for bids.
        for &l in &order {
            let next = c.view(&c.hub, "get_settings", json!({})).await?["next"]
                .as_u64()
                .ok_or("next")?;
            let root = format!("job-{next}.{}", c.hub);
            let task = format!(
                "Round {r}, lead {}: explainer, Spanish translation and a summary.",
                LEADS[l].0
            );
            tools.call(&leads[l].1, "post_job", &json!({ "budget": (70 * USD).to_string(), "manager": leads[l].0.id,
                "task": task, "window_secs": WINDOW_SECS, "lead_fee": (5 * USD).to_string(), "treasury": treasury.id })).await?;
            for (skill, max) in SKILLS {
                let max = max * USD * LEADS[l].1.max_pct / 100;
                let ptask = format!("Round {r}, lead {}: the {skill} part.", LEADS[l].0);
                let slot = tools
                    .call(&leads[l].0, "open_bids", &json!({ "job": root, "task": ptask, "max": max.to_string(), "secs": BID_SECS,
                        "bond": BOND_YOCTO.to_string(), "sealed": sealed_lead(l) }))
                    .await?
                    .as_u64()
                    .ok_or("slot")?;
                parts.push(Part {
                    lead: l,
                    root: root.clone(),
                    job: root.clone(),
                    level: 1,
                    parent: None,
                    buyer: Buyer::Lead(l),
                    skill,
                    max,
                    task: ptask,
                    slot,
                    sealed: sealed_lead(l),
                    bids: vec![],
                    winner: None,
                    price: 0,
                    newcomer: false,
                    premium_seen: 0,
                    child: String::new(),
                    score: 0,
                    passed: false,
                    paid_now: false,
                    settled: false,
                });
            }
        }
        // 2. Every active agent sets its ask once for the round.
        for a in agents.iter_mut().filter(|a| !a.gone) {
            let recent: Vec<u128> = history
                .get(a.skill)
                .map(|h| h[h.len().saturating_sub(6)..].to_vec())
                .unwrap_or_default();
            match a.strategy {
                Strategy::Undercut | Strategy::Contractor => {
                    let floor = if a.strategy == Strategy::Contractor {
                        CONTRACTOR_FLOOR
                    } else {
                        COST_FLOOR
                    };
                    if let Some(w) = recent.last() {
                        a.rate = w.saturating_sub(3).max(floor);
                    }
                }
                Strategy::Model => {
                    let rec = tools
                        .call(&a.acc, "get_record", &json!({ "account": a.acc.id }))
                        .await?;
                    let prompt = market_prompt(
                        a.skill,
                        &recent,
                        rec["jobs"].as_u64().unwrap_or(0),
                        rec["paid"].as_u64().unwrap_or(0),
                        a.rate,
                        a.lost,
                        a.won_last,
                    );
                    let ans = llm
                        .chat(&chat_body(MODEL, MARKET_BIDDER, &prompt, 80))
                        .await?;
                    (a.rate, a.why) = read_rate(&ans.content(), a.rate);
                }
                _ => {}
            }
        }
        let mut booked: HashSet<usize> = HashSet::new();
        let mut bidders: HashSet<usize> = HashSet::new();
        let mut children: HashMap<String, u32> = HashMap::new();
        // 3. Bid and award level by level: a contractor that wins becomes a
        // buyer one level down.
        let mut level_start = 0;
        for level in 1..=3u32 {
            let level_parts: Vec<usize> = (level_start..parts.len()).collect();
            if level_parts.is_empty() {
                break;
            }
            for &p in &level_parts {
                let (job, slot, skill, max) = {
                    let q = &parts[p];
                    (q.job.clone(), q.slot, q.skill, q.max)
                };
                for (i, a) in agents.iter().enumerate() {
                    if a.gone || a.skill != skill || booked.contains(&i) {
                        continue;
                    }
                    if matches!(parts[p].buyer, Buyer::Agent(b) if b == i) {
                        continue;
                    }
                    let price = (nominal_max(skill) * USD * a.rate / 100).max(USD / 10);
                    if price > max {
                        continue;
                    }
                    let placed = if parts[p].sealed {
                        tools
                            .call(
                                &a.acc,
                                "bid_sealed",
                                &json!({ "job": job, "slot": slot, "price": price.to_string(),
                                "salt": format!("r{r}p{p}a{i}") }),
                            )
                            .await
                    } else {
                        tools
                            .call(
                                &a.acc,
                                "bid",
                                &json!({ "job": job, "slot": slot, "price": price.to_string() }),
                            )
                            .await
                    };
                    if placed.is_ok() {
                        parts[p].bids.push((i, price));
                        bidders.insert(i);
                    }
                }
            }
            c.fast_forward(BID_SECS + 1).await;
            // Sealed slots: every bidder reveals its committed price.
            let mut any_sealed = false;
            for &p in &level_parts {
                if !parts[p].sealed {
                    continue;
                }
                any_sealed = true;
                for k in 0..parts[p].bids.len() {
                    let (i, price) = parts[p].bids[k];
                    tools
                        .call(
                            &agents[i].acc,
                            "reveal",
                            &json!({ "job": parts[p].job, "slot": parts[p].slot,
                            "price": price.to_string(), "salt": format!("r{r}p{p}a{i}") }),
                        )
                        .await?;
                }
            }
            if any_sealed {
                c.fast_forward(BID_SECS + 1).await;
            }
            for &p in &level_parts {
                let buyer = parts[p].buyer;
                let bname = buyer_name(buyer, &agents);
                let rules = match buyer {
                    Buyer::Lead(l) => LEADS[l].1,
                    Buyer::Agent(_) => CONTRACTOR_RULES,
                };
                let mut cands = vec![];
                for &(i, price) in &parts[p].bids {
                    if booked.contains(&i) {
                        continue;
                    }
                    let rec = tools
                        .call(
                            &keeper,
                            "get_record",
                            &json!({ "account": agents[i].acc.id }),
                        )
                        .await?;
                    cands.push((
                        i,
                        price,
                        rec["jobs"].as_u64().unwrap_or(0) as u32,
                        rec["paid"].as_u64().unwrap_or(0) as u32,
                    ));
                }
                let prem = *premium.get(&bname).unwrap_or(&0);
                let Some(w) = pick(&cands, prem, rules.record_weight) else {
                    if let Buyer::Lead(l) = buyer {
                        lead_unfilled[l] += 1;
                    }
                    continue;
                };
                let (_, price, jobs, _) = *cands.iter().find(|x| x.0 == w).unwrap();
                let buyer_acc = match buyer {
                    Buyer::Lead(l) => leads[l].0.clone(),
                    Buyer::Agent(b) => agents[b].acc.clone(),
                };
                tools
                    .call(&buyer_acc, "award", &json!({ "job": parts[p].job, "slot": parts[p].slot, "worker": agents[w].acc.id }))
                    .await?;
                let k = children.entry(parts[p].job.clone()).or_insert(0);
                *k += 1;
                let child = format!("t{}.{}", k, parts[p].job);
                booked.insert(w);
                agents[w].wins += 1;
                history
                    .entry(parts[p].skill)
                    .or_default()
                    .push(price * 100 / (nominal_max(parts[p].skill) * USD));
                if let Buyer::Lead(l) = buyer {
                    lead_spent[l] += price;
                }
                {
                    let q = &mut parts[p];
                    q.winner = Some(w);
                    q.price = price;
                    q.newcomer = jobs == 0;
                    q.premium_seen = prem;
                    q.child = child.clone();
                }
                // A contractor becomes a buyer: it puts the sub-part out for
                // bids on its own job, paid out of its own pay.
                if agents[w].strategy == Strategy::Contractor && level < 3 {
                    if let Some((sub, sub_max)) = sub_skill(agents[w].skill) {
                        let max = (sub_max * USD).min(price * SUB_SHARE / 100);
                        let task = format!("{}: the {sub} for it.", parts[p].task);
                        let slot = tools
                            .call(&agents[w].acc, "open_bids", &json!({ "job": child, "task": task, "max": max.to_string(), "secs": BID_SECS,
                                "bond": BOND_YOCTO.to_string() }))
                            .await?
                            .as_u64()
                            .ok_or("sub slot")?;
                        let (lead, root) = (parts[p].lead, parts[p].root.clone());
                        parts.push(Part {
                            lead,
                            root,
                            job: child,
                            level: level + 1,
                            parent: Some(p),
                            buyer: Buyer::Agent(w),
                            skill: sub,
                            max,
                            task,
                            slot,
                            sealed: false,
                            bids: vec![],
                            winner: None,
                            price: 0,
                            newcomer: false,
                            premium_seen: 0,
                            child: String::new(),
                            score: 0,
                            passed: false,
                            paid_now: false,
                            settled: false,
                        });
                    }
                }
            }
            level_start = level_parts.last().unwrap() + 1;
        }
        // 4. Work, bottom-up: the deepest parts first, so a contractor's
        // sub-part is judged and paid before it delivers.
        for level in (1..=3u32).rev() {
            let ps: Vec<usize> = (0..parts.len())
                .filter(|&p| parts[p].level == level && parts[p].winner.is_some())
                .collect();
            if ps.is_empty() {
                continue;
            }
            for &p in &ps {
                let w = parts[p].winner.unwrap();
                let junk = match agents[w].strategy {
                    Strategy::Junk => true,
                    Strategy::Sloppy => agents[w].wins % 3 == 0,
                    _ => false,
                };
                let (child, task) = (parts[p].child.clone(), parts[p].task.clone());
                let delivered = if junk {
                    let b = b"lorem ipsum dolor sit amet, lorem ipsum dolor sit amet".to_vec();
                    tools
                        .call(
                            &agents[w].acc,
                            "deliver",
                            &json!({ "job": child, "resp": b64(&b) }),
                        )
                        .await?;
                    b
                } else {
                    let spec = ht_core::receipt::sha256_hex(task.as_bytes());
                    let body =
                        chat_body(MODEL, WORKER, &format!("HT1:{child}:{spec}:1 {task}"), 300);
                    let rep = llm.chat(&body).await?;
                    tools
                        .call(
                            &agents[w].acc,
                            "deliver",
                            &json!({ "job": child, "resp": b64(&rep.body), "req": b64(&body),
                            "sig": rep.sig, "model": MODEL }),
                        )
                        .await?;
                    rep.body
                };
                let req = tools
                    .call(&keeper, "check_request", &json!({ "job": child, "panel": 0, "task": task, "deliverable": b64(&delivered) }))
                    .await?;
                let qb = base64::engine::general_purpose::STANDARD
                    .decode(req.as_str().ok_or("check_request")?)
                    .map_err(|e| e.to_string())?;
                let ans = llm.chat(&qb).await?;
                let score = tools
                    .call(
                        &keeper,
                        "submit_check",
                        &json!({ "job": child, "panel": 0, "task": task,
                        "deliverable": b64(&delivered), "answer": b64(&ans.body), "sig": ans.sig }),
                    )
                    .await?;
                parts[p].score = score
                    .as_u64()
                    .or_else(|| score.as_str().and_then(|x| x.parse().ok()))
                    .unwrap_or(0);
                parts[p].passed = parts[p].score >= 6;
            }
            c.fast_forward(crate::agents::CHECK_WINDOW_SECS + 1).await;
            for &p in &ps {
                let child = parts[p].child.clone();
                tools
                    .call(&keeper, "finalize_check", &json!({ "job": child }))
                    .await?;
                if !parts[p].passed {
                    continue;
                }
                match parts[p].buyer {
                    // A contractor accepts its sub-agent's passed work: the
                    // sub-job settles now and pays it.
                    Buyer::Agent(b) => {
                        tools
                            .call(&agents[b].acc, "approve", &json!({ "job": child }))
                            .await?;
                        tools
                            .call(&keeper, "settle", &json!({ "job": child }))
                            .await?;
                        parts[p].paid_now = true;
                        parts[p].settled = true;
                    }
                    // The lead's treasury buys the note: paid now.
                    Buyer::Lead(l) => {
                        let q = tools
                            .call(&leads[l].0, "quote_note", &json!({ "job": child }))
                            .await?;
                        if !q.is_null() {
                            let price = q
                                .as_str()
                                .map(String::from)
                                .unwrap_or_else(|| q.to_string());
                            tools
                                .call(
                                    &treasury,
                                    "buy_note",
                                    &json!({ "job": child, "price": price }),
                                )
                                .await?;
                            tools
                                .call(&leads[l].0, "approve", &json!({ "job": child }))
                                .await?;
                            parts[p].paid_now = true;
                        }
                    }
                }
            }
        }
        // 5. Failed or blocked parts wait for the window; then every job
        // settles bottom-up and the roots close.
        if parts.iter().any(|q| q.winner.is_some() && !q.paid_now) {
            c.fast_forward(WINDOW_SECS + 1).await;
        }
        for level in (1..=3u32).rev() {
            for q in parts
                .iter_mut()
                .filter(|q| q.level == level && q.winner.is_some() && !q.settled)
            {
                tools
                    .call(&keeper, "settle", &json!({ "job": q.child }))
                    .await?;
                q.settled = true;
            }
        }
        for &l in &order {
            let root = parts.iter().find(|q| q.lead == l).unwrap().root.clone();
            let _ = tools
                .call(&leads[l].1, "approve", &json!({ "job": root }))
                .await;
            tools
                .call(&keeper, "settle", &json!({ "job": root }))
                .await?;
        }
        // 6. Learning: buyers update their newcomer premium; agents adapt.
        let mut learned: HashSet<String> = HashSet::new();
        for q in parts.iter().filter(|q| q.winner.is_some()) {
            if q.newcomer {
                let b = buyer_name(q.buyer, &agents);
                let p = premium.entry(b.clone()).or_insert(0);
                *p = next_premium(*p, Some(!q.passed));
                learned.insert(b);
            }
        }
        for (b, p) in premium.iter_mut() {
            if !learned.contains(b) {
                *p = next_premium(*p, None);
            }
        }
        for i in bidders.iter().copied() {
            let a = &mut agents[i];
            let won = booked.contains(&i);
            a.won_last = won;
            if won {
                a.lost = 0;
                if a.strategy == Strategy::Adaptive {
                    a.rate = (a.rate + 5).min(100);
                }
                continue;
            }
            a.lost += 1;
            match a.strategy {
                Strategy::Adaptive => a.rate = a.rate.saturating_sub(10).max(COST_FLOOR),
                Strategy::Steady | Strategy::Lean | Strategy::Sloppy if a.lost % 3 == 0 => {
                    a.rate = a.rate.saturating_sub(5).max(COST_FLOOR)
                }
                Strategy::Greedy if a.lost >= 6 => a.gone = true,
                _ => {}
            }
        }
        // A caught junk bot comes back under a fresh account (a Sybil): its
        // record resets, which is what the newcomer premium is for.
        for q in parts.iter().filter(|q| !q.passed) {
            if let Some(w) = q.winner {
                agents[w].bonds_lost += 1;
            }
        }
        let caught: Vec<usize> = parts
            .iter()
            .filter(|q| !q.passed)
            .filter_map(|q| q.winner)
            .filter(|&w| agents[w].strategy == Strategy::Junk)
            .collect();
        for w in caught {
            if agents[w].respawned >= JUNK_RESPAWNS {
                agents[w].gone = true;
                continue;
            }
            agents[w].respawned += 1;
            respawns += 1;
            agents[w].earned_before += c.ft(&agents[w].acc.id).await;
            let fresh = mk(format!("w-{}-{respawns}", agents[w].name)).await;
            c.register(&fresh).await;
            agents[w].acc = fresh;
        }
        let rows: Vec<Value> = parts
            .iter()
            .enumerate()
            .map(|(k, q)| {
                let w = q.winner;
                json!({ "id": k, "lead": LEADS[q.lead].0, "root": q.root, "level": q.level, "parent": q.parent,
                    "buyer": buyer_name(q.buyer, &agents), "skill": q.skill, "max": q.max as f64 / 1e6,
                    "winner": w.map(|w| agents[w].name.clone()), "strategy": w.map(|w| agents[w].strategy.name()),
                    "price": q.price as f64 / 1e6, "rate": if q.price > 0 { q.price * 100 / (nominal_max(q.skill) * USD) } else { 0 },
                    "newcomer": q.newcomer, "newcomer_premium": q.premium_seen, "score": q.score,
                    "paid": q.passed, "paid_now": q.paid_now, "job": q.child, "sealed": q.sealed,
                    "bids": q.bids.iter().map(|(i, p)| { let a = &agents[*i];
                        let mut b = json!({ "agent": a.name, "price": *p as f64 / 1e6 });
                        if a.strategy == Strategy::Model { b["why"] = json!(a.why); }
                        b }).collect::<Vec<_>>() })
            })
            .collect();
        eprintln!(
            "swarm round {r}/{rounds}: {}",
            parts
                .iter()
                .filter_map(|q| q.winner.map(|w| format!(
                    "{}{}:{}@{}",
                    "  ".repeat(q.level as usize - 1),
                    q.skill,
                    agents[w].name,
                    q.price as f64 / 1e6
                )))
                .collect::<Vec<_>>()
                .join(", ")
        );
        log.push(json!({ "round": r, "order": order.iter().map(|&l| LEADS[l].0).collect::<Vec<_>>(), "parts": rows }));
    }
    let mut table = vec![];
    for a in &agents {
        let rec = tools
            .call(&keeper, "get_record", &json!({ "account": a.acc.id }))
            .await?;
        let earned = a.earned_before + c.ft(&a.acc.id).await;
        table.push(json!({ "agent": a.name, "skill": a.skill, "strategy": a.strategy.name(), "wins": a.wins,
            "earned": earned as f64 / 1e6, "record": rec, "final_rate": a.rate, "quit": a.gone, "bonds_lost": a.bonds_lost }));
    }
    let settings = c.view(&c.hub, "get_settings", json!({})).await?;
    Ok(
        json!({ "schema": "hiretree-swarm-2", "network": c.network_name, "rounds": log, "agents": table,
        "leads": LEADS.iter().enumerate().map(|(l, (n, r))| json!({ "lead": n, "record_weight": r.record_weight,
            "max_pct": r.max_pct, "spent": lead_spent[l] as f64 / 1e6, "unfilled": lead_unfilled[l] })).collect::<Vec<_>>(),
        "junk_respawns": respawns, "premiums": premium,
        "platform_fee_bps": SWARM_FEE_BPS, "bond_near": BOND_YOCTO as f64 / 1e24, "sealed_lead": "quality", "platform_fees": num(&settings["fees"]) as f64 / 1e6,
        "model": "scripted test model (not NEAR AI) for work, judging and the model bidder; agents adapt their asks; buyers pick by price, on-chain record and a learned newcomer premium" }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_takes_the_lowest_bid_with_a_clean_record() {
        assert_eq!(
            pick(&[(0, 27, 0, 0), (1, 19, 0, 0), (2, 30, 0, 0)], 0, 150),
            Some(1)
        );
    }

    #[test]
    fn unpaid_jobs_cost_more_than_a_low_bid_saves() {
        // 13.5 with 2 of 2 unpaid weighs 54; 27 with a clean record wins.
        assert_eq!(pick(&[(0, 27, 3, 3), (1, 13, 2, 0)], 0, 150), Some(0));
        // One unpaid job in four is a smaller penalty: 19 weighs ~26 and still wins.
        assert_eq!(pick(&[(0, 27, 3, 3), (1, 19, 4, 3)], 0, 150), Some(1));
    }

    #[test]
    fn burned_leads_charge_newcomers_a_premium() {
        // A fresh account bidding 13 beats 18 with no premium...
        assert_eq!(pick(&[(0, 18, 5, 5), (1, 13, 0, 0)], 0, 150), Some(1));
        // ...but not once half the newcomers it hired failed (+75%).
        assert_eq!(pick(&[(0, 18, 5, 5), (1, 13, 0, 0)], 75, 150), Some(0));
    }

    #[test]
    fn the_newcomer_premium_rises_on_failure_and_fades() {
        assert_eq!(next_premium(0, Some(true)), 100);
        assert_eq!(next_premium(100, Some(true)), 150);
        assert_eq!(next_premium(150, Some(false)), 75);
        assert_eq!(next_premium(150, None), 120);
        // Junk at 45% with a 150% premium weighs 112; it takes several
        // quiet rounds before a respawned bot is cheap enough to try again.
        let mut p = 150;
        let mut rounds = 0;
        while 45 * (100 + p) / 100 >= COST_FLOOR + 5 {
            p = next_premium(p, None);
            rounds += 1;
        }
        assert!(rounds >= 5, "{rounds}");
    }

    #[test]
    fn ties_go_to_the_longer_clean_record() {
        assert_eq!(pick(&[(0, 18, 1, 1), (1, 18, 4, 4)], 0, 150), Some(1));
    }

    #[test]
    fn the_model_bidders_answer_is_read_safely() {
        assert_eq!(read_rate(r#"{"rate": 62, "why": "median"}"#, 70).0, 62);
        // Prose around the JSON is fine; below cost or above max is clamped.
        assert_eq!(read_rate(r#"Sure: {"rate": 20}"#, 70).0, ENTRY_FLOOR);
        assert_eq!(read_rate(r#"{"rate": 400}"#, 70).0, 100);
        // Unreadable: keep the last ask.
        assert_eq!(read_rate("no idea", 70), (70, String::new()));
    }

    #[test]
    fn the_scripted_market_bidder_follows_the_market() {
        let ask = |recent: &[u128], lost: u32, won: bool| {
            let body = chat_body(
                MODEL,
                MARKET_BIDDER,
                &market_prompt("research", recent, 3, 3, 70, lost, won),
                80,
            );
            let req: Value = serde_json::from_slice(&body).unwrap();
            read_rate(&crate::agents::mock_script(&req), 70).0
        };
        assert_eq!(ask(&[], 0, false), 75);
        assert_eq!(ask(&[60, 64, 70], 0, false), 64);
        assert_eq!(ask(&[60, 64, 70], 2, false), 60);
        assert_eq!(ask(&[60, 64, 70], 0, true), 67);
        assert_eq!(ask(&[55, 55, 56], 3, false), COST_FLOOR);
        // No record and losing: a loss-leader bid to start one.
        let body = chat_body(
            MODEL,
            MARKET_BIDDER,
            &market_prompt("research", &[55, 55], 0, 0, 55, 3, false),
            80,
        );
        let req: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(read_rate(&crate::agents::mock_script(&req), 55).0, 50);
    }

    #[test]
    fn a_quality_lead_weighs_the_record_more() {
        // 13 with 1 of 8 jobs unpaid vs 16 with a clean record: the price
        // lead weighs 13 as 15.3 and takes it; the quality lead weighs it
        // as 19.5 and pays 16.
        let c = [(0, 16, 8, 8), (1, 13, 8, 7)];
        assert_eq!(pick(&c, 0, 150), Some(1));
        assert_eq!(pick(&c, 0, 400), Some(0));
    }

    #[test]
    fn trees_go_three_levels_deep() {
        assert_eq!(sub_skill("research"), Some(("sources", 12)));
        assert_eq!(sub_skill("sources"), Some(("fetch", 5)));
        assert_eq!(sub_skill("fetch"), None);
        assert_eq!(sub_skill("summary"), None);
        assert_eq!(nominal_max("glossary"), 8);
        // A contractor offers at most 60% of its own price for the sub-part.
        assert_eq!((12 * USD).min(15 * USD * SUB_SHARE / 100), 9 * USD);
    }

    #[test]
    fn roster_has_every_strategy_for_each_skill() {
        let r = roster();
        assert_eq!(r.len(), 33);
        for (skill, _) in SKILLS {
            assert!(r.iter().any(|x| x.1 == skill && x.2 == Strategy::Junk));
        }
        // Every sub-skill has a seller, and junk to watch out for.
        for (_, sub, _) in SUB_SKILLS {
            assert!(r.iter().any(|x| x.1 == sub && x.2 == Strategy::Junk));
            assert!(r.iter().any(|x| x.1 == sub && x.2 != Strategy::Junk));
        }
        // Contractors exist for every skill that has a sub-part.
        for (skill, _, _) in SUB_SKILLS {
            assert!(r
                .iter()
                .any(|x| x.1 == skill && x.2 == Strategy::Contractor));
        }
    }
}
