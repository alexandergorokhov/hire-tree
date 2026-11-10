//! A stolen function-call key on a local node. A worker's `deliver` key is
//! limited to one method on one job account, so a thief holding it can
//! deliver (what the key is for) but cannot move NEAR or fUSD, delete the
//! account, attach a deposit, or call anything else.

use hire_tree_agents::agents::mock_script;
use hire_tree_agents::chain::{Acc, Chain, USD};
use hire_tree_agents::llm::{Llm, Mock};
use hire_tree_agents::tools::Tools;
use near_api::types::NearToken;
use near_api::{Account, Tokens};
use serde_json::json;

const MODEL: &str = "test/judge-model";

async fn refused<T, E: std::fmt::Debug>(what: &str, r: Result<T, E>) -> String {
    match r {
        Ok(_) => panic!("{what}: the stolen key was allowed to do it"),
        Err(e) => format!("{e:?}"),
    }
}

#[tokio::test]
async fn a_stolen_function_call_key_can_only_deliver_to_its_one_job() {
    let llm = Llm::Mock(Mock::new(MODEL, Box::new(mock_script)));
    let c = Chain::sandbox(&llm.signer().unwrap(), Some(MODEL)).await;
    let tools = Tools { chain: &c };
    let client = c.account("kclient", 5).await;
    let lead = c.account("klead", 5).await;
    let worker = c.account("kworker", 5).await;
    let other = c.account("kother", 5).await;
    for a in [&client, &lead, &worker, &other] {
        c.register(a).await;
    }
    c.mint(&client.id, 100 * USD).await;
    c.mint(&worker.id, 7 * USD).await;
    c.call(
        &client,
        &c.hub,
        "prepay_post",
        json!({}),
        NearToken::from_millinear(60),
    )
    .await
    .unwrap();
    tools
        .call(
            &client,
            "post_job",
            &json!({ "budget": (50 * USD).to_string(), "manager": lead.id, "task": "Explain NEAR keys",
                "window_secs": 3600, "lead_fee": (5 * USD).to_string() }),
        )
        .await
        .unwrap();
    let root = format!("job-1.{}", c.hub);
    tools
        .call(
            &lead,
            "hire",
            &json!({ "job": root, "worker": worker.id, "amount": (20 * USD).to_string(), "task": "Explain NEAR keys" }),
        )
        .await
        .unwrap();
    let child: near_api::types::AccountId = format!("t1.{root}").parse().unwrap();

    // The owner gives the agent's runtime a key for this one job.
    let key = c
        .grant_key(
            &worker,
            "work",
            &child,
            &["deliver"],
            NearToken::from_millinear(500),
        )
        .await
        .unwrap();
    let near_before = c.near_of(&worker.id).await;
    let fusd_before = c.ft(&worker.id).await;

    // A thief with `key` can do what the key is for, and nothing else.
    let thief = Acc {
        id: key.id.clone(),
        signer: key.signer.clone(),
    };
    let junk = tools
        .call(
            &thief,
            "deliver",
            &json!({ "job": child, "resp": hire_tree_agents::agents::b64(b"lorem ipsum") }),
        )
        .await;
    assert!(junk.is_ok(), "the key can deliver on its job: {junk:?}");

    let e = refused(
        "fUSD transfer",
        c.send(&thief, &other.id, 7 * USD, None).await,
    )
    .await;
    eprintln!("ft_transfer refused: {e}");
    let e = refused(
        "NEAR transfer",
        Tokens::account(thief.id.clone())
            .send_to(other.id.clone())
            .near(NearToken::from_near(1))
            .with_signer(thief.signer.clone())
            .send_to(&c.net)
            .await,
    )
    .await;
    eprintln!("NEAR transfer refused: {e}");
    let e = refused(
        "delete account",
        Account(thief.id.clone())
            .delete_account_with_beneficiary(other.id.clone())
            .with_signer(thief.signer.clone())
            .send_to(&c.net)
            .await,
    )
    .await;
    eprintln!("delete account refused: {e}");
    let e = refused(
        "deliver with a deposit",
        c.call(
            &thief,
            &child,
            "deliver",
            json!({}),
            NearToken::from_yoctonear(1),
        )
        .await,
    )
    .await;
    eprintln!("deposit refused: {e}");
    refused(
        "another method on the same job",
        c.call(
            &thief,
            &child,
            "approve",
            json!({}),
            NearToken::from_yoctonear(0),
        )
        .await,
    )
    .await;
    refused(
        "another job",
        c.call(
            &thief,
            &root.parse().unwrap(),
            "deliver",
            json!({}),
            NearToken::from_yoctonear(0),
        )
        .await,
    )
    .await;
    refused(
        "adding a key",
        Account(thief.id.clone())
            .add_key(
                near_api::types::AccessKeyPermission::FullAccess,
                other.signer.get_public_key().await.unwrap(),
            )
            .with_signer(thief.signer.clone())
            .send_to(&c.net)
            .await,
    )
    .await;

    // Nothing moved: the account still holds its fUSD and (gas aside) its NEAR.
    assert_eq!(c.ft(&worker.id).await, fusd_before);
    assert!(
        c.near_of(&worker.id).await + NearToken::from_millinear(600).as_yoctonear() > near_before
    );
    assert_eq!(c.ft(&other.id).await, 0);

    // The owner revokes it; the key is dead.
    c.revoke_key(&worker, &key).await.unwrap();
    refused(
        "a revoked key",
        c.call(
            &thief,
            &child,
            "deliver",
            json!({}),
            NearToken::from_yoctonear(0),
        )
        .await,
    )
    .await;
}
