//! A washed record on a local node, and the buyer's payer list that does not
//! count it. A sybil gets a record that crosses the plain "trusted" rule
//! through real factory jobs among accounts it owns; a buyer that lists only
//! its own managers reads that record as empty, and the job contract's
//! `award_best` picks accordingly. Without the list the pick is the old one.

use hire_tree_agents::chain::{num, Acc, Chain, USD};
use hire_tree_agents::llm::{Llm, Mock};
use hire_tree_agents::swarm::{
    award_args, pay_through_job, trusted, wash_budget, wash_records, WashRig, SWARM_FEE_BPS,
    WASH_HIRE,
};
use hire_tree_agents::tools::Tools;
use serde_json::{json, Value};

const MODEL: &str = "test/judge-model";

async fn funded_client(c: &Chain, name: &str, budget: u128) -> Acc {
    let cl = c.account(name, 2).await;
    c.register(&cl).await;
    c.mint(&cl.id, budget).await;
    c.call(
        &cl,
        &c.hub,
        "prepay_post",
        json!({}),
        near_api::types::NearToken::from_millinear(60),
    )
    .await
    .unwrap();
    cl
}

async fn open_slot(tools: &Tools<'_>, lead: &Acc, root: &str, task: &str) -> u64 {
    tools
        .call(
            lead,
            "open_bids",
            &json!({ "job": root, "task": task, "max": (30 * USD).to_string(), "secs": 60 }),
        )
        .await
        .unwrap()
        .as_u64()
        .unwrap()
}

#[tokio::test]
async fn a_washed_record_beats_the_plain_rule_but_not_the_buyers_payer_list() {
    let llm = Llm::Mock(Mock::new(
        MODEL,
        Box::new(hire_tree_agents::agents::mock_script),
    ));
    let c = Chain::sandbox(&llm.signer().unwrap(), Some(MODEL)).await;
    let tools = Tools { chain: &c };
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
    let keeper = c.account("tkeeper", 5).await;
    let lead = c.account("tlead", 5).await;
    c.register(&lead).await;
    let honest = c.account("t-honest", 5).await;
    let sybil = c.account("t-sybil", 5).await;
    c.register(&honest).await;
    c.register(&sybil).await;

    // The sybil washes: two of its own managers pay it through real jobs.
    let rig = WashRig::new(&c).await;
    let cost = wash_records(
        &c,
        &llm,
        &tools,
        &keeper,
        &rig,
        1,
        std::slice::from_ref(&sybil),
    )
    .await
    .unwrap();
    assert_eq!((cost.jobs, cost.hires), (2, 2));
    assert!(
        cost.fee_fusd > 0 && cost.fee_fusd < USD / 2,
        "{}",
        cost.fee_fusd
    );
    assert_eq!(cost.outlay_fusd, cost.fee_fusd, "the fee is all it cost");
    assert!(cost.near > 0, "NEAR is measured");
    let whole = c
        .view(&c.hub, "get_record", json!({ "account_id": sybil.id }))
        .await
        .unwrap();
    assert_eq!(num(&whole["paid_value"]), 2 * WASH_HIRE);
    assert_eq!(whole["payers"], 2);
    assert!(trusted(&whole), "the plain rule trusts the washed record");
    assert_eq!(c.ft(&sybil.id).await, 0, "the pay went back to the bank");

    // The honest worker is paid once by the lead.
    let cl = funded_client(&c, "t-client", wash_budget(1, SWARM_FEE_BPS)).await;
    pay_through_job(
        &c,
        &llm,
        &tools,
        &keeper,
        &cl,
        &lead,
        std::slice::from_ref(&honest),
        wash_budget(1, SWARM_FEE_BPS),
        "honest work",
    )
    .await
    .unwrap();

    // The factory's view for a buyer that lists only its lead.
    let listed = |who: &Acc| {
        let c = &c;
        let lead = lead.id.clone();
        let who = who.id.clone();
        async move {
            c.view(
                &c.hub,
                "get_record_from",
                json!({ "account_id": who, "payers": [lead] }),
            )
            .await
            .unwrap()
        }
    };
    let s = listed(&sybil).await;
    assert_eq!((num(&s["paid_value"]), s["payers"].as_u64()), (0, Some(0)));
    assert_eq!(
        s["jobs"], whole["jobs"],
        "the rest of the record is the same"
    );
    let h = listed(&honest).await;
    assert_eq!(num(&h["paid_value"]), WASH_HIRE);
    assert_eq!(h["payers"], 1);

    // A root job of the lead with two slots; both workers bid on each
    // (honest 25, sybil 10).
    let budget = 70 * USD;
    let buyer_cl = funded_client(&c, "t-buyer", budget).await;
    let next = c.view(&c.hub, "get_settings", json!({})).await.unwrap()["next"]
        .as_u64()
        .unwrap();
    let root = format!("job-{next}.{}", c.hub);
    tools
        .call(
            &buyer_cl,
            "post_job",
            &json!({ "budget": budget.to_string(), "manager": lead.id, "task": "buyer job",
                "window_secs": 3600, "lead_fee": "0" }),
        )
        .await
        .unwrap();
    let with_list = open_slot(&tools, &lead, &root, "part one").await;
    let without = open_slot(&tools, &lead, &root, "part two").await;
    for slot in [with_list, without] {
        for (w, price) in [(&honest, 25 * USD), (&sybil, 10 * USD)] {
            tools
                .call(
                    w,
                    "bid",
                    &json!({ "job": root, "slot": slot, "price": price.to_string() }),
                )
                .await
                .unwrap();
        }
    }
    c.fast_forward(61).await;
    let leads = vec![lead.id.to_string()];
    let awarded = |slot: u64| {
        let (c, root) = (&c, root.clone());
        async move {
            let b = c
                .view(&root.parse().unwrap(), "get_bids", json!({ "slot": slot }))
                .await
                .unwrap();
            b["awarded"].as_str().map(String::from)
        }
    };
    // With the list: the sybil is a newcomer (10 x 4 = 40 > 25), honest wins.
    tools
        .call(
            &lead,
            "award_best",
            &award_args(&root, with_list, 150, 300, &[], Some(&leads)),
        )
        .await
        .unwrap();
    assert_eq!(awarded(with_list).await, Some(honest.id.to_string()));
    // Without it (the old call): the washed record is a record, the sybil's
    // 10 beats 25.
    tools
        .call(
            &lead,
            "award_best",
            &award_args(&root, without, 150, 300, &[], None),
        )
        .await
        .unwrap();
    assert_eq!(awarded(without).await, Some(sybil.id.to_string()));
    let _: Value = json!(null);
}
