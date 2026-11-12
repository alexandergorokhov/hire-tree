//! `hiretree`: run the agent story, check NEAR AI, verify a cassette, or
//! serve the tools over MCP.
//!
//! ```text
//! hiretree demo [--net sandbox|testnet] [--llm mock|live|record:FILE|replay:FILE] [--scene FILE]
//! hiretree nearai-check            # one tiny signed call; GO / NO-GO
//! hiretree verify-cassette FILE    # re-check every signature offline
//! hiretree mcp --as NAME           # stdio MCP server on testnet as NAME.<operator>
//! hiretree mcp --local             # stdio MCP playground on a local node (no keys)
//! hiretree rubric-check [--items FILE] [--llm mock|record:FILE|replay:FILE] # the live run's deliveries (or a probe set) under judge rubric 2
//! hiretree probe-make --tasks FILE [--llm mock|record:FILE|replay:FILE] [--out FILE] # a harder filler probe: per task a model answer, a model-written filler, a task-copy filler
//! hiretree testnet-answers         # read a testnet run's signed judge answers back from the chain into a cassette
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
                    rubric: Some(ht_core::check::RUBRIC_LATEST),
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
                rubric: Some(ht_core::check::RUBRIC_LATEST),
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
        "sandbox" => Chain::sandbox_with(&signer, Some(&model), llm.rubric()).await,
        "testnet" => Chain::testnet_with(&signer, Some(&model), llm.rubric())
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

/// The live run's deliveries, graded again under judge rubric 2:
///   hiretree rubric-check [--from demo/swarm-cassette.json] [--llm mock|record:FILE|replay:FILE] [--out FILE]
/// `record` asks NEAR AI (the source run's model) once per delivery.
async fn rubric_check(args: &[String]) {
    let from =
        arg(args, "--from").unwrap_or_else(|| format!("{}/../demo/swarm-cassette.json", out_dir()));
    let src = Cassette::load(&from).unwrap_or_else(|e| die(e));
    src.verify().unwrap_or_else(|e| die(format!("{from}: {e}")));
    let items = match arg(args, "--items") {
        Some(f) => hire_tree_agents::rubric::items_from_file(&f).unwrap_or_else(|e| die(e)),
        None => hire_tree_agents::rubric::items_from(&src),
    };
    let spec = arg(args, "--llm").unwrap_or_else(|| "mock".into());
    let llm = graded_llm(&spec, &src, arg(args, "--reuse")).await;
    eprintln!(
        "grading {} deliveries from {from} under rubric 2 ({spec})",
        items.len()
    );
    let r = hire_tree_agents::rubric::run(&items, &llm)
        .await
        .unwrap_or_else(|e| die(e));
    let out = arg(args, "--out").unwrap_or_else(|| format!("{}/rubric2.json", out_dir()));
    std::fs::write(&out, serde_json::to_string_pretty(&r).unwrap() + "\n")
        .unwrap_or_else(|e| die(e));
    for (k, v) in r["by_kind"].as_object().into_iter().flatten() {
        println!(
            "{k}: rubric 2 passed {} of {}",
            v["passed_rubric_2"], v["deliveries"]
        );
    }
    for k in ["filler", "other"] {
        println!(
            "{k:7} rubric 1 passed {} of {}, rubric 2 passed {} of {}",
            r[k]["rubric_1"]["passed"],
            r[k]["rubric_1"]["deliveries"],
            r[k]["rubric_2"]["passed"],
            r[k]["rubric_2"]["deliveries"]
        );
    }
    println!("report: {out}");
}

/// The LLM for re-grading: `record:FILE` asks NEAR AI (the source run's
/// model) and records into FILE; otherwise as `llm_from`. With `reuse`
/// (an earlier recording, by the same signer) its signed answers to the
/// same requests are copied instead of asked again.
async fn graded_llm(spec: &str, src: &Cassette, reuse: Option<String>) -> Llm {
    match spec.split_once(':') {
        Some(("record", f)) => {
            let n = NearAi::from_env(&src.model).unwrap_or_else(|e| die(e));
            let signer = n.signing_address().await.unwrap_or_else(|e| die(e));
            if !signer.eq_ignore_ascii_case(src.signer.trim_start_matches("0x")) {
                eprintln!(
                    "note: NEAR AI now signs with {signer}; the source run was signed by {}",
                    src.signer
                );
            }
            n.expect.lock().unwrap().replace(signer.to_lowercase());
            let source = match reuse {
                Some(r) => {
                    let old = Cassette::load(&r).unwrap_or_else(|e| die(e));
                    if !old
                        .signer
                        .trim_start_matches("0x")
                        .eq_ignore_ascii_case(&signer)
                    {
                        die(format!(
                            "{r} was signed by {}, NEAR AI now signs with {signer}: cannot reuse",
                            old.signer
                        ));
                    }
                    Source::reusing(Source::Live(n), old)
                        .unwrap_or_else(|e| die(format!("{r}: {e}")))
                }
                None => Source::Live(n),
            };
            Llm::Record(
                source,
                Mutex::new(Cassette {
                    model: src.model.clone(),
                    signer: signer.clone(),
                    signer_source: src.signer_source.clone(),
                    entries: vec![],
                    rubric: Some(2),
                }),
                f.into(),
            )
        }
        _ => llm_from(spec).await,
    }
}

/// A harder filler probe, made by the model itself:
///   hiretree probe-make --tasks runs/probe2-tasks.json [--llm mock|record:FILE|replay:FILE] [--out FILE]
/// Per task: a real answer (worker prompt), a filler the model writes to
/// look like the work, and a filler that copies the task. Writes the items
/// for `rubric-check --items`. Uses the swarm run's model.
async fn probe_make(args: &[String]) {
    let from = format!("{}/../demo/swarm-cassette.json", out_dir());
    let src = Cassette::load(&from).unwrap_or_else(|e| die(e));
    let tasks = arg(args, "--tasks").unwrap_or_else(|| die("--tasks FILE"));
    let spec = arg(args, "--llm").unwrap_or_else(|| "mock".into());
    let llm = graded_llm(&spec, &src, arg(args, "--reuse")).await;
    let items = hire_tree_agents::rubric::make_probe(&tasks, &llm)
        .await
        .unwrap_or_else(|e| die(e));
    let out = arg(args, "--out").unwrap_or_else(|| format!("{}/probe-items.json", out_dir()));
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&hire_tree_agents::rubric::items_json(&items)).unwrap() + "\n",
    )
    .unwrap_or_else(|e| die(e));
    println!("{} probe items: {out}", items.len());
}

/// The signed judge answers of a testnet run, fetched back from the chain:
///   hiretree testnet-answers [--scene web/scene-testnet.json] [--out demo/testnet-judge-cassette.json]
/// Reads every `submit_check` transaction the scene lists from testnet RPC
/// (HT_RPC), rebuilds the exact request each job built (rubric 2), and
/// saves request, answer and signature as a cassette, so `verify-cassette`
/// re-checks them offline.
async fn testnet_answers(args: &[String]) {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let root = format!("{}/..", out_dir());
    let scene_f = arg(args, "--scene").unwrap_or_else(|| format!("{root}/web/scene-testnet.json"));
    let out =
        arg(args, "--out").unwrap_or_else(|| format!("{root}/demo/testnet-judge-cassette.json"));
    let scene: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&scene_f).unwrap_or_else(|e| die(e)))
            .unwrap_or_else(|e| die(e));
    let run = &scene["run"];
    let (model, signer) = (
        run["model"].as_str().unwrap_or_default(),
        run["signer"].as_str().unwrap_or_default(),
    );
    let rpc = std::env::var("HT_RPC").unwrap_or_else(|_| "https://rpc.testnet.near.org".into());
    let http = reqwest::Client::new();
    let mut cas = Cassette {
        model: model.into(),
        signer: signer.into(),
        signer_source: format!(
            "signed answers submitted on NEAR testnet, read back from {}'s submit_check transactions",
            std::path::Path::new(&scene_f).file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
        ),
        entries: vec![],
        rubric: Some(2),
    };
    for t in run["transactions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["method"] == "submit_check" && t["ok"] == true)
    {
        let (hash, who, job) = (
            t["tx"].as_str().unwrap(),
            t["who"].as_str().unwrap(),
            t["on"].as_str().unwrap(),
        );
        let body = json!({ "jsonrpc": "2.0", "id": "ht", "method": "tx",
            "params": { "tx_hash": hash, "sender_account_id": who, "wait_until": "NONE" } });
        let mut got = None;
        for wait in [0u64, 2, 4, 8] {
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            if let Ok(r) = http.post(&rpc).json(&body).send().await {
                if let Ok(v) = r.json::<serde_json::Value>().await {
                    if v["result"].is_object() {
                        got = Some(v);
                        break;
                    }
                }
            }
        }
        let v = got.unwrap_or_else(|| die(format!("tx {hash}: no answer from {rpc}")));
        let a = &v["result"]["transaction"]["actions"][0]["FunctionCall"];
        let call: serde_json::Value = serde_json::from_slice(
            &b64.decode(a["args"].as_str().unwrap_or_default())
                .unwrap_or_default(),
        )
        .unwrap_or_else(|e| die(format!("tx {hash}: args: {e}")));
        let task = call["task"].as_str().unwrap_or_default();
        let deliverable = String::from_utf8(
            b64.decode(call["deliverable"].as_str().unwrap_or_default())
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        let answer = String::from_utf8(
            b64.decode(call["answer"].as_str().unwrap_or_default())
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        let sig = call["sig"].as_str().unwrap_or_default().to_string();
        let panel = call["panel"].as_u64().unwrap_or(0) as u8;
        // The delivery number is not in the call: the one whose request the signature covers.
        let req = (1..=8u32)
            .map(|seq| {
                String::from_utf8(ht_core::check::request_bytes_v(
                    2,
                    model,
                    job,
                    seq,
                    panel,
                    task,
                    &deliverable,
                ))
                .unwrap()
            })
            .find(|req| {
                recover(model, req.as_bytes(), answer.as_bytes(), &sig)
                    .is_ok_and(|a| a.eq_ignore_ascii_case(signer))
            })
            .unwrap_or_else(|| {
                die(format!(
                    "tx {hash}: no delivery number makes the signature recover to {signer}"
                ))
            });
        let content = hire_tree_agents::llm::Reply {
            body: answer.clone().into_bytes(),
            sig: sig.clone(),
        }
        .content();
        println!(
            "{hash} {job} panel {panel}: counted {:?} ({content})",
            hire_tree_agents::rubric::read_v2(&content, &deliverable)
        );
        cas.entries.push(hire_tree_agents::llm::Entry {
            req,
            resp: answer,
            sig,
        });
    }
    let n = cas.verify().unwrap_or_else(|e| die(e));
    cas.save(&out).unwrap_or_else(|e| die(e));
    println!("ok   {n} testnet judge answers, all signed by {signer}: {out}");
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
        Some("rubric-check") => rubric_check(&args).await,
        Some("probe-make") => probe_make(&args).await,
        Some("testnet-answers") => testnet_answers(&args).await,
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
            let c = Chain::sandbox_with(&signer, Some(&llm.model()), llm.rubric()).await;
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
