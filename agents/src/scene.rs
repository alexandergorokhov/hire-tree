//! Records the story for the web page (`out/scene.json`): each step has a
//! title, a narration and a snapshot of the tree and the wallets, read from
//! the chain right after the step's transactions.

use crate::chain::{num, Chain};
use near_api::types::AccountId;
use serde_json::{json, Value};

#[derive(Default)]
pub struct Story {
    pub steps: Vec<Value>,
    pub jobs: Vec<(String, AccountId)>,
    pub people: Vec<(String, AccountId)>,
    pub log: Vec<Value>,
}

impl Story {
    pub fn job(&mut self, label: &str, id: &AccountId) {
        self.jobs.push((label.into(), id.clone()));
    }

    /// An agent's decision, kept for the page and the transcript.
    pub fn decision(&mut self, who: &str, what: &str, reason: &str) {
        println!("    [{who}] {what}: {reason}");
        self.log
            .push(json!({ "who": who, "what": what, "reason": reason }));
    }

    pub async fn step(&mut self, c: &Chain, title: &str, say: &str) {
        let n = self.steps.len() + 1;
        println!("\nSCENE {n}: {title}\n  {say}");
        let mut tree = vec![];
        for (label, id) in &self.jobs {
            if c.alive(id).await {
                let j = c
                    .view(id, "get_job", json!({}))
                    .await
                    .unwrap_or(Value::Null);
                tree.push(json!({ "label": label, "account": id, "alive": true,
                    "parent": j["parent"], "status": j["status"], "worker": j["worker"],
                    "payee": j["payee"], "amount": j["amount"], "ft": c.ft(id).await.to_string(),
                    "verified": j["verified"], "quality": j["quality"], "note_sold": j["note_sold"] }));
            } else {
                tree.push(json!({ "label": label, "account": id, "alive": false }));
            }
        }
        let mut wallets = vec![];
        for (label, id) in &self.people {
            let bal = c.ft(id).await;
            println!("    {label:<11} {:>10.2} fUSD", bal as f64 / 1e6);
            wallets.push(json!({ "label": label, "account": id, "ft": bal.to_string() }));
        }
        let decisions: Vec<Value> = std::mem::take(&mut self.log);
        self.steps
            .push(json!({ "n": n, "title": title, "say": say, "jobs": tree,
                                "wallets": wallets, "decisions": decisions }));
    }

    pub fn save(&self, path: &str, network: &str, extra: Value) {
        let doc = json!({ "schema": "hiretree-scene.v1", "network": network, "steps": self.steps, "run": extra });
        std::fs::write(path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        println!("\nscene written to {path}");
    }
}

pub fn usd(v: &Value) -> String {
    format!("{:.2}", num(v) as f64 / 1e6)
}
