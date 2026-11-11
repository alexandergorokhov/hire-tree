//! The agent story on a local node with the mock model (signed with a test
//! key exactly like NEAR AI), then the same story replayed from a cassette
//! recorded during the first run: the replay path is the one Alex's real
//! NEAR AI recording will take.

use hire_tree_agents::agents::{mock_script, story};
use hire_tree_agents::chain::{num, Chain, USD};
use hire_tree_agents::llm::{Cassette, Llm, Mock};
use serde_json::{json, Value};

const MODEL: &str = "test/judge-model";

fn mock() -> Mock {
    Mock::new(MODEL, Box::new(mock_script))
}

async fn record_of(c: &Chain, who: &str) -> Value {
    c.view(&c.hub, "get_record", json!({ "account_id": who }))
        .await
        .unwrap()
}

async fn check_outcome(c: &Chain, run: &hire_tree_agents::agents::Run<'_>) {
    let t = &run.team;
    let w = |n: &str| {
        t.workers
            .iter()
            .find(|w| w.name == n)
            .unwrap()
            .acc
            .id
            .clone()
    };
    // Job 1: no records, the cheap bot wins two parts and delivers filler;
    // the judge scores it 2, the parts reopen and their pay goes back.
    let bot = record_of(c, w("cheapbot").as_str()).await;
    assert_eq!(bot["jobs"], json!(2), "cheapbot record {bot}");
    assert_eq!(bot["paid"], json!(0), "cheapbot record {bot}");
    // Job 2: the picker read that record and hired the others.
    let res = record_of(c, w("researcher").as_str()).await;
    let tra = record_of(c, w("translator").as_str()).await;
    // Job 3 (pooled) hires the same two again: +2 and +1.
    assert_eq!(res["paid"], json!(5), "researcher {res}");
    assert_eq!(tra["paid"], json!(2), "translator {tra}");
    assert_eq!(c.ft(&w("cheapbot")).await, 0);
    // Money: each job costs the client the fee the lead earned + paid parts;
    // notes bought at 1% off. The fee is earned pro rata to the parts paid:
    // job 1 pays research 30 of the 48 hired (cheapbot's 12 and 6 failed),
    // so its fee of 10 earns 10 x 30 / 48 = 6.25; job 2 pays 30 + 20 + 10
    // and job 3 (pooled, fee 9) pays every part, so both earn the whole fee.
    let client = c.ft(&t.client.id).await;
    let lead = c.ft(&t.lead.id).await;
    let paid_out = c.ft(&w("researcher")).await + c.ft(&w("translator")).await;
    assert_eq!(
        lead,
        6_250_000 + 10 * USD + 9 * USD,
        "lead keeps three earned fees (6.25 + 10 + 9)"
    );
    let mut members = 0;
    for m in &t.members {
        // Each put in 30 of the pool's 90; 90 - 9 fee - 60 work = 21 came
        // back, 7 each.
        let got = c.ft(&m.id).await;
        assert_eq!(got, 40 * USD - 30 * USD + 7 * USD, "pool member {}", m.id);
        members += got;
    }
    assert_eq!(
        c.ft(&t.treasury.id).await + paid_out + client + lead + members,
        420 * USD,
        "no fUSD created or lost"
    );
    assert_eq!(
        client,
        240 * USD - (6_250_000 + 30 * USD) - (10 * USD + 60 * USD) - 30 * USD + 7 * USD,
        "client pays earned fees + delivered work only"
    );
    for (name, id) in &run.story.jobs {
        if name == "pool" {
            let v = c.view(id, "get_pool", json!({})).await.unwrap();
            assert_eq!(v["state"], json!("Done"), "pool {v}");
            assert_eq!(c.ft(id).await, 0, "the pool paid everything back");
            continue;
        }
        assert!(!c.alive(id).await, "{id} should have deleted itself");
    }
}

#[tokio::test]
async fn agents_story_with_mock_model() {
    let llm = Llm::Mock(mock());
    let c = Chain::sandbox(&llm.signer().unwrap(), Some(MODEL)).await;
    let run = story(&c, &llm).await.unwrap();
    check_outcome(&c, &run).await;
    let decisions: Vec<&Value> = run
        .story
        .steps
        .iter()
        .flat_map(|s| s["decisions"].as_array().unwrap())
        .collect();
    assert!(
        decisions
            .iter()
            .any(|d| d["what"].as_str().unwrap().starts_with("buy ")),
        "lead buys a note"
    );
    assert!(run
        .story
        .steps
        .iter()
        .all(|s| num(&s["n"]) == 0 || s["n"].is_u64()));
}

#[tokio::test]
async fn agents_story_replays_from_a_cassette() {
    // Record with the mock, then replay: every model answer and signature
    // comes from the file, and the contract verifies them again.
    let path = format!("{}/target/test-cassette.json", env!("CARGO_MANIFEST_DIR"));
    let _ = std::fs::remove_file(&path);
    let m = mock();
    let signer = hire_tree_agents::llm::addr_of(&m.key);
    let rec = Llm::Record(
        hire_tree_agents::llm::Source::Mock(m),
        std::sync::Mutex::new(Cassette {
            model: MODEL.into(),
            signer: signer.clone(),
            signer_source: "test key".into(),
            entries: vec![],
        }),
        path.clone(),
    );
    {
        let c = Chain::sandbox(&signer, Some(MODEL)).await;
        story(&c, &rec).await.unwrap();
    }
    let cas = Cassette::load(&path).unwrap();
    assert!(cas.verify().unwrap() > 10);
    let replay = Llm::replay(cas);
    let c = Chain::sandbox(&signer, Some(MODEL)).await;
    let run = story(&c, &replay).await.unwrap();
    check_outcome(&c, &run).await;
}

/// The testnet code path (derived keys, tight funding, reuse on rerun,
/// wall-clock waits) against a local node standing in for testnet RPC.
/// Slow (real 5-minute check windows): `make testnet-dryrun`.
#[tokio::test]
#[ignore]
async fn testnet_path_dry_run_on_a_local_node() {
    use near_sandbox::config::{DEFAULT_GENESIS_ACCOUNT, DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY};
    let sbx = near_sandbox::Sandbox::start_sandbox().await.unwrap();
    std::env::set_var("HT_RPC", &sbx.rpc_addr);
    std::env::set_var("HT_OPERATOR", DEFAULT_GENESIS_ACCOUNT.as_str());
    std::env::set_var("HT_OPERATOR_KEY", DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY);
    std::env::set_var("HT_WINDOW_SECS", "900");
    let llm = Llm::Mock(mock());
    let signer = llm.signer().unwrap();
    let probe = Chain::testnet_connect().unwrap();
    let start = probe.near_of(&probe.funder.id).await;
    let c = Chain::testnet(&signer, Some(MODEL)).await.unwrap();
    let deployed = probe.near_of(&probe.funder.id).await;
    // A rerun finds everything and deploys nothing.
    let c2 = Chain::testnet(&signer, Some(MODEL)).await.unwrap();
    assert_eq!(c2.hub, c.hub);
    assert!(
        start - probe.near_of(&probe.funder.id).await < start - deployed + 10u128.pow(23),
        "rerun must not redeploy"
    );
    let t0 = std::time::Instant::now();
    let run = story(&c, &llm).await.unwrap();
    check_outcome(&c, &run).await;
    let spent = start - probe.near_of(&probe.funder.id).await;
    println!(
        "operator spent {:.2} NEAR to deploy, {:.2} NEAR in total; story took {:?}",
        (start - deployed) as f64 / 1e24,
        spent as f64 / 1e24,
        t0.elapsed()
    );
}
