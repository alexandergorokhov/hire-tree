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
    assert_eq!(res["paid"], json!(3), "researcher {res}");
    assert_eq!(tra["paid"], json!(1), "translator {tra}");
    assert_eq!(c.ft(&w("cheapbot")).await, 0);
    // Money: each job costs the client fee 10 + paid parts; notes bought at 1% off.
    // Job 1 pays research 30 only; job 2 pays 30 + 20 + 10.
    let client = c.ft(&t.client.id).await;
    let lead = c.ft(&t.lead.id).await;
    let paid_out = c.ft(&w("researcher")).await + c.ft(&w("translator")).await;
    assert_eq!(lead, 20 * USD, "lead keeps two fees");
    assert_eq!(
        c.ft(&t.treasury.id).await + paid_out + client + lead,
        300 * USD,
        "no fUSD created or lost"
    );
    assert_eq!(
        client,
        200 * USD - 20 * USD - 90 * USD,
        "client pays fees + delivered work only"
    );
    for (_, id) in &run.story.jobs {
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
    let replay = Llm::Replay(cas);
    let c = Chain::sandbox(&signer, Some(MODEL)).await;
    let run = story(&c, &replay).await.unwrap();
    check_outcome(&c, &run).await;
}
