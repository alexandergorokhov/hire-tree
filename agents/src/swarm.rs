//! `hiretree swarm`: a recursive labor market on a local node. Three leads
//! with different buying rules post jobs every round and compete for the
//! same workers (each worker takes one part per round). Worker agents with
//! different strategies bid; a contractor that wins a part becomes a buyer
//! itself: it puts a sub-part out for bids on its own job account, pays for
//! it out of its own pay and keeps the margin, so trees can grow 3 levels deep.
//! Every bid stakes a small NEAR bond (a junk winner loses it to its
//! buyer); the quality lead runs sealed bids (commit, then reveal). The
//! factory keeps a 0.5% platform fee on each posted budget.
//! The contract judges every delivery and records accumulate on-chain.
//! Nobody scripts who wins. The output (`web/swarm.json`) shows what
//! emerges: prices per lead, tree depth, margins, earnings, junk priced out.

use crate::agents::{b64, WORKER};
use crate::chain::{num, Acc, Chain, USD};
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
    /// Bids 55% and stakes a newcomer's bond like an honest agent, but
    /// delivers fluent, empty text that sometimes fools the judge.
    Polished,
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
            Strategy::Polished => "polished",
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
    /// Contractor: its sub-hires so far, (rate of the sub-part's max paid, passed).
    pub sub_hist: Vec<(u128, bool)>,
    /// Contractor: sub-parts it chose to buy, and to do itself.
    pub bought: u32,
    pub made: u32,
    pub since_buy: u32,
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

/// A contractor's own cost to do a sub-part outside its skill (percent of
/// that sub-part's max): higher than a specialist's cost floor.
pub const OWN_COST: u128 = 80;

pub const MAKE_OR_BUY: &str = "ROLE: contractor. You won a part and may buy one sub-part of it from other agents, paid out of your own pay, or do that sub-part yourself at your own cost. \
Reply with JSON only: {\"buy\": true|false, \"share\": <the most you offer for the sub-part, percent of your own price, 10-60>, \"why\": \"<one short sentence>\"}. \
Buying is worth it when the expected price, counting sub-agents that deliver junk (you then pay nothing but must still do the work), is below your own cost.";

/// What a contractor knows when it decides to make or buy a sub-part.
pub fn make_or_buy_prompt(
    my_price: u128,
    sub: &str,
    sub_nominal: u128,
    hist: &[(u128, bool)],
    recent_sub_rates: &[u128],
    since_buy: u32,
) -> String {
    let last: Vec<Value> = hist
        .iter()
        .rev()
        .take(4)
        .map(|(r, ok)| json!({ "rate": r, "delivered": ok }))
        .collect();
    json!({ "my_price_fusd": my_price as f64 / USD as f64, "sub_part": sub,
        "sub_part_max_fusd": sub_nominal, "my_own_cost_rate": OWN_COST,
        "rounds_since_i_last_bought": since_buy,
        "my_last_sub_hires": last, "recent_winning_rates_for_it": recent_sub_rates })
    .to_string()
}

/// Reads the model's make-or-buy answer: (buy, share percent 10..=60, why).
/// Unreadable answers buy at the default share.
pub fn read_make_or_buy(answer: &str) -> (bool, u128, String) {
    let v: Value = answer
        .find('{')
        .and_then(|i| answer.rfind('}').map(|j| &answer[i..=j]))
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or(Value::Null);
    (
        v["buy"].as_bool().unwrap_or(true),
        v["share"]
            .as_f64()
            .map(|r| r as u128)
            .unwrap_or(SUB_SHARE)
            .clamp(10, SUB_SHARE),
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
/// (percent) the buyer learned, divided by how many times the slot's bond
/// the bidder staked (`vouch`, read from the bid on-chain): a newcomer that
/// puts 10x the bond at risk pays a tenth of the premium. Ties go to the
/// bidder with more paid jobs. Deterministic.
pub fn pick(
    cands: &[(usize, u128, u32, u32, u128)],
    newcomer_pct: u128,
    record_weight: u128,
) -> Option<usize> {
    cands
        .iter()
        .min_by_key(|(i, bid, jobs, paid, vouch)| {
            let unpaid = jobs.saturating_sub(*paid) as u128;
            let mut score = bid * (100 + record_weight * unpaid / (*jobs as u128).max(1)) / 100;
            if *jobs == 0 {
                score = score * (100 + newcomer_pct / (*vouch).max(1)) / 100;
            }
            (score, std::cmp::Reverse(*paid), *i)
        })
        .map(|c| c.0)
}

/// What an honest newcomer stakes on top of the slot's bond: 9x more, so
/// its bid carries 10x the slot's bond. It gets it back when it delivers;
/// a junk bot that copied it would lose 0.5 NEAR per junk delivery.
/// A buyer audits a passed delivery when the worker has fewer paid jobs.
pub const AUDIT_BELOW: u64 = 3;

pub const VOUCH_EXTRA_YOCTO: u128 = 9 * BOND_YOCTO;

/// What an agent bids on a part with this `max` (yocto fUSD), or None if it
/// sits out. Its ask is `rate`% of the skill's nominal max. When the ask is
/// above the part's max but the max still covers the cost floor, an agent
/// bids the max (a contractor's sub-part pays less than the nominal max);
/// greedy agents never cut, and junk asks low anyway.
pub fn bid_price(st: Strategy, nominal: u128, rate: u128, max: u128) -> Option<u128> {
    let price = (nominal * USD * rate / 100).max(USD / 10);
    if price <= max {
        return Some(price);
    }
    let floor = nominal * USD * COST_FLOOR / 100;
    match st {
        Strategy::Greedy | Strategy::Junk | Strategy::Polished => None,
        _ if max >= floor => Some(max),
        _ => None,
    }
}

pub fn roster() -> Vec<(String, &'static str, Strategy)> {
    use Strategy::*;
    let per: [(&str, &[Strategy]); 6] = [
        (
            "research",
            &[
                Steady, Lean, Greedy, Adaptive, Junk, Sloppy, Undercut, Model, Contractor, Polished,
            ],
        ),
        (
            "translate",
            &[
                Steady, Lean, Greedy, Adaptive, Junk, Undercut, Model, Contractor, Polished,
            ],
        ),
        (
            "summary",
            &[Steady, Lean, Adaptive, Junk, Undercut, Model, Polished],
        ),
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
        Strategy::Polished => 55,
    }
}

/// A small, fixed per-agent spread (-3..=+3 points) so agents with the same
/// strategy do not bid in lockstep.
fn jitter(name: &str) -> i64 {
    (name.bytes().map(|b| b as i64).sum::<i64>() % 7) - 3
}

const BID_SECS: u64 = 300;
const WINDOW_SECS: u64 = 5400;
/// fUSD each lead's client posts per round.
const JOB_BUDGET: u128 = 70;
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
    /// Contractor's make-or-buy answer for this part, if it was asked.
    decision: Option<Value>,
    settled: bool,
    /// Delivered and checked (each part is worked once).
    worked: bool,
    /// The buyer fired this winner after a failed check.
    fired: bool,
    /// This part is the runner-up hired after `replaces` was fired.
    replaces: Option<usize>,
    /// The winner staked more than the slot's bond (a vouching newcomer).
    vouched: bool,
    /// The bytes the winner delivered.
    delivered: Vec<u8>,
    /// The buyer's audit of a passed delivery: two more judge panels.
    audit: Option<Value>,
}

fn buyer_name(b: Buyer, agents: &[Agent]) -> String {
    match b {
        Buyer::Lead(l) => format!("lead-{}", LEADS[l].0),
        Buyer::Agent(i) => agents[i].name.clone(),
    }
}

/// Runs `rounds` rounds: each round every lead posts one job.
pub async fn run(c: &Chain, llm: &Llm, rounds: u32) -> Result<Value, String> {
    let model = llm.model();
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
            sub_hist: vec![],
            bought: 0,
            made: 0,
            since_buy: 0,
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
            tools.call(&leads[l].1, "post_job", &json!({ "budget": (JOB_BUDGET * USD).to_string(), "manager": leads[l].0.id,
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
                    decision: None,
                    worked: false,
                    fired: false,
                    replaces: None,
                    vouched: false,
                    delivered: vec![],
                    audit: None,
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
                        .chat(&chat_body(&model, MARKET_BIDDER, &prompt, 80))
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
                    let Some(price) = bid_price(a.strategy, nominal_max(skill), a.rate, max) else {
                        continue;
                    };
                    // An honest agent never hired yet vouches for itself with
                    // a bigger bond; junk keeps the minimum (it expects to
                    // lose it).
                    let stake = if a.wins == 0 && a.strategy != Strategy::Junk {
                        VOUCH_EXTRA_YOCTO
                    } else {
                        0
                    };
                    let placed = if parts[p].sealed {
                        tools
                            .call(
                                &a.acc,
                                "bid_sealed",
                                &json!({ "job": job, "slot": slot, "price": price.to_string(),
                                "salt": format!("r{r}p{p}a{i}"), "stake": stake.to_string() }),
                            )
                            .await
                    } else {
                        tools
                            .call(
                                &a.acc,
                                "bid",
                                &json!({ "job": job, "slot": slot, "price": price.to_string(), "stake": stake.to_string() }),
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
                // Each bid's bond, as the job account holds it.
                let slot_info = tools
                    .call(
                        &keeper,
                        "get_bids",
                        &json!({ "job": parts[p].job, "slot": parts[p].slot }),
                    )
                    .await?;
                let slot_bond = num(&slot_info["bond"]).max(1);
                let staked: HashMap<String, u128> = slot_info["bids"]
                    .as_array()
                    .map(|bs| {
                        bs.iter()
                            .map(|b| {
                                (
                                    b["worker"].as_str().unwrap_or("").to_string(),
                                    num(&b["bond"]),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut cands = vec![];
                for &(i, price) in &parts[p].bids {
                    if booked.contains(&i) {
                        continue;
                    }
                    let vouch =
                        staked.get(agents[i].acc.id.as_str()).copied().unwrap_or(0) / slot_bond;
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
                        vouch,
                    ));
                }
                let prem = *premium.get(&bname).unwrap_or(&0);
                let Some(w) = pick(&cands, prem, rules.record_weight) else {
                    if let Buyer::Lead(l) = buyer {
                        lead_unfilled[l] += 1;
                    }
                    continue;
                };
                let (_, price, jobs, _, vouch) = *cands.iter().find(|x| x.0 == w).unwrap();
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
                    q.vouched = vouch > 1;
                }
                // A contractor becomes a buyer: it puts the sub-part out for
                // bids on its own job, paid out of its own pay.
                if agents[w].strategy == Strategy::Contractor && level < 3 {
                    if let Some((sub, sub_max)) = sub_skill(agents[w].skill) {
                        // The model decides: buy the sub-part, or do it itself.
                        let recent: Vec<u128> = history
                            .get(sub)
                            .map(|h| h.iter().rev().take(6).copied().collect())
                            .unwrap_or_default();
                        let ans = llm
                            .chat(&chat_body(
                                &model,
                                MAKE_OR_BUY,
                                &make_or_buy_prompt(
                                    price,
                                    sub,
                                    sub_max,
                                    &agents[w].sub_hist,
                                    &recent,
                                    agents[w].since_buy,
                                ),
                                80,
                            ))
                            .await?;
                        let (buy, share, why) = read_make_or_buy(&ans.content());
                        parts[p].decision = Some(json!({ "buy": buy, "share": share, "why": why }));
                        if !buy {
                            agents[w].made += 1;
                            agents[w].since_buy += 1;
                            continue;
                        }
                        agents[w].bought += 1;
                        agents[w].since_buy = 0;
                        let max = (sub_max * USD).min(price * share / 100);
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
                            decision: None,
                            worked: false,
                            fired: false,
                            replaces: None,
                            vouched: false,
                            delivered: vec![],
                            audit: None,
                        });
                    }
                }
            }
            level_start = level_parts.last().unwrap() + 1;
        }
        // 4. Work, bottom-up: the deepest parts first, so a contractor's
        // sub-part is judged and paid before it delivers.
        // A part whose winner fails the check is fired at once and the
        // runner-up is hired and worked in the same pass (once per part).
        for level in (1..=3u32).rev() {
            loop {
                let ps: Vec<usize> = (0..parts.len())
                    .filter(|&p| {
                        parts[p].level == level && parts[p].winner.is_some() && !parts[p].worked
                    })
                    .collect();
                if ps.is_empty() {
                    break;
                }
                for &p in &ps {
                    parts[p].worked = true;
                    let w = parts[p].winner.unwrap();
                    let junk = match agents[w].strategy {
                        Strategy::Junk => true,
                        Strategy::Sloppy => agents[w].wins % 3 == 0,
                        _ => false,
                    };
                    let (child, task) = (parts[p].child.clone(), parts[p].task.clone());
                    let delivered = if agents[w].strategy == Strategy::Polished {
                        let b = crate::agents::POLISHED.as_bytes().to_vec();
                        tools
                            .call(
                                &agents[w].acc,
                                "deliver",
                                &json!({ "job": child, "resp": b64(&b) }),
                            )
                            .await?;
                        b
                    } else if junk {
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
                            chat_body(&model, WORKER, &format!("HT1:{child}:{spec}:1 {task}"), 300);
                        let rep = llm.chat(&body).await?;
                        tools
                            .call(
                                &agents[w].acc,
                                "deliver",
                                &json!({ "job": child, "resp": b64(&rep.body), "req": b64(&body),
                            "sig": rep.sig, "model": model }),
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
                    parts[p].delivered = delivered;
                }
                c.fast_forward(crate::agents::CHECK_WINDOW_SECS + 1).await;
                for &p in &ps {
                    let child = parts[p].child.clone();
                    tools
                        .call(&keeper, "finalize_check", &json!({ "job": child }))
                        .await?;
                }
                // Audit: before paying a pass from a thin record (fewer than
                // AUDIT_BELOW paid jobs), the buyer disputes it on-chain; two
                // more judge panels answer, and two failing votes of three
                // refund the buyer and keep the bond.
                let mut audited = vec![];
                for &p in &ps {
                    if !parts[p].passed {
                        continue;
                    }
                    let w = parts[p].winner.unwrap();
                    let rec = tools
                        .call(
                            &keeper,
                            "get_record",
                            &json!({ "account": agents[w].acc.id }),
                        )
                        .await?;
                    if rec["paid"].as_u64().unwrap_or(0) >= AUDIT_BELOW {
                        continue;
                    }
                    let buyer_acc = match parts[p].buyer {
                        Buyer::Lead(l) => leads[l].0.clone(),
                        Buyer::Agent(b) => agents[b].acc.clone(),
                    };
                    let (child, task) = (parts[p].child.clone(), parts[p].task.clone());
                    let d = b64(&parts[p].delivered);
                    tools
                        .call(&buyer_acc, "dispute", &json!({ "job": child, "resp": d }))
                        .await?;
                    let mut votes = vec![parts[p].score];
                    for panel in 1..=2u8 {
                        let req = tools
                            .call(&keeper, "check_request", &json!({ "job": child, "panel": panel, "task": task, "deliverable": d }))
                            .await?;
                        let qb = base64::engine::general_purpose::STANDARD
                            .decode(req.as_str().ok_or("check_request")?)
                            .map_err(|e| e.to_string())?;
                        let ans = llm.chat(&qb).await?;
                        let sc = tools
                            .call(
                                &keeper,
                                "submit_check",
                                &json!({ "job": child, "panel": panel, "task": task,
                                "deliverable": d, "answer": b64(&ans.body), "sig": ans.sig }),
                            )
                            .await?;
                        votes.push(
                            sc.as_u64()
                                .or_else(|| sc.as_str().and_then(|x| x.parse().ok()))
                                .unwrap_or(0),
                        );
                    }
                    parts[p].audit = Some(json!({ "votes": votes }));
                    audited.push(p);
                }
                if !audited.is_empty() {
                    c.fast_forward(crate::agents::CHECK_WINDOW_SECS + 1).await;
                }
                for &p in &audited {
                    let upheld = tools
                        .call(
                            &keeper,
                            "finalize_dispute",
                            &json!({ "job": parts[p].child }),
                        )
                        .await?
                        .as_bool()
                        .unwrap_or(false);
                    parts[p].audit.as_mut().unwrap()["caught"] = json!(upheld);
                    if upheld {
                        parts[p].passed = false;
                    }
                }
                for &p in &ps {
                    let child = parts[p].child.clone();
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
                            // A sub-part that hired below itself settles only
                            // once its own sub-parts have (a failed one waits
                            // for its window); until then it is approved, unpaid.
                            if parts
                                .iter()
                                .any(|k| k.parent == Some(p) && k.winner.is_some() && !k.settled)
                            {
                                continue;
                            }
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
                // Fire failed winners and hire the runner-up at its own bid.
                for &p in &ps {
                    if parts[p].passed || parts[p].replaces.is_some() {
                        continue;
                    }
                    if parts
                        .iter()
                        .any(|k| k.parent == Some(p) && k.winner.is_some() && !k.settled)
                    {
                        continue;
                    }
                    let buyer = parts[p].buyer;
                    let buyer_acc = match buyer {
                        Buyer::Lead(l) => leads[l].0.clone(),
                        Buyer::Agent(b) => agents[b].acc.clone(),
                    };
                    let child = parts[p].child.clone();
                    // A refunded audit closes with approve; a failed check with fire.
                    let caught = parts[p]
                        .audit
                        .as_ref()
                        .is_some_and(|a| a["caught"] == json!(true));
                    tools
                        .call(
                            &buyer_acc,
                            if caught { "approve" } else { "fire" },
                            &json!({ "job": child }),
                        )
                        .await?;
                    tools
                        .call(&keeper, "settle", &json!({ "job": child }))
                        .await?;
                    parts[p].fired = true;
                    parts[p].settled = true;
                    let failed = parts[p].winner;
                    let rules = match buyer {
                        Buyer::Lead(l) => LEADS[l].1,
                        Buyer::Agent(_) => CONTRACTOR_RULES,
                    };
                    let mut cands = vec![];
                    for &(i, price) in &parts[p].bids {
                        if Some(i) == failed || booked.contains(&i) || agents[i].gone {
                            continue;
                        }
                        let rec = tools
                            .call(
                                &keeper,
                                "get_record",
                                &json!({ "account": agents[i].acc.id }),
                            )
                            .await?;
                        // Losers' bonds went back at the award: no vouching now.
                        cands.push((
                            i,
                            price,
                            rec["jobs"].as_u64().unwrap_or(0) as u32,
                            rec["paid"].as_u64().unwrap_or(0) as u32,
                            1,
                        ));
                    }
                    let bname = buyer_name(buyer, &agents);
                    let prem = *premium.get(&bname).unwrap_or(&0);
                    let Some(r2) = pick(&cands, prem, rules.record_weight) else {
                        continue;
                    };
                    let (_, price, jobs, _, _) = *cands.iter().find(|x| x.0 == r2).unwrap();
                    let job = parts[p].job.clone();
                    let task = parts[p].task.clone();
                    if tools
                        .call(
                            &buyer_acc,
                            "hire",
                            &json!({ "job": job, "worker": agents[r2].acc.id,
                        "amount": price.to_string(), "task": task }),
                        )
                        .await
                        .is_err()
                    {
                        continue;
                    }
                    let k = children.entry(job.clone()).or_insert(0);
                    *k += 1;
                    let child = format!("t{}.{}", k, job);
                    booked.insert(r2);
                    agents[r2].wins += 1;
                    if let Buyer::Lead(l) = buyer {
                        lead_spent[l] += price;
                    }
                    let q = &parts[p];
                    let next = Part {
                        lead: q.lead,
                        root: q.root.clone(),
                        job,
                        level: q.level,
                        parent: q.parent,
                        buyer,
                        skill: q.skill,
                        max: q.max,
                        task,
                        slot: q.slot,
                        sealed: q.sealed,
                        bids: vec![],
                        winner: Some(r2),
                        price,
                        newcomer: jobs == 0,
                        premium_seen: prem,
                        child,
                        score: 0,
                        passed: false,
                        paid_now: false,
                        settled: false,
                        decision: None,
                        worked: false,
                        fired: false,
                        replaces: Some(p),
                        vouched: false,
                        delivered: vec![],
                        audit: None,
                    };
                    parts.push(next);
                }
            }
        }
        // 5. Failed or blocked parts wait for the window; then every job
        // settles bottom-up and the roots close.
        if parts
            .iter()
            .any(|q| q.winner.is_some() && !q.paid_now && !q.settled)
        {
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
            if let Buyer::Agent(b) = q.buyer {
                let rate = q.price * 100 / (nominal_max(q.skill) * USD);
                agents[b].sub_hist.push((rate, q.passed));
            }
        }
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
        // A runner-up hired after a firing staked no bond.
        for q in parts.iter().filter(|q| !q.passed && q.replaces.is_none()) {
            if let Some(w) = q.winner {
                agents[w].bonds_lost += 1;
            }
        }
        let caught: Vec<usize> = parts
            .iter()
            .filter(|q| !q.passed)
            .filter_map(|q| q.winner)
            .filter(|&w| matches!(agents[w].strategy, Strategy::Junk | Strategy::Polished))
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
                    "paid": q.passed, "paid_now": q.paid_now, "make_or_buy": q.decision, "fired": q.fired, "replaces": q.replaces, "vouched": q.vouched, "audit": q.audit, "job": q.child, "sealed": q.sealed,
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
            "earned": earned as f64 / 1e6, "record": rec, "final_rate": a.rate, "quit": a.gone, "bonds_lost": a.bonds_lost, "bought": a.bought, "made": a.made }));
    }
    let settings = c.view(&c.hub, "get_settings", json!({})).await?;
    Ok(
        json!({ "schema": "hiretree-swarm-2", "network": c.network_name, "rounds": log, "agents": table,
        "leads": LEADS.iter().enumerate().map(|(l, (n, r))| json!({ "lead": n, "record_weight": r.record_weight,
            "max_pct": r.max_pct, "spent": lead_spent[l] as f64 / 1e6, "unfilled": lead_unfilled[l] })).collect::<Vec<_>>(),
        "junk_respawns": respawns, "premiums": premium,
        "platform_fee_bps": SWARM_FEE_BPS, "job_budget": JOB_BUDGET, "bond_near": BOND_YOCTO as f64 / 1e24, "sealed_lead": "quality", "platform_fees": num(&settings["fees"]) as f64 / 1e6,
        "model": match llm {
            Llm::Mock(_) => "scripted test model (not NEAR AI) for work, judging, the model bidder and contractors' make-or-buy; agents adapt their asks; buyers pick by price, on-chain record and a learned newcomer premium".to_string(),
            _ => format!("NEAR AI Cloud ({model}) for work, judging, the model bidder and contractors' make-or-buy, every answer signed; agents adapt their asks; buyers pick by price, on-chain record and a learned newcomer premium"),
        } }),
    )
}

#[cfg(test)]
mod tests {
    use crate::judge::MODEL;
    fn decide(hist: &[(u128, bool)], recent: &[u128], since: u32) -> (bool, u128, String) {
        let user = make_or_buy_prompt(15 * USD, "fetch", 5, hist, recent, since);
        let req = json!({ "messages": [ { "role": "system", "content": MAKE_OR_BUY }, { "role": "user", "content": user } ] });
        read_make_or_buy(&crate::agents::mock_script(&req))
    }

    #[test]
    fn a_contractor_buys_until_its_sub_agents_deliver_junk() {
        // No history: 60% / 0.8 = 75% < own cost 80%: buy.
        assert!(decide(&[], &[], 0).0);
        // One junk hire: 60% / (4/6) = 90% > 80%: do it itself.
        let (buy, _, why) = decide(&[(48, false)], &[48, 60, 62], 0);
        assert!(!buy, "{why}");
        // After 3 rounds of doing it itself, it tests the market again.
        assert!(decide(&[(48, false)], &[48, 60, 62], 3).0);
        // Good hires keep it buying.
        assert!(
            decide(
                &[(60, true), (58, true), (62, true), (48, false)],
                &[55, 60],
                0
            )
            .0
        );
    }

    #[test]
    fn make_or_buy_answers_are_clamped() {
        assert_eq!(
            read_make_or_buy(r#"{"buy": true, "share": 90}"#).1,
            SUB_SHARE
        );
        assert_eq!(
            read_make_or_buy(r#"{"buy": false, "share": 2}"#),
            (false, 10, String::new())
        );
        assert_eq!(
            read_make_or_buy("not json"),
            (true, SUB_SHARE, String::new())
        );
    }

    #[test]
    fn an_agent_bids_the_max_when_it_still_covers_its_cost() {
        use Strategy::*;
        // fetch: nominal 5; a contractor's sub-part offers at most 3.24.
        let max = 324 * USD / 100;
        assert_eq!(bid_price(Lean, 5, 66, max), Some(max));
        assert_eq!(bid_price(Lean, 5, 60, max), Some(3 * USD));
        assert_eq!(bid_price(Greedy, 5, 100, max), None);
        // Below the cost floor (55% of 5 = 2.75) nobody but junk bids.
        assert_eq!(bid_price(Steady, 5, 90, 2 * USD), None);
        assert_eq!(bid_price(Junk, 5, 45, 3 * USD), Some(225 * USD / 100));
    }

    use super::*;

    #[test]
    fn pick_takes_the_lowest_bid_with_a_clean_record() {
        assert_eq!(
            pick(
                &[(0, 27, 0, 0, 1), (1, 19, 0, 0, 1), (2, 30, 0, 0, 1)],
                0,
                150
            ),
            Some(1)
        );
    }

    #[test]
    fn unpaid_jobs_cost_more_than_a_low_bid_saves() {
        // 13.5 with 2 of 2 unpaid weighs 54; 27 with a clean record wins.
        assert_eq!(pick(&[(0, 27, 3, 3, 1), (1, 13, 2, 0, 1)], 0, 150), Some(0));
        // One unpaid job in four is a smaller penalty: 19 weighs ~26 and still wins.
        assert_eq!(pick(&[(0, 27, 3, 3, 1), (1, 19, 4, 3, 1)], 0, 150), Some(1));
    }

    #[test]
    fn burned_leads_charge_newcomers_a_premium() {
        // A fresh account bidding 13 beats 18 with no premium...
        assert_eq!(pick(&[(0, 18, 5, 5, 1), (1, 13, 0, 0, 1)], 0, 150), Some(1));
        // ...but not once half the newcomers it hired failed (+75%).
        assert_eq!(
            pick(&[(0, 18, 5, 5, 1), (1, 13, 0, 0, 1)], 75, 150),
            Some(0)
        );
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
    fn a_newcomer_that_stakes_more_bond_pays_less_premium() {
        // Both new: junk bids 13 at the slot's bond, an honest agent bids 18
        // and stakes 10x. With a 100% premium junk weighs 26, honest 19.8.
        assert_eq!(
            pick(&[(0, 13, 0, 0, 1), (1, 18, 0, 0, 10)], 100, 150),
            Some(1)
        );
        // With no premium learned yet, the cheaper bid still wins.
        assert_eq!(
            pick(&[(0, 13, 0, 0, 1), (1, 18, 0, 0, 10)], 0, 150),
            Some(0)
        );
    }

    #[test]
    fn ties_go_to_the_longer_clean_record() {
        assert_eq!(pick(&[(0, 18, 1, 1, 1), (1, 18, 4, 4, 1)], 0, 150), Some(1));
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
        let c = [(0, 16, 8, 8, 1), (1, 13, 8, 7, 1)];
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
        assert_eq!(r.len(), 36);
        for (skill, _) in SKILLS {
            assert!(r.iter().any(|x| x.1 == skill && x.2 == Strategy::Junk));
            // Junk that can fool the judge competes for every top-level part.
            assert!(r.iter().any(|x| x.1 == skill && x.2 == Strategy::Polished));
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
