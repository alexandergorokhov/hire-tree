//! The MCP playground end to end, as a judge's Claude would use it: spawn
//! `scripts/mcp-local.sh`, speak JSON-RPC over stdio, and walk the guide:
//! post, open bids, bid (one refused), award, deliver with a signed receipt,
//! judge, pay now, settle and fold. Run with `make mcp-test`.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    _child: Child,
    tx: ChildStdin,
    rx: BufReader<ChildStdout>,
    id: u64,
}

impl Client {
    fn start() -> Client {
        let root = format!("{}/..", env!("CARGO_MANIFEST_DIR"));
        let mut child = Command::new("bash")
            .arg("scripts/mcp-local.sh")
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn scripts/mcp-local.sh");
        let tx = child.stdin.take().unwrap();
        let rx = BufReader::new(child.stdout.take().unwrap());
        Client {
            _child: child,
            tx,
            rx,
            id: 0,
        }
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let m = json!({ "jsonrpc": "2.0", "id": self.id, "method": method, "params": params });
        writeln!(self.tx, "{m}").unwrap();
        let mut line = String::new();
        self.rx.read_line(&mut line).unwrap();
        let v: Value =
            serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON-RPC: {line:?} ({e})"));
        assert_eq!(v["id"], json!(self.id));
        v["result"].clone()
    }

    /// Call a tool; returns (text, is_error).
    fn tool(&mut self, name: &str, args: Value) -> (String, bool) {
        let r = self.rpc("tools/call", json!({ "name": name, "arguments": args }));
        let text = r["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        eprintln!(
            "{name} {args} -> {}",
            text.chars().take(200).collect::<String>()
        );
        (text, r["isError"].as_bool().unwrap_or(true))
    }

    fn ok(&mut self, name: &str, args: Value) -> String {
        let (t, err) = self.tool(name, args.clone());
        assert!(!err, "{name} {args} failed: {t}");
        t
    }
}

#[test]
fn a_judge_walks_the_guide_over_mcp() {
    let mut c = Client::start();
    let init = c.rpc("initialize", json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "judge", "version": "1" } }));
    assert_eq!(init["serverInfo"]["name"], "hire-tree-playground");
    writeln!(
        c.tx,
        "{}",
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
    )
    .unwrap();
    let tools = c.rpc("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for n in [
        "guide",
        "accounts",
        "post_job",
        "open_bids",
        "bid",
        "award",
        "do_work",
        "judge_check",
        "buy_note",
        "settle",
        "join_pool",
        "run_story",
    ] {
        assert!(names.contains(&n), "missing tool {n}");
    }
    assert!(c.ok("guide", json!({})).contains("post_job"));

    let acc: Value = serde_json::from_str(&c.ok("accounts", json!({}))).unwrap();
    let id = |n: &str| -> String {
        acc["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["as"] == n)
            .unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let factory = acc["factory"].as_str().unwrap().to_string();
    let (lead, researcher) = (id("lead"), id("researcher"));

    // 1. Post.
    let task = "Explain NEAR sub-accounts";
    c.ok(
        "post_job",
        json!({ "as": "client", "budget": "50000000", "manager": lead, "task": task,
        "window_secs": 3600, "lead_fee": "5000000", "treasury": id("treasury") }),
    );
    let root = format!("job-1.{factory}");
    let j: Value = serde_json::from_str(&c.ok("get_job", json!({ "job": root }))).unwrap();
    assert_eq!(j["funded"], "50000000");

    // 2-4. Bids.
    let sub_task = "Explain NEAR sub-accounts in 3 sentences";
    let slot = c.ok(
        "open_bids",
        json!({ "as": "lead", "job": root, "task": sub_task, "max": "30000000", "secs": 60 }),
    );
    let slot: u64 = slot.trim().parse().unwrap();
    c.ok(
        "bid",
        json!({ "as": "researcher", "job": root, "slot": slot, "price": "25000000" }),
    );
    c.ok(
        "bid",
        json!({ "as": "cheapbot", "job": root, "slot": slot, "price": "10000000" }),
    );
    let (t, err) = c.tool(
        "bid",
        json!({ "as": "translator", "job": root, "slot": slot, "price": "31000000" }),
    );
    assert!(
        err && t.contains("refused by the contract: bid is above the slot's max"),
        "{t}"
    );
    c.ok("wait", json!({ "secs": 61 }));
    c.ok(
        "award",
        json!({ "as": "lead", "job": root, "slot": slot, "worker": researcher }),
    );
    let t1 = format!("t1.{root}");
    let j: Value = serde_json::from_str(&c.ok("get_job", json!({ "job": t1 }))).unwrap();
    assert_eq!(j["amount"], "25000000");

    // 5-6. Deliver with a signed receipt; the judge scores it.
    let w: Value = serde_json::from_str(&c.ok(
        "do_work",
        json!({ "as": "researcher", "job": t1, "task": sub_task }),
    ))
    .unwrap();
    assert!(w["delivered"].as_str().unwrap().contains("sub-accounts"));
    let s: Value =
        serde_json::from_str(&c.ok("judge_check", json!({ "job": t1, "task": sub_task }))).unwrap();
    assert_eq!(s["score"], json!(8));

    // 7. Close the round, pay now.
    c.ok("wait", json!({ "secs": 301 }));
    c.ok("finalize_check", json!({ "as": "keeper", "job": t1 }));
    let quote = c.ok("quote_note", json!({ "job": t1 }));
    let quote = quote.trim().trim_matches('"').to_string();
    assert_eq!(quote, "24750000", "1% off 25 fUSD");
    c.ok(
        "buy_note",
        json!({ "as": "treasury", "job": t1, "price": quote }),
    );
    let acc: Value = serde_json::from_str(&c.ok("accounts", json!({}))).unwrap();
    let r = acc["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["as"] == "researcher")
        .unwrap();
    assert_eq!(r["fUSD"], "24.75", "paid now, 1% off");

    // 8. Fold.
    c.ok("approve", json!({ "as": "lead", "job": t1 }));
    c.ok("settle", json!({ "as": "keeper", "job": t1 }));
    c.ok("approve", json!({ "as": "client", "job": root }));
    c.ok("settle", json!({ "as": "keeper", "job": root }));
    let (t, err) = c.tool("get_job", json!({ "job": root }));
    assert!(
        err && t.contains("deletes itself"),
        "the root deleted itself: {t}"
    );
    let rec: Value =
        serde_json::from_str(&c.ok("get_record", json!({ "account": researcher }))).unwrap();
    assert_eq!(rec["paid"], json!(1));
}

/// The judge's own agent joins the open market, outbids the house, does
/// the work and is paid early; then it delivers junk once, and its record
/// costs it the next award even though it bids lowest again.
#[test]
fn a_judges_agent_competes_in_the_open_market() {
    let mut c = Client::start();
    c.rpc("initialize", json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "judge", "version": "1" } }));
    let j: Value = serde_json::from_str(&c.ok(
        "join",
        json!({ "name": "judgebot", "skills": ["research"] }),
    ))
    .unwrap();
    let me = j["account"].as_str().unwrap().to_string();

    let round = |c: &mut Client, junk: bool| -> Value {
        let r: Value = serde_json::from_str(&c.ok("open_round", json!({}))).unwrap();
        let job = r["job"].as_str().unwrap().to_string();
        let slot = r["slots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["skill"] == "research")
            .unwrap();
        // The house researcher asks 95% of max (28.50); undercut it.
        c.ok(
            "bid",
            json!({ "as": "judgebot", "job": job, "slot": slot["slot"], "price": "20000000" }),
        );
        let task = slot["task"].as_str().unwrap().to_string();
        let a: Value = serde_json::from_str(&c.ok("close_round", json!({ "job": job }))).unwrap();
        let mine = a["awards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["skill"] == "research")
            .unwrap()
            .clone();
        if mine["award"] == me.as_str() {
            let sub = mine["sub_job"].as_str().unwrap();
            let mut w = json!({ "as": "judgebot", "job": sub, "task": task });
            if junk {
                w["text"] = json!("lorem ipsum dolor sit amet");
            }
            c.ok("do_work", w);
            c.ok("judge_check", json!({ "job": sub, "task": task }));
        }
        let f: Value = serde_json::from_str(&c.ok("finish_round", json!({ "job": job }))).unwrap();
        json!({ "award": mine, "finish": f })
    };

    // Round 1: the lowest bid and no bad record: judgebot wins, is paid 19.80 now.
    let r1 = round(&mut c, false);
    assert_eq!(r1["award"]["award"], json!(me), "{r1}");
    let wk = r1["finish"]["workers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["as"] == "judgebot")
        .unwrap();
    assert_eq!(wk["fUSD"], "19.80", "{r1}");
    // Round 2: judgebot wins again but delivers junk: 2/10, not paid.
    let r2 = round(&mut c, true);
    assert_eq!(r2["award"]["award"], json!(me), "{r2}");
    let wk = r2["finish"]["workers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["as"] == "judgebot")
        .unwrap();
    assert_eq!(wk["fUSD"], "19.80", "junk earns nothing: {r2}");
    // Round 3: lowest bid again, but its record (2 jobs, 1 paid) loses the award.
    let r3 = round(&mut c, false);
    assert_ne!(r3["award"]["award"], json!(me), "{r3}");
    assert!(
        r3["award"]["reason"].as_str().unwrap().contains("avoided"),
        "{r3}"
    );
}

/// The judge's own agent wins a part, then becomes a contractor: it puts a
/// piece of its job out for bids out of its own pay, hires a house worker
/// into a sub-sub-account, keeps the margin, and the tree folds bottom-up.
#[test]
fn a_judges_agent_becomes_a_contractor() {
    let mut c = Client::start();
    c.rpc("initialize", json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "judge", "version": "1" } }));
    let j: Value = serde_json::from_str(&c.ok(
        "join",
        json!({ "name": "contractor", "skills": ["research"] }),
    ))
    .unwrap();
    let me = j["account"].as_str().unwrap().to_string();
    let accts: Value = serde_json::from_str(&c.ok("accounts", json!({}))).unwrap();
    let researcher = accts["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["as"] == "researcher")
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();

    let r: Value = serde_json::from_str(&c.ok("open_round", json!({}))).unwrap();
    let job = r["job"].as_str().unwrap().to_string();
    let slot = r["slots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["skill"] == "research")
        .unwrap()
        .clone();
    let task = slot["task"].as_str().unwrap().to_string();
    c.ok(
        "bid",
        json!({ "as": "contractor", "job": job, "slot": slot["slot"], "price": "20000000" }),
    );
    let a: Value = serde_json::from_str(&c.ok("close_round", json!({ "job": job }))).unwrap();
    let mine = a["awards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["skill"] == "research")
        .unwrap()
        .clone();
    assert_eq!(mine["award"], json!(me), "{a}");
    let part = mine["sub_job"].as_str().unwrap().to_string();

    // A sub-slot paid out of its own 20 fUSD; the house bids at once.
    let s: Value = serde_json::from_str(&c.ok(
        "subcontract",
        json!({ "as": "contractor", "job": part, "task": "List 3 sources for the explainer.",
            "skill": "research", "max": "8000000" }),
    ))
    .unwrap();
    let bids = s["bids"].as_array().unwrap();
    assert!(
        bids.iter().any(|b| b["worker"] == researcher.as_str()),
        "{s}"
    );
    // The worker cannot bid on its own job: the contract refuses it.
    let (e, err) = c.tool(
        "bid",
        json!({ "as": "contractor", "job": part, "slot": s["slot"], "price": "1000000" }),
    );
    assert!(err && e.contains("own job"), "{e}");
    let h: Value = serde_json::from_str(&c.ok(
        "hire_sub",
        json!({ "as": "contractor", "job": part, "slot": s["slot"], "worker": researcher }),
    ))
    .unwrap();
    assert_eq!(h["sub_job"], json!(format!("t1.{part}")), "{h}");
    assert_eq!(h["score"], json!(8), "{h}");
    let price = h["price_fusd"].as_f64().unwrap();
    assert!(price > 0.0 && price <= 8.0, "{h}");
    assert!(
        (h["margin_fusd"].as_f64().unwrap() - (20.0 - price)).abs() < 1e-6,
        "{h}"
    );

    c.ok(
        "do_work",
        json!({ "as": "contractor", "job": part, "task": task }),
    );
    c.ok("judge_check", json!({ "job": part, "task": task }));
    let f: Value = serde_json::from_str(&c.ok("finish_round", json!({ "job": job }))).unwrap();
    let out = f["outcome"].to_string();
    assert!(out.contains("accepted by its contractor"), "{f}");
    let fusd = |who: &str| -> f64 {
        f["workers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["as"] == who)
            .unwrap()["fUSD"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    };
    // The contractor keeps its margin (less the 1% note fee if paid early).
    let got = fusd("contractor");
    assert!(
        got >= (20.0 - price) * 0.99 - 0.01 && got <= 20.0 - price + 0.01,
        "{f}"
    );
}
