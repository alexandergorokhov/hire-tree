//! `hiretree`: run the agent story, check NEAR AI, verify a cassette, or
//! serve the tools over MCP.
//!
//! ```text
//! hiretree demo [--net sandbox|testnet] [--llm mock|live|record:FILE|replay:FILE] [--scene FILE]
//! hiretree nearai-check            # one tiny signed call; GO / NO-GO
//! hiretree verify-cassette FILE    # re-check every signature offline
//! hiretree mcp --as NAME           # stdio MCP server on testnet as NAME.<operator>
//! hiretree mcp --local             # stdio MCP playground on a local node (no keys)
//! hiretree swarm [--rounds N] [--out FILE] [--blind|--audit-all] [--seed N] [--storm R [--wash]] [--allow-list] [--llm mock|live|record:FILE|replay:FILE] # 3 leads, 36 agents, trees up to 3 deep, N rounds on a local node
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
        _ if spec == "mock" => Llm::Mock(Mock::new("scripted-test-model", Box::new(mock_script))),
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
            Llm::replay(c)
        }
        // The test model, recorded like NEAR AI: proves a run replays.
        Some(("record-mock", f)) => {
            let m = Mock::new("scripted-test-model", Box::new(mock_script));
            let signer = hire_tree_agents::llm::addr_of(&m.key);
            Llm::Record(
                Source::Mock(m),
                Mutex::new(Cassette {
                    model: "scripted-test-model".into(),
                    signer,
                    signer_source: "test key (not NEAR AI)".into(),
                    entries: vec![],
                }),
                f.into(),
            )
        }
        Some(("record", f)) => {
            let n = NearAi::from_env(&model()).unwrap_or_else(|e| die(e));
            let signer = n.signing_address().await.unwrap_or_else(|e| die(e));
            let c = Cassette {
                model: n.model.clone(),
                signer,
                signer_source: if std::env::var("HT_SIGNER").is_ok() {
                    "HT_SIGNER (set by the owner after make nearai-check)".into()
                } else {
                    format!(
                        "{}/v1/attestation/report?model={}&signing_algo=ecdsa (address read from the report; the TEE quote is not verified here)",
                        n.base, n.model
                    )
                },
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
        Llm::Replay(c, _) => format!(
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
    llm.expect_signer(&signer);
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
    // 1. The receipt format, on a tagged worker-style request.
    let body = chat_body(
        &n.model,
        "Reply with one word.",
        "HT1:check.testnet:00:1 Say OK.",
        4,
    );
    let r = n.chat(&body).await.unwrap_or_else(|e| die(e));
    let addr = recover(&n.model, &body, &r.body, &r.sig).unwrap_or_else(|e| die(e));
    println!("reply: {:?}", r.content());
    println!("signature recovers to {addr}");
    match &attested {
        Ok(a) if *a == addr => println!("ok: NEAR AI signs model:sha256(request):sha256(response) with the address in its attestation report (the TEE quote itself is not verified here)"),
        Ok(a) => die(format!("NO-GO: signature by {addr}, attestation report says {a}")),
        Err(_) => println!("ok (format); the attestation lookup failed, so export HT_SIGNER={addr}"),
    }
    // 2. The judge: the contract's exact request must give a score it can read.
    let req = ht_core::check::request_bytes(
        &n.model,
        "t1.job-1.hub.check.testnet",
        1,
        0,
        "Explain NEAR global contracts in 3 sentences.",
        "Global contracts let many accounts share one deployed code by hash.",
    );
    let j = n.chat(&req).await.unwrap_or_else(|e| die(e));
    match ht_core::check::parse_score(&j.content()) {
        Some(sc) => println!("judge: {:?} -> score {sc}", j.content()),
        None => die(format!(
            "NO-GO: the judge reply {:?} has no score the contract can read (try another HT_MODEL)",
            j.content()
        )),
    }
    // 3. The planner: JSON within the token budget.
    let p = chat_body(&n.model, &hire_tree_agents::agents::PLANNER.replace("BUDGET", "90"),
        "Write a short explainer of NEAR global contracts in English and Spanish, plus a one-line summary.", 400);
    let pr = n.chat(&p).await.unwrap_or_else(|e| die(e));
    match hire_tree_agents::agents::json_of(&pr.content()) {
        Some(v) if v["subtasks"].is_array() => println!(
            "planner: {} subtasks",
            v["subtasks"].as_array().unwrap().len()
        ),
        _ => die(format!(
            "NO-GO: planner reply is not the JSON the lead needs: {:?} (try another HT_MODEL)",
            pr.content()
        )),
    }
    println!("GO");
}

async fn mcp(args: &[String]) {
    if args.iter().any(|a| a == "--local") {
        return hire_tree_agents::judge::serve().await;
    }
    let c = Chain::testnet_connect().unwrap_or_else(|e| die(e));
    // An outside agent brings its own account and key; demo roles may use
    // keys derived from the operator key, but never the factory or token.
    let who = match (
        std::env::var("HT_AGENT_ACCOUNT"),
        std::env::var("HT_AGENT_KEY"),
    ) {
        (Ok(id), Ok(key)) => Acc {
            id: id
                .parse()
                .unwrap_or_else(|e| die(format!("HT_AGENT_ACCOUNT: {e}"))),
            signer: near_api::Signer::from_secret_key(
                key.parse()
                    .unwrap_or_else(|e| die(format!("HT_AGENT_KEY: {e:?}"))),
            )
            .unwrap_or_else(|e| die(format!("{e:?}"))),
        },
        _ => {
            let name = arg(args, "--as").unwrap_or_else(|| {
                die("--as NAME (e.g. lead), or HT_AGENT_ACCOUNT + HT_AGENT_KEY")
            });
            if name == "hub" || name == "usdc" || name.starts_with('g') && name.len() == 13 {
                die("refusing to sign as the factory, the token or a probe account");
            }
            Acc {
                id: format!("{name}.{}", c.funder.id).parse().unwrap(),
                signer: c.derived(&name),
            }
        }
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
                "ok   {n} entries, all signed by {} ({}), model {}",
                c.signer, c.signer_source, c.model
            );
        }
        Some("mcp") => mcp(&args).await,
        Some("swarm") => {
            let jobs: u32 = arg(&args, "--rounds")
                .and_then(|j| j.parse().ok())
                .unwrap_or(12);
            let out = arg(&args, "--out").unwrap_or_else(|| format!("{}/swarm.json", out_dir()));
            // --llm mock (default), live, record:FILE or replay:FILE, as in demo.
            let spec = arg(&args, "--llm").unwrap_or_else(|| "mock".into());
            let llm = llm_from(&spec).await;
            let signer = match llm.signer() {
                Some(s) => s,
                None => match &llm {
                    Llm::Live(n) => n.signing_address().await.unwrap_or_else(|e| die(e)),
                    _ => unreachable!(),
                },
            };
            llm.expect_signer(&signer);
            let c = Chain::sandbox(&signer, Some(&llm.model())).await;
            let v = hire_tree_agents::swarm::run(
                &c,
                &llm,
                jobs,
                if args.iter().any(|a| a == "--blind") {
                    hire_tree_agents::swarm::Arm::Blind
                } else if args.iter().any(|a| a == "--audit-all") {
                    hire_tree_agents::swarm::Arm::AuditAll
                } else {
                    hire_tree_agents::swarm::Arm::Records
                },
                arg(&args, "--seed")
                    .and_then(|x| x.parse().ok())
                    .unwrap_or(0),
                hire_tree_agents::swarm::Opts {
                    storm: arg(&args, "--storm").and_then(|x| x.parse().ok()),
                    wash: args.iter().any(|a| a == "--wash"),
                    allow: args.iter().any(|a| a == "--allow-list"),
                },
            )
            .await
            .unwrap_or_else(|e| die(e));

            std::fs::write(&out, serde_json::to_string_pretty(&v).unwrap())
                .unwrap_or_else(|e| die(e.to_string()));
            eprintln!("swarm written to {out}");
        }
        _ => {
            eprintln!(
                "usage: hiretree demo|nearai-check|verify-cassette FILE|mcp --as NAME|mcp --local"
            );
            std::process::exit(2)
        }
    }
}
