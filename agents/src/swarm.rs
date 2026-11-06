//! `hiretree swarm`: many worker agents with different strategies compete
//! for many jobs on a local node. Nobody scripts who wins: each agent bids
//! by its own rule, the lead awards by price weighed against the bidder's
//! on-chain record, the contract judges the work, and records accumulate.
//! The output (`web/swarm.json`) shows what emerges: prices, win shares,
//! earnings, and junk being priced out.

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
        }
    }
}

pub struct Agent {
    pub name: String,
    pub acc: Acc,
    pub skill: &'static str,
    pub strategy: Strategy,
    /// Current asking rate, percent of max.
    pub rate: u128,
    pub wins: u32,
    pub earned: u128,
    /// Booked through this job number: a winner sits out the next round.
    pub busy: u32,
    /// A junk bot that gave up (never paid, and each new account costs).
    pub gone: bool,
    pub respawned: u32,
    /// Rounds bid and lost in a row.
    pub lost: u32,
    pub won_last: bool,
    /// The model bidder's reason for its last ask.
    pub why: String,
}

pub const MARKET_BIDDER: &str = "ROLE: market bidder. You sell one skill in an open market of AI agents. \
Reply with JSON only: {\"rate\": <your ask, percent of the part's max>, \"why\": \"<one short sentence>\"}. \
Asking below cost_floor loses money; you may do it (never below 40) only to win a first job and start a record. The lead picks the lowest ask after a penalty for unpaid jobs on the record.";

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

/// The lead's newcomer premium (percent), learned from its own history: a
/// failed newcomer adds 100 (cap 150), a newcomer that delivered halves it,
/// and a round with no newcomer hired lets it fade by a fifth.
pub fn next_premium(p: u128, newcomer_failed: Option<bool>) -> u128 {
    match newcomer_failed {
        Some(true) => (p + 100).min(150),
        Some(false) => p / 2,
        None => p * 4 / 5,
    }
}

pub const SKILLS: [(&str, u128); 3] = [("research", 30), ("translate", 20), ("summary", 10)];

/// The lead's pick: the lowest bid after a penalty for the share of
/// unpaid jobs on the bidder's on-chain record (all unpaid: x2.5), and for
/// a bidder with no record yet, a newcomer premium (percent) the lead
/// learns from how often newcomers it hired failed. Ties go to the bidder
/// with more paid jobs. Deterministic.
pub fn pick(cands: &[(usize, u128, u32, u32)], newcomer_pct: u128) -> Option<usize> {
    cands
        .iter()
        .min_by_key(|(i, bid, jobs, paid)| {
            let unpaid = jobs.saturating_sub(*paid) as u128;
            let mut score = bid * (100 + 150 * unpaid / (*jobs as u128).max(1)) / 100;
            if *jobs == 0 {
                score = score * (100 + newcomer_pct) / 100;
            }
            (score, std::cmp::Reverse(*paid), *i)
        })
        .map(|c| c.0)
}

pub fn roster() -> Vec<(String, &'static str, Strategy)> {
    let mut v = vec![];
    let strategies = [
        Strategy::Steady,
        Strategy::Lean,
        Strategy::Greedy,
        Strategy::Adaptive,
        Strategy::Junk,
        Strategy::Sloppy,
        Strategy::Undercut,
        Strategy::Model,
    ];
    for (si, (skill, _)) in SKILLS.iter().enumerate() {
        for st in strategies.iter() {
            // 8 per skill would be 24; drop greedy for summary: 23 agents.
            if si == 2 && *st == Strategy::Greedy {
                continue;
            }
            v.push((format!("{}-{}", &skill[..3], st.name()), *skill, *st));
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
    }
}

/// A small, fixed per-agent spread (-3..=+3 points) so agents with the same
/// strategy do not bid in lockstep.
fn jitter(name: &str) -> i64 {
    (name.bytes().map(|b| b as i64).sum::<i64>() % 7) - 3
}

const BID_SECS: u64 = 30;
const WINDOW_SECS: u64 = 1200;

pub async fn run(c: &Chain, llm: &Llm, jobs: u32) -> Result<Value, String> {
    let tools = Tools { chain: c };
    let mk = |n: String| async move { c.account(&n, 5).await };
    let client = mk("sclient".into()).await;
    let lead = mk("slead".into()).await;
    let treasury = mk("streasury".into()).await;
    let keeper = mk("skeeper".into()).await;
    for a in [&client, &lead, &treasury] {
        c.register(a).await;
    }
    c.mint(&client.id, 70 * USD * jobs as u128).await;
    c.mint(&treasury.id, 200 * USD).await;
    c.call(
        &client,
        &c.hub,
        "prepay_post",
        json!({}),
        near_api::types::NearToken::from_millinear(60 * jobs as u128),
    )
    .await?;
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
            earned: 0,
            busy: 0,
            gone: false,
            respawned: 0,
            lost: 0,
            won_last: false,
            why: String::new(),
        });
    }
    let mut log = vec![];
    // The lead's own memory of how newcomers did.
    let mut premium = 0u128;
    let mut history: std::collections::HashMap<&str, Vec<u128>> = Default::default();
    let mut respawns = 0u32;
    for n in 1..=jobs {
        let next = c.view(&c.hub, "get_settings", json!({})).await?["next"]
            .as_u64()
            .ok_or("next")?;
        let root = format!("job-{next}.{}", c.hub);
        let task = format!("Swarm job {n}: explainer, Spanish translation and a summary.");
        tools.call(&client, "post_job", &json!({ "budget": (70 * USD).to_string(), "manager": lead.id, "task": task,
            "window_secs": WINDOW_SECS, "lead_fee": (5 * USD).to_string(), "treasury": treasury.id })).await?;
        let mut parts = vec![];
        for (skill, max) in SKILLS {
            let ptask = format!("Job {n}: the {skill} part.");
            let slot = tools
                .call(
                    &lead,
                    "open_bids",
                    &json!({ "job": root, "task": ptask,
                "max": (max * USD).to_string(), "secs": BID_SECS }),
                )
                .await?
                .as_u64()
                .ok_or("slot")?;
            let mut bids = vec![];
            for (i, a) in agents
                .iter_mut()
                .enumerate()
                .filter(|(_, a)| a.skill == skill && a.busy < n && !a.gone)
            {
                let recent: Vec<u128> = history
                    .get(skill)
                    .map(|h| h[h.len().saturating_sub(6)..].to_vec())
                    .unwrap_or_default();
                if a.strategy == Strategy::Undercut {
                    if let Some(r) = recent.last() {
                        a.rate = r.saturating_sub(3).max(COST_FLOOR);
                    }
                }
                if a.strategy == Strategy::Model {
                    let r = tools
                        .call(&a.acc, "get_record", &json!({ "account": a.acc.id }))
                        .await?;
                    let prompt = market_prompt(
                        skill,
                        &recent,
                        r["jobs"].as_u64().unwrap_or(0),
                        r["paid"].as_u64().unwrap_or(0),
                        a.rate,
                        a.lost,
                        a.won_last,
                    );
                    let ans = llm
                        .chat(&chat_body(MODEL, MARKET_BIDDER, &prompt, 80))
                        .await?;
                    (a.rate, a.why) = read_rate(&ans.content(), a.rate);
                }
                let price = (max * USD * a.rate / 100).max(USD / 10);
                if tools
                    .call(
                        &a.acc,
                        "bid",
                        &json!({ "job": root, "slot": slot, "price": price.to_string() }),
                    )
                    .await
                    .is_ok()
                {
                    bids.push((i, price));
                }
            }
            parts.push((skill, max, ptask, slot, bids));
        }
        c.fast_forward(BID_SECS + 1).await;
        let mut hired = vec![];
        let mut rows = vec![];
        let mut newcomer_this_job = false;
        for (skill, max, ptask, slot, bids) in &parts {
            let mut cands = vec![];
            for (i, price) in bids {
                let r = tools
                    .call(
                        &lead,
                        "get_record",
                        &json!({ "account": agents[*i].acc.id }),
                    )
                    .await?;
                cands.push((
                    *i,
                    *price,
                    r["jobs"].as_u64().unwrap_or(0) as u32,
                    r["paid"].as_u64().unwrap_or(0) as u32,
                ));
            }
            let Some(w) = pick(&cands, premium) else {
                continue;
            };
            let newcomer = cands
                .iter()
                .find(|x| x.0 == w)
                .map(|x| x.2 == 0)
                .unwrap_or(false);
            let price = cands.iter().find(|x| x.0 == w).unwrap().1;
            tools
                .call(
                    &lead,
                    "award",
                    &json!({ "job": root, "slot": slot, "worker": agents[w].acc.id }),
                )
                .await?;
            let sub = format!("t{}.{root}", hired.len() + 1);
            // Bidders adapt: the winner asks more next time, losers less,
            // and a greedy agent that keeps losing leaves the market.
            for (i, _) in bids {
                let a = &mut agents[*i];
                a.won_last = *i == w;
                if *i == w {
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
            agents[w].wins += 1;
            agents[w].busy = n + 1;
            history
                .entry(skill)
                .or_default()
                .push(price * 100 / (max * USD));
            let junk = match agents[w].strategy {
                Strategy::Junk => true,
                Strategy::Sloppy => agents[w].wins % 3 == 0,
                _ => false,
            };
            let delivered = if junk {
                let b = b"lorem ipsum dolor sit amet, lorem ipsum dolor sit amet".to_vec();
                tools
                    .call(
                        &agents[w].acc,
                        "deliver",
                        &json!({ "job": sub, "resp": b64(&b) }),
                    )
                    .await?;
                b
            } else {
                let spec = ht_core::receipt::sha256_hex(ptask.as_bytes());
                let body = chat_body(MODEL, WORKER, &format!("HT1:{sub}:{spec}:1 {ptask}"), 300);
                let r = llm.chat(&body).await?;
                tools
                    .call(
                        &agents[w].acc,
                        "deliver",
                        &json!({ "job": sub, "resp": b64(&r.body), "req": b64(&body),
                    "sig": r.sig, "model": MODEL }),
                    )
                    .await?;
                r.body
            };
            let req = tools.call(&keeper, "check_request", &json!({ "job": sub, "panel": 0, "task": ptask, "deliverable": b64(&delivered) })).await?;
            let qb = base64::engine::general_purpose::STANDARD
                .decode(req.as_str().ok_or("check_request")?)
                .map_err(|e| e.to_string())?;
            let ans = llm.chat(&qb).await?;
            let score = tools
                .call(
                    &keeper,
                    "submit_check",
                    &json!({ "job": sub, "panel": 0, "task": ptask,
                "deliverable": b64(&delivered), "answer": b64(&ans.body), "sig": ans.sig }),
                )
                .await?;
            let failed = num(&score) < 6;
            let premium_seen = premium;
            if newcomer {
                premium = next_premium(premium, Some(failed));
                newcomer_this_job = true;
            }
            rows.push(json!({ "newcomer": newcomer, "newcomer_premium": premium_seen, "skill": skill, "max": max, "winner": agents[w].name, "strategy": agents[w].strategy.name(),
                "price": price as f64 / 1e6, "score": score,
                "bids": bids.iter().map(|(i, p)| { let a = &agents[*i];
                    let mut b = json!({ "agent": a.name, "price": *p as f64 / 1e6 });
                    if a.strategy == Strategy::Model { b["why"] = json!(a.why); }
                    b }).collect::<Vec<_>>() }));
            hired.push((sub, w, price));
            // A caught junk bot comes back under a fresh account (a Sybil):
            // its record resets, which is what the newcomer premium is for.
            if failed && agents[w].strategy == Strategy::Junk {
                if agents[w].respawned >= JUNK_RESPAWNS {
                    agents[w].gone = true;
                    continue;
                }
                agents[w].respawned += 1;
                respawns += 1;
                let fresh = mk(format!("w-{}-{respawns}", agents[w].name)).await;
                c.register(&fresh).await;
                agents[w].acc = fresh;
            }
        }
        if !newcomer_this_job {
            premium = next_premium(premium, None);
        }
        c.fast_forward(crate::agents::CHECK_WINDOW_SECS + 1).await;
        let mut wait = false;
        for (k, (sub, w, _)) in hired.iter().enumerate() {
            tools
                .call(&keeper, "finalize_check", &json!({ "job": sub }))
                .await?;
            let q = tools
                .call(&lead, "quote_note", &json!({ "job": sub }))
                .await?;
            let paid_now = !q.is_null();
            if paid_now {
                let price = q
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| q.to_string());
                tools
                    .call(
                        &treasury,
                        "buy_note",
                        &json!({ "job": sub, "price": price }),
                    )
                    .await?;
                agents[*w].earned += num(&q);
                tools.call(&lead, "approve", &json!({ "job": sub })).await?;
            } else {
                wait = true;
            }
            rows[k]["paid"] = json!(paid_now);
        }
        if wait {
            c.fast_forward(WINDOW_SECS + 1).await;
        }
        for (sub, _, _) in &hired {
            tools
                .call(&keeper, "settle", &json!({ "job": sub }))
                .await?;
        }
        let _ = tools
            .call(&client, "approve", &json!({ "job": root }))
            .await;
        tools
            .call(&keeper, "settle", &json!({ "job": root }))
            .await?;
        eprintln!(
            "swarm job {n}/{jobs}: {}",
            rows.iter()
                .map(|r| format!("{} -> {} at {}", r["skill"], r["winner"], r["price"]))
                .collect::<Vec<_>>()
                .join(", ")
        );
        log.push(json!({ "n": n, "job": root, "parts": rows }));
    }
    let mut table = vec![];
    for a in &agents {
        let r = tools
            .call(&lead, "get_record", &json!({ "account": a.acc.id }))
            .await?;
        table.push(json!({ "agent": a.name, "skill": a.skill, "strategy": a.strategy.name(), "wins": a.wins,
            "earned": a.earned as f64 / 1e6, "fUSD": c.ft(&a.acc.id).await as f64 / 1e6, "record": r, "final_rate": a.rate, "quit": a.gone }));
    }
    Ok(
        json!({ "schema": "hiretree-swarm-1", "network": c.network_name, "jobs": log, "agents": table,
        "junk_respawns": respawns, "final_newcomer_premium": premium,
        "model": "scripted test model (not NEAR AI) for work, judging and the model bidder; agents adapt their asks to the market; the lead picks by price, on-chain record and a learned newcomer premium" }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_takes_the_lowest_bid_with_a_clean_record() {
        assert_eq!(
            pick(&[(0, 27, 0, 0), (1, 19, 0, 0), (2, 30, 0, 0)], 0),
            Some(1)
        );
    }

    #[test]
    fn unpaid_jobs_cost_more_than_a_low_bid_saves() {
        // 13.5 with 2 of 2 unpaid weighs 54; 27 with a clean record wins.
        assert_eq!(pick(&[(0, 27, 3, 3), (1, 13, 2, 0)], 0), Some(0));
        // One unpaid job in four is a smaller penalty: 19 weighs ~26 and still wins.
        assert_eq!(pick(&[(0, 27, 3, 3), (1, 19, 4, 3)], 0), Some(1));
    }

    #[test]
    fn burned_leads_charge_newcomers_a_premium() {
        // A fresh account bidding 13 beats 18 with no premium...
        assert_eq!(pick(&[(0, 18, 5, 5), (1, 13, 0, 0)], 0), Some(1));
        // ...but not once half the newcomers it hired failed (+75%).
        assert_eq!(pick(&[(0, 18, 5, 5), (1, 13, 0, 0)], 75), Some(0));
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
        assert_eq!(pick(&[(0, 18, 1, 1), (1, 18, 4, 4)], 0), Some(1));
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
    fn roster_has_every_strategy_for_each_skill() {
        let r = roster();
        assert_eq!(r.len(), 23);
        for (skill, _) in SKILLS {
            assert!(r.iter().any(|x| x.1 == skill && x.2 == Strategy::Junk));
            assert!(r.iter().any(|x| x.1 == skill && x.2 == Strategy::Model));
        }
    }
}
