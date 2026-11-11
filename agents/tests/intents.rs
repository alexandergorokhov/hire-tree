//! NEAR Intents on a local node, with the official Verifier contract (the
//! defuse 0.2.10 release wasm from github.com/near/intents). A worker's pay
//! lands as its balance in the Verifier, the worker swaps it with a solver
//! through signed NEP-413 `token_diff` intents (atomic: both or neither),
//! a replay of the same intents fails, and the worker withdraws the token it
//! swapped into.

use hire_tree_agents::chain::{Acc, Chain, USD};
use hire_tree_agents::llm::{Llm, Mock};
use hire_tree_agents::swarm::{pay_through_job_via, wash_budget, SWARM_FEE_BPS, WASH_HIRE};
use hire_tree_agents::tools::Tools;
use hire_tree_agents::verifier::{self, nep141};
use near_api::types::NearToken;
use serde_json::json;

const MODEL: &str = "test/judge-model";
const NEAR: u128 = 10u128.pow(24);

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

async fn wnear_register(c: &Chain, payer: &Acc, wnear: &near_api::types::AccountId, who: &str) {
    c.call(
        payer,
        wnear,
        "storage_deposit",
        json!({ "account_id": who, "registration_only": true }),
        NearToken::from_millinear(13),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn pay_lands_in_the_intents_verifier_and_the_worker_swaps_it_atomically() {
    let llm = Llm::Mock(Mock::new(
        MODEL,
        Box::new(hire_tree_agents::agents::mock_script),
    ));
    let c = Chain::sandbox(&llm.signer().unwrap(), Some(MODEL)).await;
    let tools = Tools { chain: &c };
    let v = verifier::deploy(&c).await.unwrap();
    let usdc = c.usdc.to_string();
    let wnear = v.wnear.to_string();

    let keeper = c.account("ikeeper", 5).await;
    let lead = c.account("ilead", 5).await;
    c.register(&lead).await;
    let worker = c.account("iworker", 5).await;
    let solver = c.account("isolver", 10).await;
    v.add_key(&c, &worker).await.unwrap();
    v.add_key(&c, &solver).await.unwrap();

    // The solver brings 2 wNEAR into the Verifier.
    for who in [solver.id.as_str(), v.id.as_str(), worker.id.as_str()] {
        wnear_register(&c, &solver, &v.wnear, who).await;
    }
    c.call(
        &solver,
        &v.wnear,
        "near_deposit",
        json!({}),
        NearToken::from_near(2),
    )
    .await
    .unwrap();
    c.call(
        &solver,
        &v.wnear,
        "ft_transfer_call",
        json!({ "receiver_id": v.id, "amount": (2 * NEAR).to_string(), "msg": solver.id }),
        NearToken::from_yoctonear(1),
    )
    .await
    .unwrap();
    assert_eq!(v.balance(&c, solver.id.as_str(), &wnear).await, 2 * NEAR);

    // A real job pays the worker; the worker routed its pay to the Verifier.
    let budget = wash_budget(1, SWARM_FEE_BPS);
    let cl = funded_client(&c, "iclient", budget).await;
    let root = pay_through_job_via(
        &c,
        &llm,
        &tools,
        &keeper,
        &cl,
        &lead,
        std::slice::from_ref(&worker),
        budget,
        "translate a page",
        Some(v.id.as_str()),
    )
    .await
    .unwrap();
    assert!(
        !c.alive(&format!("t1.{root}").parse().unwrap()).await,
        "the job folded"
    );
    assert_eq!(c.ft(&worker.id).await, 0, "no plain transfer");
    assert_eq!(v.balance(&c, worker.id.as_str(), &usdc).await, WASH_HIRE);

    // Swap: the worker gives its pay, the solver gives 1 wNEAR. Both sign.
    let deadline = verifier::deadline_in(&c, 600).await;
    let w_int = v
        .sign_diff(
            &worker,
            json!({ nep141(&usdc): format!("-{WASH_HIRE}"), nep141(&wnear): NEAR.to_string() }),
            [1u8; 32],
            &deadline,
        )
        .await
        .unwrap();
    let s_int = v
        .sign_diff(
            &solver,
            json!({ nep141(&usdc): WASH_HIRE.to_string(), nep141(&wnear): format!("-{NEAR}") }),
            [2u8; 32],
            &deadline,
        )
        .await
        .unwrap();

    // One side alone does not balance: the Verifier refuses and nothing moves.
    assert!(v.execute(&c, &keeper, vec![w_int.clone()]).await.is_err());
    assert_eq!(v.balance(&c, worker.id.as_str(), &usdc).await, WASH_HIRE);

    v.execute(&c, &keeper, vec![w_int.clone(), s_int.clone()])
        .await
        .unwrap();
    assert_eq!(v.balance(&c, worker.id.as_str(), &usdc).await, 0);
    assert_eq!(v.balance(&c, worker.id.as_str(), &wnear).await, NEAR);
    assert_eq!(v.balance(&c, solver.id.as_str(), &usdc).await, WASH_HIRE);
    assert_eq!(v.balance(&c, solver.id.as_str(), &wnear).await, NEAR);

    // The same signed intents cannot run twice (nonces are spent).
    assert!(v.execute(&c, &keeper, vec![w_int, s_int]).await.is_err());

    // The worker takes its wNEAR out to its own account.
    v.withdraw(&c, &worker, &wnear, NEAR).await.unwrap();
    assert_eq!(v.balance(&c, worker.id.as_str(), &wnear).await, 0);
    let held = c
        .view(
            &v.wnear,
            "ft_balance_of",
            json!({ "account_id": worker.id }),
        )
        .await
        .unwrap();
    assert_eq!(held.as_str().unwrap(), NEAR.to_string());
    let _ = USD;
}
