//! The agents. Every decision is a model call (planner, picker, reviewer,
//! worker, judge); every action is a tool call (see `tools`). The lead
//! plans, reads workers' on-chain track records, hires, reviews and pays
//! early or disputes; workers do the work through NEAR AI and deliver with
//! the signed receipt; a keeper asks the judge model the contract's own
//! check question and submits the signed answer.

use crate::chain::{num, Acc, Chain, USD};
use crate::llm::{chat_body, Llm, Reply};
use crate::scene::Story;
use crate::tools::Tools;
use base64::Engine;
use near_api::types::AccountId;
use serde_json::{json, Value};

pub const CHECK_WINDOW_SECS: u64 = 300;
pub const JOB_WINDOW_SECS: u64 = 3600;

pub const PLANNER: &str = "ROLE: planner. You are the lead agent of a job tree on NEAR. Split the client's task into 2 or 3 subtasks for worker agents. The workers' total pay must not exceed BUDGET fUSD. Reply with only JSON: {\"subtasks\":[{\"name\":\"one word\",\"task\":\"full instructions\",\"pay\":integer}]}";
pub const PICKER: &str = "ROLE: picker. You are the lead agent. Pick the best worker for this subtask from the candidates, using their skills and their on-chain track record (jobs, paid = jobs where they were paid, checked = passed quality checks, junk = disputes they lost). A worker who was hired but not paid failed its check. Reply with only JSON: {\"worker\":\"account id\",\"reason\":\"one sentence\"}";
pub const REVIEWER: &str = "ROLE: reviewer. You are the lead agent. A worker delivered and the contract's quality check finished. Choose one action: buy_note (your treasury pays the worker now at 1% off; only if note_quote is not null and the work is good), dispute (the work is junk even though it passed the check), or none. Reply with only JSON: {\"action\":\"buy_note|dispute|none\",\"reason\":\"one sentence\"}";
pub const WORKER: &str = "ROLE: worker. Do the task in the user message. The first token is a job tag; ignore it. Reply with the deliverable only, under 120 words.";

pub fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// Pull the first JSON object out of a model reply (code fences allowed).
pub fn json_of(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

pub struct WorkerAgent {
    pub acc: Acc,
    pub name: String,
    pub skills: Vec<String>,
    /// A junk bot: delivers filler without calling a model.
    pub junk: bool,
}

pub struct Team {
    pub client: Acc,
    pub lead: Acc,
    pub treasury: Acc,
    pub keeper: Acc,
    pub workers: Vec<WorkerAgent>,
}

/// One hired sub-job, as the agents know it (the task text travels off-chain).
pub struct Hired {
    pub job: AccountId,
    pub name: String,
    pub task: String,
    pub worker: usize,
    pub delivered: Option<Vec<u8>>,
}

pub struct Run<'a> {
    pub chain: &'a Chain,
    pub llm: &'a Llm,
    pub team: Team,
    pub story: Story,
    pub calls: u32,
    /// Job window (sandbox: long, skipped by fast-forward; testnet: short).
    pub window_secs: u64,
}

impl<'a> Run<'a> {
    fn tools(&self) -> Tools<'a> {
        Tools { chain: self.chain }
    }

    async fn ask(&mut self, system: &str, user: &str, max_tokens: u32) -> Result<Reply, String> {
        self.calls += 1;
        let body = chat_body(&self.llm.model(), system, user, max_tokens);
        self.llm.chat(&body).await
    }

    async fn ask_json(&mut self, system: &str, user: &str) -> Result<Value, String> {
        let r = self.ask(system, user, 400).await?;
        json_of(&r.content()).ok_or_else(|| format!("model reply is not JSON: {}", r.content()))
    }

    async fn tool(&self, who: &Acc, name: &str, a: Value) -> Result<Value, String> {
        self.tools().call(who, name, &a).await
    }

    /// One whole job, from post to fold. Returns the root account.
    pub async fn job(
        &mut self,
        n: u32,
        task: &str,
        budget: u128,
        fee: u128,
    ) -> Result<AccountId, String> {
        let c = self.chain;
        let (client, lead, treasury) = (
            self.team.client.clone(),
            self.team.lead.clone(),
            self.team.treasury.clone(),
        );
        let next = c.view(&c.hub, "get_settings", json!({})).await?["next"]
            .as_u64()
            .ok_or("factory: no next")?;
        let root: AccountId = format!("job-{next}.{}", c.hub).parse().unwrap();
        self.story.jobs.clear();
        self.story.job(&format!("job-{n}"), &root);

        // 1. Post.
        self.tool(&client, "post_job", json!({ "budget": budget.to_string(), "manager": lead.id, "task": task,
            "window_secs": self.window_secs, "lead_fee": fee.to_string(), "treasury": treasury.id })).await?;
        self.story.step(c, &format!("Job {n}: the client posts"),
            &format!("\"{task}\" One transfer of {:.0} fUSD to the factory creates {root} from the global contract; the lead's fee is {:.0}. The rest comes back to the client at the end.", budget as f64 / 1e6, fee as f64 / 1e6)).await;

        // 2. Plan.
        let pay_budget = (budget - fee) / USD;
        let plan = self
            .ask_json(&PLANNER.replace("BUDGET", &pay_budget.to_string()), task)
            .await?;
        let subs: Vec<(String, String, u128)> = plan["subtasks"]
            .as_array()
            .ok_or("planner: no subtasks")?
            .iter()
            .take(3)
            .map(|s| {
                (
                    s["name"].as_str().unwrap_or("part").to_lowercase(),
                    s["task"].as_str().unwrap_or("").to_string(),
                    s["pay"].as_u64().unwrap_or(0) as u128 * USD,
                )
            })
            .filter(|s| !s.1.is_empty() && s.2 > 0)
            .collect();
        let total: u128 = subs.iter().map(|s| s.2).sum();
        if subs.is_empty() || total > budget - fee {
            return Err(format!("planner: bad plan {plan}"));
        }
        for (name, t, pay) in &subs {
            self.story.decision(
                "lead",
                &format!("plan: {name} for {:.0} fUSD", *pay as f64 / 1e6),
                t,
            );
        }

        // 3. Pick by track record, then hire.
        let mut hired: Vec<Hired> = vec![];
        for (i, (name, t, pay)) in subs.iter().enumerate() {
            let mut cands = vec![];
            for w in &self.team.workers {
                let rec = self
                    .tool(&lead, "get_record", json!({ "account": w.acc.id }))
                    .await?;
                cands.push(json!({ "account": w.acc.id, "skills": w.skills, "record": rec }));
            }
            let pick = self
                .ask_json(
                    PICKER,
                    &json!({ "subtask": { "name": name, "task": t }, "candidates": cands })
                        .to_string(),
                )
                .await?;
            let who = pick["worker"].as_str().unwrap_or_default();
            let wi = self
                .team
                .workers
                .iter()
                .position(|w| w.acc.id.as_str() == who)
                .ok_or(format!("picker chose unknown {who}"))?;
            self.story.decision(
                "lead",
                &format!("pick {} for {name}", self.team.workers[wi].name),
                pick["reason"].as_str().unwrap_or(""),
            );
            self.tool(
                &lead,
                "hire",
                json!({ "job": root, "worker": who, "amount": pay.to_string(), "task": t }),
            )
            .await?;
            let job: AccountId = format!("t{}.{root}", i + 1).parse().unwrap();
            self.story.job(&format!("t{} {name}", i + 1), &job);
            hired.push(Hired {
                job,
                name: name.clone(),
                task: t.clone(),
                worker: wi,
                delivered: None,
            });
        }
        self.story.step(c, "The lead agent plans and hires",
            "The lead's model splits the task, reads each candidate's on-chain track record, and picks. Each hire creates a sub-account holding exactly that pay.").await;

        // 4. Work and deliver.
        for h in hired.iter_mut() {
            let w = &self.team.workers[h.worker];
            let (wacc, junk, wname) = (w.acc.clone(), w.junk, w.name.clone());
            let spec = ht_core::receipt::sha256_hex(h.task.as_bytes());
            if junk {
                let bytes = b"lorem ipsum dolor sit amet, lorem ipsum dolor sit amet".to_vec();
                self.tool(
                    &wacc,
                    "deliver",
                    json!({ "job": h.job, "resp": b64(&bytes) }),
                )
                .await?;
                self.story
                    .decision(&wname, "deliver", "filler text, no model call, no receipt");
                h.delivered = Some(bytes);
            } else {
                let user = format!("HT1:{}:{spec}:1 {}", h.job, h.task);
                let body = chat_body(&self.llm.model(), WORKER, &user, 300);
                self.calls += 1;
                let r = self.llm.chat(&body).await?;
                self.tool(
                    &wacc,
                    "deliver",
                    json!({ "job": h.job, "resp": b64(&r.body), "req": b64(&body),
                    "sig": r.sig, "model": self.llm.model() }),
                )
                .await?;
                let preview: String = r.content().chars().take(140).collect();
                self.story
                    .decision(&wname, "deliver with NEAR AI receipt", &preview);
                h.delivered = Some(r.body);
            }
        }
        self.story.step(c, "Workers deliver",
            "Workers that used NEAR AI deliver the raw response with its signature: the contract checks that the request carries this job's HT1 tag and that NEAR AI signed it. Each delivery opens a 5-minute check round.").await;

        // 5. Keeper: the contract's own check question, signed answer.
        self.check_round(&hired, 0).await?;
        self.chain.fast_forward(CHECK_WINDOW_SECS).await;
        let keeper = self.team.keeper.clone();
        for h in &hired {
            self.tool(&keeper, "finalize_check", json!({ "job": h.job }))
                .await?;
        }
        self.story.step(c, "The contract's check, answered by NEAR AI",
            "For each delivery the contract builds the judge question itself. A keeper asks NEAR AI exactly that and submits the signed answer; the contract verifies the signature and reads the score. Low scores reopen the job.").await;

        // 6. Review: pay now, dispute, or wait.
        let mut disputed = vec![];
        for h in &hired {
            let j = self.tool(&lead, "get_job", json!({ "job": h.job })).await?;
            let quote = self
                .tool(&lead, "quote_note", json!({ "job": h.job }))
                .await?;
            let content = h
                .delivered
                .as_ref()
                .map(|d| {
                    String::from_utf8_lossy(d)
                        .chars()
                        .take(1500)
                        .collect::<String>()
                })
                .unwrap_or_default();
            let r = self
                .ask_json(
                    REVIEWER,
                    &json!({ "task": h.task, "deliverable": content, "status": j["status"],
                "quality": j["quality"], "verified": j["verified"], "note_quote": quote })
                    .to_string(),
                )
                .await?;
            let action = r["action"].as_str().unwrap_or("none");
            let reason = r["reason"].as_str().unwrap_or("");
            match action {
                "buy_note" if !quote.is_null() => {
                    self.tool(
                        &treasury,
                        "buy_note",
                        json!({ "job": h.job, "price": quote }),
                    )
                    .await?;
                    self.story.decision(
                        "lead",
                        &format!("buy {}'s note for {}", h.name, crate::scene::usd(&quote)),
                        reason,
                    );
                }
                "dispute"
                    if matches!(j["status"].as_str(), Some("Checked") | Some("Delivered")) =>
                {
                    self.tool(
                        &lead,
                        "dispute",
                        json!({ "job": h.job, "resp": b64(h.delivered.as_ref().unwrap()) }),
                    )
                    .await?;
                    self.story
                        .decision("lead", &format!("dispute {}", h.name), reason);
                    disputed.push(h.job.clone());
                }
                _ => self.story.decision(
                    "lead",
                    &format!(
                        "no action on {} ({})",
                        h.name,
                        j["status"].as_str().unwrap_or("?")
                    ),
                    reason,
                ),
            }
        }
        if !disputed.is_empty() {
            let ds: Vec<&Hired> = hired.iter().filter(|h| disputed.contains(&h.job)).collect();
            for p in [1u8, 2] {
                for h in &ds {
                    self.answer(h, p).await?;
                }
            }
            self.chain.fast_forward(CHECK_WINDOW_SECS).await;
            for h in &ds {
                self.tool(&keeper, "finalize_dispute", json!({ "job": h.job }))
                    .await?;
            }
        }
        self.story.step(c, "The lead reviews: paid now, or disputed",
            "The lead's model reads each delivery and its score. For good, verified work its treasury buys the Job Note: the worker is paid now, not when the job closes.").await;

        // 7. Fold: the lead approves what it can; anything still open
        // (a reopened junk job) waits out the window. Then settle bottom-up.
        let mut wait = false;
        for h in &hired {
            if self
                .tool(&lead, "approve", json!({ "job": h.job }))
                .await
                .is_err()
            {
                wait = true;
            }
        }
        if wait {
            let j = self.tool(&lead, "get_job", json!({ "job": root })).await?;
            self.chain
                .wait_past(num(&j["window_end_ns"]) as u64, self.window_secs)
                .await;
        }
        for h in &hired {
            self.tool(&keeper, "settle", json!({ "job": h.job }))
                .await?;
        }
        let _ = self.tool(&client, "approve", json!({ "job": root })).await;
        self.tool(&keeper, "settle", json!({ "job": root })).await?;
        let mut recs = vec![];
        for w in &self.team.workers {
            let r = self
                .tool(&lead, "get_record", json!({ "account": w.acc.id }))
                .await?;
            recs.push(format!("{} {}/{} paid", w.name, r["paid"], r["jobs"]));
        }
        self.story.step(c, "The tree folds; track records update",
            &format!("Each account pays out, proves its balance is zero and deletes itself; the client gets the unspent budget back. Each finished job reported its worker's outcome to the factory: {}.", recs.join(", "))).await;
        Ok(root)
    }

    async fn check_round(&mut self, hired: &[Hired], panel: u8) -> Result<(), String> {
        for h in hired {
            self.answer(h, panel).await?;
        }
        Ok(())
    }

    /// Keeper: ask the judge model the contract's question; submit the answer.
    async fn answer(&mut self, h: &Hired, panel: u8) -> Result<(), String> {
        let keeper = self.team.keeper.clone();
        let d = h.delivered.as_ref().ok_or("nothing delivered")?;
        if std::str::from_utf8(d).is_err() {
            return Ok(());
        }
        let req = self
            .tool(
                &keeper,
                "check_request",
                json!({ "job": h.job, "panel": panel, "task": h.task, "deliverable": b64(d) }),
            )
            .await?;
        let body = base64::engine::general_purpose::STANDARD
            .decode(req.as_str().ok_or("check_request")?)
            .map_err(|e| e.to_string())?;
        self.calls += 1;
        let r = self.llm.chat(&body).await?;
        match self
            .tool(
                &keeper,
                "submit_check",
                json!({ "job": h.job, "panel": panel, "task": h.task,
            "deliverable": b64(d), "answer": b64(&r.body), "sig": r.sig }),
            )
            .await
        {
            Ok(score) => self.story.decision(
                "keeper",
                &format!("judge says {score}/10 for {} (panel {panel})", h.name),
                "signed NEAR AI answer, verified on-chain",
            ),
            Err(e) => self.story.decision(
                "keeper",
                &format!("judge answer for {} refused", h.name),
                &e,
            ),
        }
        Ok(())
    }
}

/// The mock model used in tests: deterministic answers per role.
pub fn mock_script(req: &Value) -> String {
    let sys = req["messages"][0]["content"].as_str().unwrap_or_default();
    let user = req["messages"][1]["content"].as_str().unwrap_or_default();
    if sys.starts_with("ROLE: planner") {
        return json!({ "subtasks": [
            { "name": "research", "task": "Explain NEAR global contracts in 3 sentences.", "pay": 30 },
            { "name": "translate", "task": "Translate the explanation into Spanish.", "pay": 20 },
            { "name": "summary", "task": "Write a one-line summary.", "pay": 10 } ] }).to_string();
    }
    if sys.starts_with("ROLE: picker") {
        let u: Value = serde_json::from_str(user).unwrap_or(Value::Null);
        let name = u["subtask"]["name"].as_str().unwrap_or_default();
        let mut best: Option<(i64, &Value)> = None;
        let mut avoided = vec![];
        for c in u["candidates"].as_array().into_iter().flatten() {
            if !c["skills"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|s| s.as_str() == Some(name))
            {
                continue;
            }
            let (jobs, paid) = (
                c["record"]["jobs"].as_i64().unwrap_or(0),
                c["record"]["paid"].as_i64().unwrap_or(0),
            );
            let score = paid - 2 * (jobs - paid);
            if jobs > paid {
                avoided.push(format!(
                    "{} ({jobs} jobs, {paid} paid)",
                    c["account"].as_str().unwrap_or("?")
                ));
            }
            if best.map(|b| score > b.0).unwrap_or(true) {
                best = Some((score, c));
            }
        }
        let c = best.map(|b| b.1).unwrap_or(&Value::Null);
        let reason = if avoided.is_empty() {
            "skills match; no bad record yet".to_string()
        } else {
            format!(
                "skills match; avoided {} on its on-chain record",
                avoided.join(", ")
            )
        };
        return json!({ "worker": c["account"], "reason": reason }).to_string();
    }
    if sys.starts_with("ROLE: reviewer") {
        let u: Value = serde_json::from_str(user).unwrap_or(Value::Null);
        let q = u["quality"].as_u64().unwrap_or(0);
        if !u["note_quote"].is_null() && q >= 7 {
            return json!({ "action": "buy_note", "reason": "checked, verified, good work: pay now" }).to_string();
        }
        return json!({ "action": "none", "reason": "not checked as good; settle decides" })
            .to_string();
    }
    if sys.starts_with("ROLE: worker") {
        return format!(
            "Deliverable: {}",
            user.split_once(' ').map(|x| x.1).unwrap_or(user)
        );
    }
    // The contract's judge question.
    let u: Value = serde_json::from_str(user).unwrap_or(Value::Null);
    let d = u["deliverable"].as_str().unwrap_or_default();
    let score = if d.contains("lorem") { 2 } else { 8 };
    json!({ "score": score }).to_string()
}

/// Accounts for the story (sandbox: created and funded here).
pub async fn team(c: &Chain) -> Team {
    let mk = |n: &'static str| async move { c.account(n, 10).await };
    let (client, lead, treasury, keeper) = (
        mk("client").await,
        mk("lead").await,
        mk("treasury").await,
        mk("keeper").await,
    );
    let researcher = mk("researcher").await;
    let translator = mk("translator").await;
    let cheapbot = mk("cheapbot").await;
    for a in [
        &client,
        &lead,
        &treasury,
        &researcher,
        &translator,
        &cheapbot,
    ] {
        c.register(a).await;
    }
    c.mint(&client.id, 200 * USD).await;
    c.mint(&treasury.id, 100 * USD).await;
    c.call(
        &client,
        &c.hub,
        "prepay_post",
        json!({}),
        near_api::types::NearToken::from_millinear(120),
    )
    .await
    .unwrap();
    Team {
        client,
        lead,
        treasury,
        keeper,
        workers: vec![
            WorkerAgent {
                acc: cheapbot,
                name: "cheapbot".into(),
                skills: vec!["summary".into(), "translate".into()],
                junk: true,
            },
            WorkerAgent {
                acc: researcher,
                name: "researcher".into(),
                skills: vec!["research".into(), "summary".into()],
                junk: false,
            },
            WorkerAgent {
                acc: translator,
                name: "translator".into(),
                skills: vec!["translate".into()],
                junk: false,
            },
        ],
    }
}

/// Two jobs: in job 1 the lead has no records and hires the cheap bot for
/// one part; in job 2 its record steers the lead away.
pub async fn story<'a>(c: &'a Chain, llm: &'a Llm) -> Result<Run<'a>, String> {
    let team = team(c).await;
    let mut people = vec![
        ("client".into(), team.client.id.clone()),
        ("lead".into(), team.lead.id.clone()),
        ("treasury".into(), team.treasury.id.clone()),
    ];
    for w in &team.workers {
        people.push((w.name.clone(), w.acc.id.clone()));
    }
    let story = Story {
        people,
        ..Default::default()
    };
    let window_secs = std::env::var("HT_WINDOW_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if c.sbx.is_some() {
            JOB_WINDOW_SECS
        } else {
            1200
        });
    let mut run = Run {
        chain: c,
        llm,
        team,
        story,
        calls: 0,
        window_secs,
    };
    let task = "Write a short explainer of NEAR global contracts in English and Spanish, plus a one-line summary.";
    run.job(1, task, 100 * USD, 10 * USD).await?;
    run.job(2, task, 100 * USD, 10 * USD).await?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(system: &str, user: Value) -> Value {
        let req = json!({ "messages": [{ "content": system }, { "content": user.to_string() }] });
        json_of(&mock_script(&req)).unwrap()
    }

    #[test]
    fn json_of_takes_the_object_out_of_fences() {
        assert_eq!(json_of("```json\n{\"a\":1}\n```").unwrap()["a"], 1);
        assert!(json_of("no json here").is_none());
    }

    #[test]
    fn picker_avoids_a_worker_with_unpaid_jobs() {
        let cands = |bot_jobs: u32| {
            json!({ "subtask": { "name": "summary" }, "candidates": [
            { "account": "bot", "skills": ["summary"], "record": { "jobs": bot_jobs, "paid": 0 } },
            { "account": "good", "skills": ["summary"], "record": { "jobs": 0, "paid": 0 } },
            { "account": "other", "skills": ["research"], "record": { "jobs": 9, "paid": 9 } } ] })
        };
        assert_eq!(
            ask(PICKER, cands(0))["worker"],
            "bot",
            "no record: first match"
        );
        let p = ask(PICKER, cands(2));
        assert_eq!(p["worker"], "good");
        assert!(p["reason"].as_str().unwrap().contains("avoided bot"));
    }

    #[test]
    fn reviewer_buys_only_quoted_good_work() {
        let r = |q: u8, quote: Value| {
            ask(REVIEWER, json!({ "quality": q, "note_quote": quote }))["action"].clone()
        };
        assert_eq!(r(8, json!("29700000")), "buy_note");
        assert_eq!(r(8, Value::Null), "none");
        assert_eq!(r(2, json!("1")), "none");
    }

    #[test]
    fn mock_judge_scores_filler_low() {
        let judge = |d: &str| {
            let req = json!({ "messages": [{ "content": "You grade" }, { "content": json!({ "deliverable": d }).to_string() }] });
            mock_script(&req)
        };
        assert_eq!(judge("lorem ipsum"), r#"{"score":2}"#);
        assert_eq!(judge("a real answer"), r#"{"score":8}"#);
    }
}
