//! The judge's playground: `hiretree mcp --local`. A stdio MCP server that
//! starts its own local NEAR node on the first tool call, deploys the
//! contracts, creates and funds the demo accounts, and lets any MCP client
//! (a judge's Claude, for example) act as any of them with `as`. Nothing
//! leaves the machine: the node is local, the model is the scripted test
//! model signing with a test key (pinned by this local factory), and all
//! money is fake.

use crate::agents::{b64, mock_script, story, team, Team, CHECK_WINDOW_SECS};
use crate::chain::{Acc, Chain};
use crate::llm::{chat_body, Llm, Mock};
use crate::mcp::PROTOCOL;
use crate::tools::{list, Tools};
use base64::Engine;
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
}

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
Or call run_story once to watch the whole scripted three-job story (bids, a junk bot losing work by its record, three clients pooling money); it takes a few minutes.";

impl Playground {
    pub async fn boot() -> Playground {
        let llm = Llm::Mock(Mock::new(MODEL, Box::new(mock_script)));
        let chain = Chain::sandbox(&llm.signer().unwrap(), Some(MODEL)).await;
        let team = team(&chain).await;
        Playground {
            chain,
            llm,
            team,
            delivered: Default::default(),
        }
    }

    fn named(&self) -> Vec<(String, &Acc)> {
        let t = &self.team;
        let mut v = vec![
            ("client".to_string(), &t.client),
            ("client2".to_string(), &t.members[0]),
            ("client3".to_string(), &t.members[1]),
            ("lead".to_string(), &t.lead),
            ("treasury".to_string(), &t.treasury),
            ("keeper".to_string(), &t.keeper),
        ];
        for w in &t.workers {
            v.push((w.name.clone(), &w.acc));
        }
        v
    }

    fn who(&self, a: &Value) -> Result<Acc, String> {
        let name = a["as"].as_str().unwrap_or("client");
        self.named()
            .into_iter()
            .find(|(n, acc)| n == name || acc.id.as_str() == name)
            .map(|(_, acc)| acc.clone())
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
                Ok(json!({ "factory": c.hub, "token": c.usdc, "accounts": out }))
            }
            "wait" => {
                let secs = a["secs"].as_u64().ok_or("missing secs")?.min(7200);
                c.fast_forward(secs).await;
                Ok(json!(format!("{secs} s of chain time passed")))
            }
            "do_work" => self.do_work(a).await,
            "judge_check" => self.judge_check(a).await,
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

/// The tools a judge sees: the shared tool list (with `as`) plus the
/// playground's own.
pub fn tool_list() -> Vec<Value> {
    let mut v: Vec<Value> = list()
        .into_iter()
        .map(|(name, description, mut schema)| {
            schema["properties"]["as"] = json!({ "type": "string", "description": "which demo account acts (default client)" });
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
                    (Some(p), _) => p.call(&name, &args).await,
                    (None, _) => unreachable!(),
                };
                let (text, err) = match r {
                    Ok(Value::String(s)) => (s, false),
                    Ok(v) => (v.to_string(), false),
                    Err(e) => (e, true),
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
