//! `hiretree`: run the agent story, check NEAR AI, verify a cassette, or
//! serve the tools over MCP.
//!
//! ```text
//! hiretree demo [--net sandbox|testnet] [--llm mock|live|record:FILE|replay:FILE] [--scene FILE]
//! hiretree nearai-check            # one tiny signed call; GO / NO-GO
//! hiretree verify-cassette FILE    # re-check every signature offline
//! hiretree mcp --as NAME           # stdio MCP server on testnet as NAME.<operator>
//! ```
//! Env: NEARAI_API_KEY, HT_MODEL, HT_OPERATOR, HT_OPERATOR_KEY, HT_RPC.

use hire_tree_agents::agents::{mock_script, story};
use hire_tree_agents::chain::{out_dir, Acc, Chain};
use hire_tree_agents::llm::{chat_body, recover, Cassette, Llm, Mock, NearAi, Source};
use hire_tree_agents::tools::Tools;
use serde_json::json;
use std::sync::Mutex;

const DEFAULT_MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash-1017";

fn model() -> String {
    std::env::var("HT_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into())
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}

async fn llm_from(spec: &str) -> Llm {
    match spec.split_once(':') {
        _ if spec == "mock" => Llm::Mock(Mock::new(&model(), Box::new(mock_script))),
        _ if spec == "live" => Llm::Live(NearAi::from_env(&model()).unwrap_or_else(|e| die(e))),
        Some(("replay", f)) => {
            let c = Cassette::load(f).unwrap_or_else(|e| die(e));
            let n = c
                .verify()
                .unwrap_or_else(|e| die(format!("cassette {f}: {e}")));
            eprintln!(
                "replaying {n} signed answers from {f} (signer {})",
                c.signer
            );
            Llm::Replay(c)
        }
        Some(("record", f)) => {
            let n = NearAi::from_env(&model()).unwrap_or_else(|e| die(e));
            let signer = n.signing_address().await.unwrap_or_else(|e| die(e));
            let c = Cassette {
                model: n.model.clone(),
                signer,
                signer_source: format!(
                    "{}/v1/attestation/report?model={}&signing_algo=ecdsa",
                    n.base, n.model
                ),
                entries: vec![],
            };
            Llm::Record(Source::Live(n), Mutex::new(c), f.into())
        }
        _ => die(format!("unknown --llm {spec}")),
    }
}

async fn demo(args: &[String]) {
    let net = arg(args, "--net").unwrap_or_else(|| "sandbox".into());
    let spec = arg(args, "--llm").unwrap_or_else(|| "mock".into());
    let llm = llm_from(&spec).await;
    let answers = match &llm {
        Llm::Mock(_) => "a scripted test model signing with a test key (not NEAR AI)".to_string(),
        Llm::Live(_) => "NEAR AI Cloud, live".to_string(),
        Llm::Record(..) => "NEAR AI Cloud, live, recorded to a cassette".to_string(),
        Llm::Replay(c) => format!(
            "NEAR AI Cloud answers replayed from a cassette, signatures as NEAR AI made them ({})",
            c.signer_source
        ),
    };
    let signer = match llm.signer() {
        Some(s) => s,
        None => match &llm {
            Llm::Live(n) => n.signing_address().await.unwrap_or_else(|e| die(e)),
            _ => unreachable!(),
        },
    };
    let model = llm.model();
    let c = match net.as_str() {
        "sandbox" => Chain::sandbox(&signer, Some(&model)).await,
        "testnet" => Chain::testnet(&signer, Some(&model))
            .await
            .unwrap_or_else(|e| die(e)),
        n => die(format!("unknown --net {n}")),
    };
    let run = story(&c, &llm).await.unwrap_or_else(|e| die(e));
    let scene = arg(args, "--scene").unwrap_or_else(|| format!("{}/scene.json", out_dir()));
    let txs: Vec<_> = c.txs.lock().unwrap().clone();
    run.story.save(
        &scene,
        &c.network_name,
        json!({ "model": model, "signer": signer, "answers": answers, "model_calls": run.calls, "explorer": c.explorer,
                "factory": c.hub, "token": c.usdc, "transactions": txs }),
    );
    eprintln!("{} model calls, {} transactions", run.calls, txs.len());
}

async fn nearai_check() {
    let n = NearAi::from_env(&model()).unwrap_or_else(|e| die(e));
    match n.models().await {
        Ok(m) => println!("models served: {}", m.join(", ")),
        Err(e) => println!("models: {e}"),
    }
    let attested = n.signing_address().await;
    println!("attestation signing address for {}: {attested:?}", n.model);
    let body = chat_body(
        &n.model,
        "Reply with one word.",
        "HT1:check.testnet:00:1 Say OK.",
        4,
    );
    match n.chat(&body).await {
        Ok(r) => {
            let addr = recover(&n.model, &body, &r.body, &r.sig).unwrap_or_else(|e| die(e));
            println!("reply: {:?}", r.content());
            println!("signature recovers to {addr}");
            match attested {
                Ok(a) if a == addr => println!("GO: NEAR AI signs model:sha256(request):sha256(response) with the attested key"),
                Ok(a) => die(format!("NO-GO: signature by {addr}, attestation says {a}")),
                Err(_) => println!("GO (signature format); the attestation lookup failed, so export HT_SIGNER={addr}"),
            }
        }
        Err(e) => die(e),
    }
}

async fn mcp(args: &[String]) {
    let name = arg(args, "--as").unwrap_or_else(|| die("--as NAME (e.g. lead)"));
    let c = Chain::testnet_connect().unwrap_or_else(|e| die(e));
    let who = Acc {
        id: format!("{name}.{}", c.funder.id).parse().unwrap(),
        signer: c.derived(&name),
    };
    hire_tree_agents::mcp::serve(Tools { chain: &c }, who).await;
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("demo") => demo(&args).await,
        Some("nearai-check") => nearai_check().await,
        Some("verify-cassette") => {
            let f = args.get(2).unwrap_or_else(|| die("verify-cassette FILE"));
            let c = Cassette::load(f).unwrap_or_else(|e| die(e));
            let n = c.verify().unwrap_or_else(|e| die(e));
            println!(
                "{n} entries, all signed by {} ({}), model {}",
                c.signer, c.signer_source, c.model
            );
        }
        Some("mcp") => mcp(&args).await,
        _ => {
            eprintln!("usage: hiretree demo|nearai-check|verify-cassette FILE|mcp --as NAME");
            std::process::exit(2)
        }
    }
}
