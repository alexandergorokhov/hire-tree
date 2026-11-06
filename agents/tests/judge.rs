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
    assert!(err && t.contains("above the slot's max"), "{t}");
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
    assert!(err, "the root deleted itself: {t}");
    let rec: Value =
        serde_json::from_str(&c.ok("get_record", json!({ "account": researcher }))).unwrap();
    assert_eq!(rec["paid"], json!(1));
}
