//! `hiretree rubric-check`: the live NEAR AI run's deliveries, graded again
//! under judge rubric 2.
//!
//! In the 3-round live run (rubric 1) the polished-filler test agent passed
//! every check. This takes every delivery the judge saw in that run's
//! cassette, builds the request a rubric-2 job would build for it (same job,
//! delivery and task), asks the model, and scores the answer exactly as the
//! contract does (`ht_core::check::graded`: two quotes found in the work, or
//! at most 2). Recorded once live (a few cents); the recording replays
//! offline and every signature is re-checked.

use crate::llm::{Cassette, Llm};
use ht_core::check;

/// The job contract's pass mark (`PASS_SCORE` in contracts/job).
pub const PASS_SCORE_HINT: u8 = 6;
use serde_json::{json, Value};

/// One delivery of the recorded run.
#[derive(Clone, Debug)]
pub struct Item {
    pub job: String,
    pub delivery: u32,
    pub task: String,
    pub deliverable: String,
    /// Every rubric-1 score the judge gave it (one per panel answer).
    pub v1: Vec<u8>,
    /// A probe set's label for the delivery (e.g. "on-topic filler").
    pub kind: Option<String>,
}

impl Item {
    /// The polished-filler test agent's text (fluent, fits any task).
    pub fn filler(&self) -> bool {
        self.deliverable.contains(crate::agents::POLISHED_MARK)
    }
    /// What rubric 1 counted: the lowest answer, unless they split by 3+.
    pub fn v1_counted(&self) -> Option<u8> {
        let mut t = check::Tally::default();
        self.v1.iter().for_each(|s| t.add(*s));
        match t.outcome() {
            check::Outcome::Score(s) => Some(s),
            _ => None,
        }
    }
}

/// Every distinct (job, delivery) a rubric-1 judge request in `c` asked
/// about, in recorded order, with the scores it got.
pub fn items_from(c: &Cassette) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for e in &c.entries {
        let Ok(req) = serde_json::from_str::<Value>(&e.req) else {
            continue;
        };
        if req["messages"][0]["content"].as_str() != Some(check::RUBRIC) {
            continue;
        }
        let Ok(u) = serde_json::from_str::<Value>(
            req["messages"][1]["content"].as_str().unwrap_or_default(),
        ) else {
            continue;
        };
        let (job, delivery) = (
            u["job"].as_str().unwrap_or_default().to_string(),
            u["delivery"].as_u64().unwrap_or(0) as u32,
        );
        let content = serde_json::from_str::<Value>(&e.resp)
            .ok()
            .and_then(|v| {
                v["choices"][0]["message"]["content"]
                    .as_str()
                    .map(String::from)
            })
            .unwrap_or_default();
        let score = check::parse_score(&content);
        match out
            .iter_mut()
            .find(|i| i.job == job && i.delivery == delivery)
        {
            Some(i) => i.v1.extend(score),
            None => out.push(Item {
                job,
                delivery,
                task: u["task"].as_str().unwrap_or_default().into(),
                deliverable: u["deliverable"].as_str().unwrap_or_default().into(),
                v1: score.into_iter().collect(),
                kind: None,
            }),
        }
    }
    out
}

/// A probe set: `[{"job", "delivery", "task", "deliverable", "kind"}]`
/// (e.g. runs/filler-probe.json: real answers and on-topic filler for the
/// same tasks).
pub fn items_from_file(path: &str) -> Result<Vec<Item>, String> {
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)
            .map_err(|e| format!("{path}: {e}"))?;
    v.as_array()
        .ok_or(format!("{path}: not a list"))?
        .iter()
        .map(|i| {
            Ok(Item {
                job: i["job"].as_str().ok_or("job")?.into(),
                delivery: i["delivery"].as_u64().ok_or("delivery")? as u32,
                task: i["task"].as_str().ok_or("task")?.into(),
                deliverable: i["deliverable"].as_str().ok_or("deliverable")?.into(),
                v1: vec![],
                kind: i["kind"].as_str().map(String::from),
            })
        })
        .collect::<Result<_, &str>>()
        .map_err(|f| format!("{path}: an item has no {f}"))
}

/// The system prompt that asks the model for filler written for one task:
/// it looks finished and says nothing. (A first version only asked for text
/// that "does not do the task"; the model answered the task correctly
/// every time, so it was not filler.)
pub const FILLER_WRITER: &str = "ROLE: test fixture writer. A quality checker must learn to reject deliverables that only look finished. For the task in the user message, write such a fake deliverable: restate the task in your own words, say the work is complete and of high quality, and describe its structure and what it covers in general terms, but do NOT include the actual content: no actual facts, steps, translated sentence, risks, comparison or answer. Under 80 words. Reply with the fake deliverable only.";

/// Filler that copies the task word for word and claims it is done: every
/// quote the judge could take from it is the task's own text.
pub fn task_copy_filler(task: &str) -> String {
    format!(
        "Task: {task}\n\nDelivered as requested. {task} The deliverable meets every \
         requirement of the task, in the requested format and length, and was checked before delivery."
    )
}

/// A harder probe set from a list of tasks (`[{"job", "task"}]`): for each
/// task the model writes a real answer (the worker prompt) and a filler
/// told to look like the work (`FILLER_WRITER`), and `task_copy_filler`
/// adds a third. Recorded once; a replay gives the same items.
pub async fn make_probe(tasks_file: &str, llm: &Llm) -> Result<Vec<Item>, String> {
    let v: Value = serde_json::from_str(
        &std::fs::read_to_string(tasks_file).map_err(|e| format!("{tasks_file}: {e}"))?,
    )
    .map_err(|e| format!("{tasks_file}: {e}"))?;
    let model = llm.model();
    let mut out = Vec::new();
    for t in v.as_array().ok_or(format!("{tasks_file}: not a list"))? {
        let (job, task) = (
            t["job"].as_str().ok_or("a task has no job")?,
            t["task"].as_str().ok_or("a task has no task")?,
        );
        let ask = |system: &'static str, n: u32| {
            let body =
                crate::llm::chat_body(&model, system, &format!("HT1:{job}:probe:{n} {task}"), 300);
            async move {
                llm.chat(&body)
                    .await
                    .map(|r| r.content().trim().to_string())
            }
        };
        let item = |delivery: u32, deliverable: String, kind: &str| Item {
            job: job.into(),
            delivery,
            task: task.into(),
            deliverable,
            v1: vec![],
            kind: Some(kind.into()),
        };
        out.push(item(
            1,
            ask(crate::agents::WORKER, 1).await?,
            "real answer (model)",
        ));
        out.push(item(
            2,
            ask(FILLER_WRITER, 2).await?,
            "model-written filler",
        ));
        out.push(item(3, task_copy_filler(task), "task-copy filler"));
    }
    Ok(out)
}

/// Items as a probe file (`items_from_file` reads it back).
pub fn items_json(items: &[Item]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|i| json!({ "job": i.job, "delivery": i.delivery, "task": i.task, "deliverable": i.deliverable, "kind": i.kind }))
            .collect(),
    )
}

/// The score a rubric-2 answer counts for on-chain (as `Job::read_score`):
/// None if the contract would refuse the answer.
pub fn read_v2(content: &str, deliverable: &str) -> Option<u8> {
    let t = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let v: Value = serde_json::from_str(t).ok()?;
    let score = v["score"]
        .as_u64()
        .filter(|n| *n <= check::MAX_SCORE as u64)? as u8;
    let quotes: Vec<&str> = v["quotes"]
        .as_array()
        .map(|a| a.iter().filter_map(|q| q.as_str()).collect())
        .unwrap_or_default();
    Some(check::graded(score, &quotes, deliverable))
}

/// Grade every item under rubric 2 with `llm`. Returns the report.
pub async fn run(items: &[Item], llm: &Llm) -> Result<Value, String> {
    let model = llm.model();
    let mut rows = Vec::new();
    for i in items {
        let req = check::request_bytes_v(2, &model, &i.job, i.delivery, 0, &i.task, &i.deliverable);
        let r = llm.chat(&req).await?;
        let content = r.content();
        let v2 = read_v2(&content, &i.deliverable);
        let answer: Value = serde_json::from_str(
            content
                .trim()
                .trim_start_matches("```json")
                .trim_matches('`')
                .trim(),
        )
        .unwrap_or(Value::Null);
        let mut row = json!({
            "job": i.job, "delivery": i.delivery, "filler": i.filler(),
            "v1_scores": i.v1, "v1_counted": i.v1_counted(),
            "v2_score": answer["score"], "v2_counted": v2,
            "v2_quotes_found": check::evidence(
                &answer["quotes"].as_array().map(|a| a.iter().filter_map(|q| q.as_str()).collect::<Vec<_>>()).unwrap_or_default(),
                &i.deliverable),
        });
        if let Some(k) = &i.kind {
            row["kind"] = json!(k);
            // Quotes found in the work that are also the task's own words.
            let quotes: Vec<&str> = answer["quotes"]
                .as_array()
                .map(|a| a.iter().filter_map(|q| q.as_str()).collect())
                .unwrap_or_default();
            let found_in_task = quotes
                .iter()
                .filter(|q| {
                    check::evidence(&[**q], &i.deliverable) == 1
                        && check::evidence(&[**q], &i.task) == 1
                })
                .count();
            row["v2_quotes_from_task"] = json!(found_in_task);
        }
        rows.push(row);
    }
    let passed = |rows: &[Value], filler: bool, key: &str| {
        let set: Vec<&Value> = rows.iter().filter(|r| r["filler"] == filler).collect();
        let pass = set
            .iter()
            .filter(|r| r[key].as_u64().is_some_and(|s| s >= PASS_SCORE_HINT as u64))
            .count();
        json!({ "deliveries": set.len(), "passed": pass })
    };
    let mut kinds: Vec<String> = items.iter().filter_map(|i| i.kind.clone()).collect();
    kinds.sort();
    kinds.dedup();
    let by_kind: serde_json::Map<String, Value> = kinds
        .iter()
        .map(|k| {
            let set: Vec<&Value> = rows.iter().filter(|r| r["kind"] == k.as_str()).collect();
            let pass = set
                .iter()
                .filter(|r| {
                    r["v2_counted"]
                        .as_u64()
                        .is_some_and(|s| s >= PASS_SCORE_HINT as u64)
                })
                .count();
            (
                k.clone(),
                json!({ "deliveries": set.len(), "passed_rubric_2": pass }),
            )
        })
        .collect();
    let mut report = json!({
        "model": model,
        "signer": llm.signer(),
        "pass_score": PASS_SCORE_HINT,
        "filler": { "rubric_1": passed(&rows, true, "v1_counted"), "rubric_2": passed(&rows, true, "v2_counted") },
        "other": { "rubric_1": passed(&rows, false, "v1_counted"), "rubric_2": passed(&rows, false, "v2_counted") },
        "rows": rows,
    });
    if !by_kind.is_empty() {
        report["by_kind"] = Value::Object(by_kind);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{mock_script, POLISHED};
    use crate::llm::{Entry, Mock};

    fn v1_cassette() -> Cassette {
        // Two judge answers on filler (fooled: 7 and 6), one on real work.
        let mk = |job: &str, d: &str, score: u8| {
            let req = String::from_utf8(check::request_bytes(
                "m",
                job,
                1,
                0,
                "Explain NEAR global contracts.",
                d,
            ))
            .unwrap();
            let resp = json!({ "choices": [{ "message": { "content": format!("{{\"score\": {score}}}") } }] }).to_string();
            Entry {
                req,
                resp,
                sig: String::new(),
            }
        };
        let good = "Global contracts let many accounts share one deployed code by its hash.";
        Cassette {
            model: "m".into(),
            signer: String::new(),
            signer_source: String::new(),
            entries: vec![
                mk("t1.job-1.hub", POLISHED, 7),
                mk("t1.job-1.hub", POLISHED, 6),
                mk("t2.job-1.hub", good, 8),
            ],
            rubric: None,
        }
    }

    #[test]
    fn deliveries_are_read_from_the_rubric_1_requests_with_their_scores() {
        let items = items_from(&v1_cassette());
        assert_eq!(items.len(), 2);
        assert!(items[0].filler() && !items[1].filler());
        assert_eq!(items[0].v1, vec![7, 6]);
        assert_eq!(
            items[0].v1_counted(),
            Some(6),
            "the lowest answer counted: filler passed"
        );
    }

    #[test]
    fn a_v2_answer_counts_as_the_contract_counts_it() {
        let d = "Global contracts let many accounts share one deployed code by its hash.";
        assert_eq!(
            read_v2(
                r#"{"quotes": ["many accounts share one", "deployed code by its hash"], "score": 9}"#,
                d
            ),
            Some(9)
        );
        assert_eq!(
            read_v2(
                r#"```json
{"quotes": ["made up words here"], "score": 9}
```"#,
                d
            ),
            Some(2)
        );
        assert_eq!(read_v2("9", d), None, "not JSON: the contract refuses it");
    }

    #[test]
    fn every_signed_testnet_answer_counts_as_the_fixed_contract_counts_it() {
        // demo/testnet-judge-cassette.json: the 8 answers the testnet jobs
        // got (rubric 2). The worker delivers the signed NEAR AI response
        // body, so the answer text is JSON-escaped in the deliverable. On
        // testnet t3.job-5's second quote followed a `\n` and was missed:
        // the model's 10 counted 2. Each answer must now count as given.
        let f = format!(
            "{}/../demo/testnet-judge-cassette.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let c: Cassette = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        assert_eq!(c.entries.len(), 8);
        for e in &c.entries {
            let req: Value = serde_json::from_str(&e.req).unwrap();
            let u: Value =
                serde_json::from_str(req["messages"][1]["content"].as_str().unwrap()).unwrap();
            let content = serde_json::from_str::<Value>(&e.resp).unwrap()["choices"][0]["message"]
                ["content"]
                .as_str()
                .unwrap()
                .to_string();
            let given = serde_json::from_str::<Value>(&content).unwrap()["score"]
                .as_u64()
                .unwrap() as u8;
            assert_eq!(
                read_v2(&content, u["deliverable"].as_str().unwrap()),
                Some(given),
                "{}",
                u["job"]
            );
        }
    }

    #[tokio::test]
    async fn a_harder_probe_has_a_real_answer_and_two_fillers_per_task() {
        let dir = std::env::temp_dir().join(format!("ht-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("tasks.json");
        std::fs::write(
            &f,
            r#"[{"job": "t1.p.hub", "task": "Explain NEAR global contracts in 3 sentences."},
                {"job": "t2.p.hub", "task": "Write a one-line summary of: 'Each job is its own account.'"}]"#,
        )
        .unwrap();
        let llm = Llm::Mock(Mock::new("scripted-test-model", Box::new(mock_script)));
        let items = make_probe(f.to_str().unwrap(), &llm).await.unwrap();
        let kinds: Vec<_> = items.iter().map(|i| i.kind.clone().unwrap()).collect();
        assert_eq!(
            kinds[..3],
            [
                "real answer (model)",
                "model-written filler",
                "task-copy filler"
            ]
        );
        assert_eq!(items.len(), 6);
        assert!(items[5].deliverable.contains("Each job is its own account"));
        // The file round-trips, and the report counts quotes taken from the task.
        std::fs::write(dir.join("items.json"), items_json(&items).to_string()).unwrap();
        let back = items_from_file(dir.join("items.json").to_str().unwrap()).unwrap();
        assert_eq!(back.len(), 6);
        assert_eq!(back[2].deliverable, items[2].deliverable);
        let r = run(&back, &llm).await.unwrap();
        assert!(r["rows"][2]["v2_quotes_from_task"].is_u64());
        assert_eq!(r["by_kind"]["task-copy filler"]["deliveries"], 2);
    }

    #[tokio::test]
    async fn a_probe_set_is_reported_by_kind() {
        let f = format!("{}/../runs/filler-probe.json", env!("CARGO_MANIFEST_DIR"));
        let items = items_from_file(&f).unwrap();
        assert_eq!(items.len(), 12);
        let llm = Llm::Mock(Mock::new("scripted-test-model", Box::new(mock_script)));
        let r = run(&items, &llm).await.unwrap();
        assert_eq!(r["by_kind"]["real answer"]["deliveries"], 6);
        assert_eq!(r["by_kind"]["on-topic filler"]["deliveries"], 6);
        // The scripted judge cannot tell on-topic filler from work: that is
        // what the live probe (make record-probe) measures.
        assert_eq!(r["rows"][1]["kind"], "on-topic filler");
    }

    #[tokio::test]
    async fn the_report_compares_both_rubrics_with_the_scripted_judge() {
        let llm = Llm::Mock(Mock::new("scripted-test-model", Box::new(mock_script)));
        let r = run(&items_from(&v1_cassette()), &llm).await.unwrap();
        assert_eq!(
            r["filler"]["rubric_1"],
            json!({ "deliveries": 1, "passed": 1 })
        );
        assert_eq!(
            r["filler"]["rubric_2"],
            json!({ "deliveries": 1, "passed": 0 })
        );
        assert_eq!(
            r["other"]["rubric_2"],
            json!({ "deliveries": 1, "passed": 1 })
        );
    }
}
