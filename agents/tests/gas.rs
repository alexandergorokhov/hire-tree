//! What one part costs on-chain, measured on a local node: gas burnt by
//! every step of a part's life (post, bid, award_best with 4 and 16 bidders,
//! with a 16-payer list, deliver, check, settle and fold), plus the NEAR
//! attached for storage (returned when the accounts delete), and the same for
//! a payout through the NEAR Intents Verifier. With `HT_GAS_OUT=<path>` it
//! writes the numbers as JSON (`make gas` writes `runs/gas.json`, which
//! `scripts/economics.py` turns into docs/ECONOMICS.md).

use hire_tree_agents::chain::{Acc, Chain, USD};
use hire_tree_agents::llm::{Llm, Mock};
use hire_tree_agents::swarm::{
    award_args, pay_through_job, pay_through_job_via, wash_budget, SWARM_FEE_BPS,
};
use hire_tree_agents::tools::Tools;
use hire_tree_agents::verifier;
use near_api::types::NearToken;
use serde_json::{json, Map};

const MODEL: &str = "test/judge-model";

/// Gas burnt by the transactions sent since `from` (index into `c.txs`).
fn gas_since(c: &Chain, from: usize) -> u64 {
    c.txs.lock().unwrap()[from..]
        .iter()
        .map(|t| t["gas"].as_u64().unwrap_or(0))
        .sum()
}

fn mark(c: &Chain) -> usize {
    c.txs.lock().unwrap().len()
}

async fn funded_client(c: &Chain, name: &str, budget: u128) -> Acc {
    let cl = c.account(name, 2).await;
    c.register(&cl).await;
    c.mint(&cl.id, budget).await;
    c.call(
        &cl,
        &c.hub,
        "prepay_post",
        json!({}),
        NearToken::from_millinear(60),
    )
    .await
    .unwrap();
    cl
}

#[tokio::test]
async fn gas_and_storage_per_part() {
    let llm = Llm::Mock(Mock::new(
        MODEL,
        Box::new(hire_tree_agents::agents::mock_script),
    ));
    let c = Chain::sandbox(&llm.signer().unwrap(), Some(MODEL)).await;
    let tools = Tools { chain: &c };
    let keeper = c.account("gkeeper", 5).await;
    let lead = c.account("glead", 20).await;
    c.register(&lead).await;
    let mut steps = Map::new();
    let mut put = |k: &str, v: u64| {
        steps.insert(k.into(), json!(v));
    };

    // A root job with two bid slots: 4 bidders on one, 16 on the other.
    let budget = 70 * USD;
    let cl = funded_client(&c, "gclient", budget).await;
    let next = c.view(&c.hub, "get_settings", json!({})).await.unwrap()["next"]
        .as_u64()
        .unwrap();
    let root = format!("job-{next}.{}", c.hub);
    let m = mark(&c);
    tools
        .call(
            &cl,
            "post_job",
            &json!({ "budget": budget.to_string(), "manager": lead.id,
            "task": "gas job", "window_secs": 3600, "lead_fee": "0" }),
        )
        .await
        .unwrap();
    put("post_job", gas_since(&c, m));
    let mut slots = vec![];
    for task in ["four bidders", "sixteen bidders"] {
        let m = mark(&c);
        let s = tools
            .call(
                &lead,
                "open_bids",
                &json!({ "job": root, "task": task,
                "max": (30 * USD).to_string(), "secs": 120 }),
            )
            .await
            .unwrap()
            .as_u64()
            .unwrap();
        put("open_bids", gas_since(&c, m));
        slots.push(s);
    }
    let mut bidders = vec![];
    for i in 0..16 {
        let w = c.account(&format!("gw{i}"), 2).await;
        c.register(&w).await;
        bidders.push(w);
    }
    for (slot, n) in [(slots[0], 4usize), (slots[1], 16)] {
        for (i, w) in bidders.iter().take(n).enumerate() {
            let m = mark(&c);
            tools
                .call(
                    w,
                    "bid",
                    &json!({ "job": root, "slot": slot,
                    "price": (20 * USD + i as u128 * USD / 10).to_string() }),
                )
                .await
                .unwrap();
            put("bid", gas_since(&c, m));
        }
    }
    c.fast_forward(121).await;
    let payers: Vec<String> = (0..16).map(|i| format!("payer{i}.{}", c.hub)).collect();
    let m = mark(&c);
    tools
        .call(
            &lead,
            "award_best",
            &award_args(&root, slots[0], 150, 300, &[], None),
        )
        .await
        .unwrap();
    put("award_best_4_bidders", gas_since(&c, m));
    let m = mark(&c);
    tools
        .call(
            &lead,
            "award_best",
            &award_args(&root, slots[1], 150, 300, &[], Some(&payers)),
        )
        .await
        .unwrap();
    put("award_best_16_bidders_16_payers", gas_since(&c, m));

    // The first winner delivers, is checked, paid and its job folds.
    let job = c
        .view(&root.parse().unwrap(), "get_job", json!({}))
        .await
        .unwrap();
    let child = job["children"][0]["account"].as_str().unwrap().to_string();
    let winner = bidders
        .iter()
        .find(|b| job["children"][0]["worker"].as_str() == Some(b.id.as_str()))
        .unwrap();
    let task = "four bidders";
    use base64::Engine;
    let resp = base64::engine::general_purpose::STANDARD.encode(format!("Deliverable: {task}"));
    let m = mark(&c);
    tools
        .call(winner, "deliver", &json!({ "job": child, "resp": resp }))
        .await
        .unwrap();
    put("deliver", gas_since(&c, m));
    let m = mark(&c);
    let req = tools
        .call(
            &keeper,
            "check_request",
            &json!({ "job": child, "panel": 0, "task": task, "deliverable": resp }),
        )
        .await
        .unwrap();
    let qb = base64::engine::general_purpose::STANDARD
        .decode(req.as_str().unwrap())
        .unwrap();
    let ans = llm.chat(&qb).await.unwrap();
    tools
        .call(&keeper, "submit_check", &json!({ "job": child, "panel": 0, "task": task,
            "deliverable": resp, "answer": base64::engine::general_purpose::STANDARD.encode(&ans.body), "sig": ans.sig }))
        .await
        .unwrap();
    put("submit_check", gas_since(&c, m));
    c.fast_forward(hire_tree_agents::agents::CHECK_WINDOW_SECS + 1)
        .await;
    let m = mark(&c);
    tools
        .call(&keeper, "finalize_check", &json!({ "job": child }))
        .await
        .unwrap();
    put("finalize_check", gas_since(&c, m));
    let m = mark(&c);
    tools
        .call(&lead, "approve", &json!({ "job": child }))
        .await
        .unwrap();
    put("approve", gas_since(&c, m));
    let m = mark(&c);
    tools
        .call(&keeper, "settle", &json!({ "job": child }))
        .await
        .unwrap();
    put("settle_and_delete_sub_job", gas_since(&c, m));
    assert!(
        !c.alive(&child.parse().unwrap()).await,
        "the sub-job deleted itself"
    );

    // One whole plain part (post, hire, deliver, check, settle, fold) and the
    // same with the pay routed into the NEAR Intents Verifier.
    let v = verifier::deploy(&c).await.unwrap();
    let w1 = c.account("gpart", 2).await;
    let w2 = c.account("gpart-intents", 2).await;
    for (name, via, w) in [
        ("whole_part_plain", None, &w1),
        ("whole_part_via_intents", Some(v.id.as_str()), &w2),
    ] {
        let b = wash_budget(1, SWARM_FEE_BPS);
        let cl = funded_client(&c, &format!("gc-{name}"), b).await;
        let m = mark(&c);
        let near0 = c.near_of(&lead.id).await;
        if via.is_some() {
            pay_through_job_via(
                &c,
                &llm,
                &tools,
                &keeper,
                &cl,
                &lead,
                std::slice::from_ref(w),
                b,
                "part",
                via,
            )
            .await
            .unwrap();
        } else {
            pay_through_job(
                &c,
                &llm,
                &tools,
                &keeper,
                &cl,
                &lead,
                std::slice::from_ref(w),
                b,
                "part",
            )
            .await
            .unwrap();
        }
        put(name, gas_since(&c, m));
        put(&format!("{name}_txs"), (mark(&c) - m) as u64);
        let _ = near0;
    }

    let out = json!({
        "schema": "hiretree-gas-1",
        "network": "near-sandbox (local node)",
        "note": "gas = total gas burnt by each transaction and all its receipts; bid and open_bids are per call (last value)",
        "steps": steps,
        "attached_near": {
            "hire_or_award": "0.035",
            "bid_storage": "0.001",
            "post_prepay": "0.06"
        }
    });
    eprintln!("{}", serde_json::to_string_pretty(&out).unwrap());
    if let Ok(p) = std::env::var("HT_GAS_OUT") {
        std::fs::write(&p, serde_json::to_string_pretty(&out).unwrap() + "\n").unwrap();
    }
}
