//! Hire Tree on a real local NEAR node (near-sandbox).
//!
//! - G4: the job wasm is registered once as a global contract (by code hash);
//!   the root job and every sub-job created by `hire` run it by hash.
//! - G1: settle registers an unregistered payee and pays it; a job deletes
//!   itself only after its settle transfers succeeded and
//!   the token reports a zero balance; the storage NEAR goes to the parent.
//! - Fold: a 2-level tree (root -> t1 -> t1.t1, root -> t2) settles bottom-up,
//!   every account is deleted, and the money rule holds.
//!
//! Run with `make sandbox` (builds the wasm first). WASM paths: JOB_WASM and
//! USDC_WASM, default out/*.wasm. Needs the near-sandbox binary (downloaded
//! on first build) and nothing else from the network.

use near_api::types::transaction::actions::{
    CreateAccountAction, FunctionCallAction, GlobalContractIdentifier, TransferAction,
    UseGlobalContractAction,
};
use near_api::types::{AccountId, Action, CryptoHash, NearGas, NearToken};
use near_api::{Account, Contract, NetworkConfig, Signer, Transaction};
use near_sandbox::config::{DEFAULT_GENESIS_ACCOUNT, DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const USD: u128 = 1_000_000;
const SPEC: &str = "abababababababababababababababababababababababababababababababab";
const SIGNER: &str = "1111111111111111111111111111111111111111";
/// One yoctoNEAR per byte costs 1e19 yocto: NEAR's storage price.
const YOCTO_PER_BYTE: u128 = 10_000_000_000_000_000_000;

fn wasm(var: &str, default: &str) -> Vec<u8> {
    let path = std::env::var(var)
        .unwrap_or_else(|_| format!("{}/../../out/{default}", env!("CARGO_MANIFEST_DIR")));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e} (run `make build` first)"))
}

fn near_str(yocto: u128) -> String {
    format!("{:.5} NEAR", yocto as f64 / 1e24)
}

#[derive(Clone)]
struct Acc {
    id: AccountId,
    signer: Arc<Signer>,
}

struct World {
    _sbx: near_sandbox::Sandbox,
    net: NetworkConfig,
    genesis: Acc,
    usdc: Acc,
    factory: Acc,
    code_hash: [u8; 32],
}

impl World {
    async fn start() -> World {
        let sbx = near_sandbox::Sandbox::start_sandbox()
            .await
            .expect("start near-sandbox");
        let net = NetworkConfig::from_rpc_url("sandbox", sbx.rpc_addr.parse().unwrap());
        let genesis = Acc {
            id: DEFAULT_GENESIS_ACCOUNT.as_str().parse().unwrap(),
            signer: Signer::from_secret_key(DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse().unwrap())
                .unwrap(),
        };
        let mut w = World {
            _sbx: sbx,
            net,
            genesis: genesis.clone(),
            usdc: genesis.clone(),
            factory: genesis,
            code_hash: [0; 32],
        };
        w.usdc = w.account("usdc", 20).await;
        Contract::deploy(w.usdc.id.clone())
            .use_code(wasm("USDC_WASM", "mock_usdc.wasm"))
            .with_init_call("new", json!({ "owner_id": w.usdc.id }))
            .unwrap()
            .with_signer(w.usdc.signer.clone())
            .send_to(&w.net)
            .await
            .unwrap()
            .assert_success();

        // Stand-in for the factory (built later): registers the global code.
        w.factory = w.account("hiretree", 100).await;
        let code = wasm("JOB_WASM", "hire_tree_job.wasm");
        w.code_hash = Sha256::digest(&code).into();
        Contract::deploy_global_contract_code(code)
            .as_hash()
            .with_signer(w.factory.id.clone(), w.factory.signer.clone())
            .send_to(&w.net)
            .await
            .unwrap()
            .assert_success();
        w
    }

    async fn account(&self, name: &str, near: u128) -> Acc {
        let id: AccountId = format!("{name}.{}", self.genesis.id).parse().unwrap();
        let sk = near_api::signer::generate_secret_key().unwrap();
        Account::create_account(id.clone())
            .fund_myself(self.genesis.id.clone(), NearToken::from_near(near))
            .with_public_key(sk.public_key())
            .with_signer(self.genesis.signer.clone())
            .send_to(&self.net)
            .await
            .unwrap()
            .assert_success();
        Acc {
            id,
            signer: Signer::from_secret_key(sk).unwrap(),
        }
    }

    async fn call(
        &self,
        who: &Acc,
        contract: &AccountId,
        method: &str,
        args: Value,
        deposit: NearToken,
    ) -> Result<Value, String> {
        let r = Contract(contract.clone())
            .call_function(method, args)
            .transaction()
            .gas(NearGas::from_tgas(300))
            .deposit(deposit)
            .with_signer(who.id.clone(), who.signer.clone())
            .send_to(&self.net)
            .await
            .map_err(|e| format!("{e:?}"))?;
        match r.into_result() {
            Ok(ok) => {
                eprintln!(
                    "  {method} on {contract}: ok, {:.1} Tgas burnt",
                    ok.total_gas_burnt.as_gas() as f64 / 1e12
                );
                Ok(ok.json::<Value>().unwrap_or(Value::Null))
            }
            Err(e) => Err(short_error(&format!("{e:?}"))),
        }
    }

    async fn ok(&self, who: &Acc, contract: &AccountId, method: &str, args: Value) -> Value {
        self.call(who, contract, method, args, NearToken::from_yoctonear(0))
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    async fn view(&self, contract: &AccountId, method: &str, args: Value) -> Value {
        Contract(contract.clone())
            .call_function(method, args)
            .read_only::<Value>()
            .fetch_from(&self.net)
            .await
            .unwrap()
            .data
    }

    async fn job(&self, id: &AccountId) -> Value {
        self.view(id, "get_job", json!({})).await
    }

    async fn ft(&self, id: &AccountId) -> u128 {
        let v = self
            .view(&self.usdc.id, "ft_balance_of", json!({ "account_id": id }))
            .await;
        v.as_str().unwrap().parse().unwrap()
    }

    async fn mint(&self, to: &AccountId, amount: u128) {
        self.ok(
            &self.usdc,
            &self.usdc.id,
            "mint",
            json!({ "account_id": to, "amount": amount.to_string() }),
        )
        .await;
    }

    async fn register(&self, who: &Acc) {
        self.call(
            who,
            &self.usdc.id,
            "storage_deposit",
            json!({ "account_id": who.id, "registration_only": true }),
            NearToken::from_millinear(10),
        )
        .await
        .unwrap();
    }

    async fn send(&self, from: &Acc, to: &AccountId, amount: u128, msg: Option<&str>) {
        let (method, args) = match msg {
            Some(m) => (
                "ft_transfer_call",
                json!({ "receiver_id": to, "amount": amount.to_string(), "msg": m }),
            ),
            None => (
                "ft_transfer",
                json!({ "receiver_id": to, "amount": amount.to_string() }),
            ),
        };
        self.call(
            from,
            &self.usdc.id,
            method,
            args,
            NearToken::from_yoctonear(1),
        )
        .await
        .unwrap_or_else(|e| panic!("{method}: {e}"));
    }

    /// (balance, storage bytes, uses the global job code), or None if deleted.
    async fn account_state(&self, id: &AccountId) -> Option<(u128, u64, bool)> {
        let a = Account(id.clone())
            .view()
            .fetch_from(&self.net)
            .await
            .ok()?
            .data;
        let global = matches!(
            a.contract_state,
            near_api::types::account::ContractState::GlobalHash(h) if h.0 == self.code_hash
        );
        Some((a.amount.as_yoctonear(), a.storage_usage, global))
    }

    /// The factory's job, done by hand until the factory exists: one batch
    /// that creates the root account, links the global code and inits it.
    async fn create_root(&self, n: u32, client: &Acc, lead: &Acc, near: u128) -> AccountId {
        let root: AccountId = format!("job-{n}.{}", self.factory.id).parse().unwrap();
        let init = json!({
            "client": client.id, "manager": lead.id, "worker": null,
            "token": self.usdc.id, "amount": "0", "spec_hash": SPEC,
            "window_secs": 72 * 3600, "signer": SIGNER,
            "code_hash": near_api::types::CryptoHash(self.code_hash).to_string(),
        });
        Transaction::construct(self.factory.id.clone(), root.clone())
            .add_action(Action::CreateAccount(CreateAccountAction {}))
            .add_action(Action::Transfer(TransferAction {
                deposit: NearToken::from_near(near),
            }))
            .add_action(Action::UseGlobalContract(Box::new(
                UseGlobalContractAction {
                    contract_identifier: GlobalContractIdentifier::CodeHash(CryptoHash(
                        self.code_hash,
                    )),
                },
            )))
            .add_action(Action::FunctionCall(Box::new(FunctionCallAction {
                method_name: "new".into(),
                args: init.to_string().into_bytes(),
                gas: NearGas::from_tgas(30),
                deposit: NearToken::from_yoctonear(0),
            })))
            .with_signer(self.factory.signer.clone())
            .send_to(&self.net)
            .await
            .unwrap()
            .assert_success();
        // The factory will also register the root on the token.
        let factory = self.factory.clone();
        self.call(
            &factory,
            &self.usdc.id,
            "storage_deposit",
            json!({ "account_id": root, "registration_only": true }),
            NearToken::from_millinear(10),
        )
        .await
        .unwrap();
        root
    }

    async fn hire(
        &self,
        lead: &Acc,
        job: &AccountId,
        worker: &AccountId,
        amount: u128,
        sub: u128,
        near: NearToken,
    ) -> Result<Value, String> {
        // The manager pays the child account's storage and its token
        // registration (0.035 NEAR minimum).
        let near = near.saturating_add(NearToken::from_millinear(35));
        self.call(
            lead,
            job,
            "hire",
            json!({ "worker": worker, "amount": amount.to_string(), "spec_hash": SPEC,
                    "sub": sub.to_string() }),
            near,
        )
        .await
    }
}

/// The first contract panic message in a failure dump, or the dump itself.
fn short_error(dump: &str) -> String {
    match dump.find("ExecutionError(\"") {
        Some(i) => {
            let rest = &dump[i + 16..];
            rest[..rest.find('"').unwrap_or(rest.len())].to_string()
        }
        None => dump.chars().take(600).collect(),
    }
}

fn child(parent: &AccountId, n: u32) -> AccountId {
    format!("t{n}.{parent}").parse().unwrap()
}

fn num(v: &Value) -> u128 {
    v.as_str().unwrap().parse().unwrap()
}

/// G4: global contract by hash, root and sub-job accounts run it.
#[tokio::test]
async fn g4_global_contract_and_hire_create_sub_accounts() {
    let w = World::start().await;
    let client = w.account("client", 10).await;
    let lead = w.account("lead", 10).await;
    let worker = w.account("worker", 10).await;
    w.mint(&client.id, 100 * USD).await;

    let root = w.create_root(1, &client, &lead, 5).await;
    let (_, root_bytes, root_global) = w.account_state(&root).await.unwrap();
    assert!(root_global, "root runs the global job code by hash");
    eprintln!("root job account: {root_bytes} bytes of storage");

    w.send(&client, &root, 100 * USD, Some("fund")).await;
    assert_eq!(w.ft(&root).await, 100 * USD);

    // A stranger cannot hire.
    let err = w
        .hire(
            &worker,
            &root,
            &worker.id,
            USD,
            0,
            NearToken::from_yoctonear(0),
        )
        .await
        .unwrap_err();
    assert!(err.contains("only the manager can hire"), "{err}");

    w.hire(
        &lead,
        &root,
        &worker.id,
        30 * USD,
        0,
        NearToken::from_yoctonear(0),
    )
    .await
    .unwrap();
    let t1 = child(&root, 1);
    let (t1_near, t1_bytes, t1_global) = w.account_state(&t1).await.expect("t1 exists");
    assert!(t1_global, "sub-job runs the global job code by hash");
    eprintln!(
        "t1 sub-job: {t1_bytes} bytes (needs {}), holds {}",
        near_str(t1_bytes as u128 * YOCTO_PER_BYTE),
        near_str(t1_near)
    );
    assert!(t1_bytes < 10_000, "no full wasm copy per account");

    let j = w.job(&t1).await;
    assert_eq!(j["parent"], root.as_str());
    assert_eq!(j["worker"], worker.id.as_str());
    assert_eq!(j["status"], "Open");
    assert_eq!(num(&j["funded"]), 30 * USD);
    assert_eq!(w.ft(&t1).await, 30 * USD);
    let r = w.job(&root).await;
    assert_eq!(r["children_open"], 1);
    assert_eq!(r["children_pending"], 0);
    assert_eq!(num(&r["ft_balance"]), 70 * USD);
    assert_eq!(w.ft(&root).await, 70 * USD);
    let bounds = w
        .view(&w.usdc.id, "storage_balance_bounds", json!({}))
        .await;
    eprintln!("token storage_balance_bounds: {bounds}");
}

/// G1: delete only after confirmed transfers and a zero real balance.
#[tokio::test]
async fn g1_delete_only_after_confirmed_transfer_and_zero_balance() {
    let w = World::start().await;
    let client = w.account("client", 10).await;
    let lead = w.account("lead", 10).await;
    let worker = w.account("worker", 10).await; // not registered on the token yet
    w.mint(&client.id, 100 * USD + 1).await;
    let root = w.create_root(1, &client, &lead, 5).await;
    w.send(&client, &root, 100 * USD, Some("fund")).await;
    w.hire(
        &lead,
        &root,
        &worker.id,
        30 * USD,
        5 * USD,
        NearToken::from_yoctonear(0),
    )
    .await
    .unwrap();
    let t1 = child(&root, 1);
    w.ok(
        &worker,
        &t1,
        "deliver",
        json!({ "resp_hash": "cd".repeat(32) }),
    )
    .await;
    w.ok(&lead, &t1, "approve", json!({})).await;

    // 1. Someone sends 1 unit straight to the job (no ft_transfer_call).
    w.send(&client, &t1, 1, None).await;

    // 2. Settle: the job registers the (unregistered) worker on the token and
    //    pays it 30, the rest (5) reaches the parent. The balance check then
    //    sees 1 unit the ledger never had: it is not booked (it could be a
    //    refunded ft_transfer_call in flight) but swept to the parent; only
    //    after a second check reads 0 is the account deleted.
    let (root_near_before, _, _) = w.account_state(&root).await.unwrap();
    w.ok(&lead, &t1, "settle", json!({})).await;
    assert_eq!(w.ft(&worker.id).await, 30 * USD);
    assert!(w.account_state(&t1).await.is_none(), "t1 deleted");
    let (root_near_after, _, _) = w.account_state(&root).await.unwrap();
    let refund = root_near_after.saturating_sub(root_near_before);
    eprintln!("parent gained {} of storage NEAR", near_str(refund));
    // The child's storage NEAR came back (it started with 0.03).
    assert!(refund >= NearToken::from_millinear(20).as_yoctonear());

    let r = w.job(&root).await;
    assert_eq!(r["children_open"], 0);
    assert_eq!(r["children"][0]["state"], "Closed");
    assert_eq!(w.ft(&worker.id).await, 30 * USD, "worker paid once");
    // 100 funded - 35 hired + 5 returned + the swept stray 1.
    assert_eq!(num(&r["ft_balance"]), 65 * USD + 5 * USD + 1);
    assert_eq!(w.ft(&root).await, num(&r["ft_balance"]));
}

/// The 2-level tree folds bottom-up; every account is deleted.
#[tokio::test]
async fn two_level_tree_folds_bottom_up_and_money_rule_holds() {
    let w = World::start().await;
    let client = w.account("client", 10).await;
    let lead = w.account("lead", 10).await;
    let w1 = w.account("w1", 5).await;
    let w2 = w.account("w2", 5).await;
    let w3 = w.account("w3", 5).await;
    for a in [&lead, &w1, &w2, &w3] {
        w.register(a).await;
    }
    w.mint(&client.id, 100 * USD).await;
    w.mint(&lead.id, 10 * USD).await;
    let factory_near_0 = w.account_state(&w.factory.id).await.unwrap().0;

    let root = w.create_root(1, &client, &lead, 3).await;
    w.send(&client, &root, 100 * USD, Some("fund")).await;
    w.send(&lead, &root, 10 * USD, Some("bond")).await;

    // root -> t1 (w1: 30 + 10 sub-budget, 0.5 NEAR extra so it can hire)
    // root -> t2 (w2: 20); t1 -> t1.t1 (w3: 8)
    w.hire(
        &lead,
        &root,
        &w1.id,
        30 * USD,
        10 * USD,
        NearToken::from_millinear(500),
    )
    .await
    .unwrap();
    w.hire(
        &lead,
        &root,
        &w2.id,
        20 * USD,
        0,
        NearToken::from_yoctonear(0),
    )
    .await
    .unwrap();
    let (t1, t2) = (child(&root, 1), child(&root, 2));
    w.hire(&lead, &t1, &w3.id, 8 * USD, 0, NearToken::from_yoctonear(0))
        .await
        .unwrap();
    let t11 = child(&t1, 1);
    for (id, name) in [(&root, "root"), (&t1, "t1"), (&t2, "t2"), (&t11, "t1.t1")] {
        let (near, bytes, global) = w.account_state(id).await.unwrap();
        assert!(global);
        eprintln!(
            "{name}: {bytes} bytes (locks {}), holds {}",
            near_str(bytes as u128 * YOCTO_PER_BYTE),
            near_str(near)
        );
    }
    assert_eq!(w.ft(&t11).await, 8 * USD);
    assert_eq!(w.ft(&t1).await, 32 * USD);

    w.ok(
        &w3,
        &t11,
        "deliver",
        json!({ "resp_hash": "01".repeat(32) }),
    )
    .await;
    w.ok(&w1, &t1, "deliver", json!({ "resp_hash": "02".repeat(32) }))
        .await;
    w.ok(&w2, &t2, "deliver", json!({ "resp_hash": "03".repeat(32) }))
        .await;
    for id in [&t11, &t1, &t2] {
        w.ok(&lead, id, "approve", json!({})).await;
    }
    w.ok(&client, &root, "approve", json!({})).await;

    // Settling a parent before its children is refused.
    let err = w
        .call(
            &lead,
            &root,
            "settle",
            json!({}),
            NearToken::from_yoctonear(0),
        )
        .await
        .unwrap_err();
    assert!(err.contains("children still open"), "{err}");

    for id in [&t11, &t1, &t2, &root] {
        w.ok(&lead, id, "settle", json!({})).await;
        assert!(w.account_state(id).await.is_none(), "{id} deleted");
    }

    let paid = [
        w.ft(&w1.id).await,
        w.ft(&w2.id).await,
        w.ft(&w3.id).await,
        w.ft(&lead.id).await,
    ];
    eprintln!(
        "paid out: w1 {} w2 {} w3 {} lead {}",
        paid[0], paid[1], paid[2], paid[3]
    );
    // The lead gets its bond back (no fee on this root); the client gets
    // the unspent 42 back.
    assert_eq!(paid, [30 * USD, 20 * USD, 8 * USD, 10 * USD]);
    assert_eq!(w.ft(&client.id).await, 42 * USD);
    // Money rule: tree balances (all 0, accounts gone) + paid out == funding + bond.
    for id in [&root, &t1, &t2, &t11] {
        assert_eq!(w.ft(id).await, 0);
    }
    assert_eq!(paid.iter().sum::<u128>() + 42 * USD, 100 * USD + 10 * USD);

    let factory_near_1 = w.account_state(&w.factory.id).await.unwrap().0;
    eprintln!(
        "factory: sent 3 NEAR to the root, got back {} on delete",
        near_str(factory_near_1 + NearToken::from_near(3).as_yoctonear() - factory_near_0)
    );
}

// ---------------------------------------------------------------- full scene

mod scene_util {
    use ht_core::receipt::{eip191_digest, eth_address, sha256, sha256_hex};
    use secp256k1::{Message, Secp256k1, SecretKey};

    pub const MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash-1017";

    /// Stand-in for the NEAR AI Cloud TEE key (sandbox only).
    pub fn key() -> SecretKey {
        SecretKey::from_slice(&[0x42; 32]).unwrap()
    }

    pub fn addr_hex(k: &SecretKey) -> String {
        let pk = k.public_key(&Secp256k1::new()).serialize_uncompressed();
        hex::encode(eth_address(&pk[1..].try_into().unwrap()))
    }

    /// (resp_hash, req base64, sig hex, model) for a request bound to `task`.
    pub fn receipt(task: &str, spec: &str, seq: u32, answer: &str) -> serde_json::Value {
        let req = format!(
            "{{\"model\":\"{MODEL}\",\"messages\":[{{\"role\":\"user\",\"content\":\"HT1:{task}:{spec}:{seq} Research: NEAR global contracts\"}}]}}"
        );
        let resp = format!("{{\"choices\":[{{\"message\":{{\"content\":\"{answer}\"}}}}]}}");
        let text = format!(
            "{MODEL}:{}:{}",
            sha256_hex(req.as_bytes()),
            sha256_hex(resp.as_bytes())
        );
        let (rid, rs) = Secp256k1::new()
            .sign_ecdsa_recoverable(
                &Message::from_slice(&eip191_digest(text.as_bytes())).unwrap(),
                &key(),
            )
            .serialize_compact();
        let mut sig = rs.to_vec();
        sig.push(27 + rid.to_i32() as u8);
        use near_api::types::json::Base64VecU8;
        serde_json::json!({
            "resp_hash": hex::encode(sha256(resp.as_bytes())),
            "req": Base64VecU8::from(req.into_bytes()),
            "sig": hex::encode(sig),
            "model": MODEL,
            "resp": Base64VecU8::from(resp.into_bytes()),
        })
    }

    /// A delivery with no receipt: the response bytes and their sha256.
    pub fn plain(text: &str) -> serde_json::Value {
        use near_api::types::json::Base64VecU8;
        serde_json::json!({
            "resp_hash": hex::encode(sha256(text.as_bytes())),
            "resp": Base64VecU8::from(text.as_bytes().to_vec()),
        })
    }
}

/// The demo story: each step prints a narrated line and records a snapshot
/// of the tree (for `make demo` and the web page, `out/scene.json`).
struct Story {
    steps: Vec<Value>,
}

impl Story {
    async fn step(
        &mut self,
        w: &World,
        title: &str,
        say: &str,
        jobs: &[(&str, &AccountId)],
        people: &[(&str, &AccountId)],
    ) {
        let n = self.steps.len() + 1;
        println!("\nSCENE {n}: {title}\n  {say}");
        let mut tree = vec![];
        for (label, id) in jobs {
            let alive = w.account_state(id).await;
            let v = if alive.is_some() {
                let j = w.job(id).await;
                json!({ "label": label, "account": id, "alive": true,
                        "parent": j["parent"], "status": j["status"], "worker": j["worker"],
                        "payee": j["payee"], "amount": j["amount"], "ft": w.ft(id).await.to_string(),
                        "verified": j["verified"], "quality": j["quality"], "note_sold": j["note_sold"],
                        "storage_near": near_str(alive.map(|a| a.0).unwrap_or(0)) })
            } else {
                json!({ "label": label, "account": id, "alive": false })
            };
            tree.push(v);
        }
        let mut wallets = vec![];
        for (label, id) in people {
            let bal = w.ft(id).await;
            println!("    {label:<11} {:>10.2} fUSD", bal as f64 / 1e6);
            wallets.push(json!({ "label": label, "account": id, "ft": bal.to_string() }));
        }
        self.steps
            .push(json!({ "n": n, "title": title, "say": say, "jobs": tree, "wallets": wallets }));
    }

    fn save(&self) {
        let path = std::env::var("SCENE_OUT")
            .unwrap_or_else(|_| format!("{}/../../out/scene.json", env!("CARGO_MANIFEST_DIR")));
        let doc = json!({ "schema": "hiretree-scene.v1", "network": "near-sandbox (local node)", "steps": self.steps });
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        println!("\nscene written to {path}");
    }
}

/// The demo story on a real local node: factory posts the job; the lead
/// hires three workers; a cheater's copied receipt is rejected; a verified
/// and checked worker sells its note to the lead's treasury and is paid at
/// once; a junk delivery is disputed and refunded; everything settles
/// bottom-up, every job account is deleted, and the money rule holds.
#[tokio::test]
async fn scene_full_tree_on_local_node() {
    use scene_util::*;
    let w = World::start().await;
    let client = w.account("client", 10).await;
    let lead = w.account("lead", 10).await;
    let treasury = w.account("treasury", 10).await;
    let researcher = w.account("researcher", 5).await;
    let translator = w.account("translator", 5).await;
    let junk = w.account("junk", 5).await;

    // Checker and factory.
    let checker = w.account("outlayer", 10).await;
    Contract::deploy(checker.id.clone())
        .use_code(wasm("OUTLAYER_WASM", "mock_outlayer.wasm"))
        .with_init_call("new", json!({ "owner": checker.id, "default_score": 8 }))
        .unwrap()
        .with_signer(checker.signer.clone())
        .send_to(&w.net)
        .await
        .unwrap()
        .assert_success();
    let hub = w.account("hub", 30).await;
    Contract::deploy(hub.id.clone())
        .use_code(wasm("FACTORY_WASM", "hire_tree_factory.wasm"))
        .with_init_call(
            "new",
            json!({ "owner": hub.id, "token": w.usdc.id,
                    "code_hash": near_api::types::CryptoHash(w.code_hash).to_string(),
                    "signer": addr_hex(&key()), "checker": checker.id }),
        )
        .unwrap()
        .with_signer(hub.signer.clone())
        .send_to(&w.net)
        .await
        .unwrap()
        .assert_success();
    for a in [&client, &lead, &treasury, &hub] {
        w.register(a).await;
    }
    w.mint(&client.id, 100 * USD).await;
    w.mint(&treasury.id, 50 * USD).await;
    // The client prepays the posting NEAR (root storage + registration).
    w.call(
        &client,
        &hub.id,
        "prepay_post",
        json!({}),
        NearToken::from_millinear(60),
    )
    .await
    .unwrap();
    let (hub_near_start, _, _) = w.account_state(&hub.id).await.unwrap();
    let mut story = Story { steps: vec![] };

    // 1. The client posts and funds the job in one transfer.
    let post = json!({ "manager": lead.id, "spec_hash": SPEC, "window_secs": 3600,
                       "treasury": treasury.id, "lead_fee": (10 * USD).to_string() })
    .to_string();
    w.send(&client, &hub.id, 100 * USD, Some(&post)).await;
    let root: AccountId = format!("job-1.{}", hub.id).parse().unwrap();
    assert_eq!(w.ft(&root).await, 100 * USD);
    assert_eq!(w.ft(&client.id).await, 0);
    assert!(
        w.account_state(&root).await.unwrap().2,
        "root runs the global code"
    );
    let people = [
        ("client", &client.id),
        ("lead", &lead.id),
        ("treasury", &treasury.id),
        ("researcher", &researcher.id),
        ("translator", &translator.id),
        ("junk", &junk.id),
    ];
    story.step(&w, "The client posts a job",
        "One ft_transfer_call to the factory creates job-1 from the global contract and funds it with 100 fUSD. Nobody holds the money: the job account does.",
        &[("job-1", &root)], &people).await;

    // 2. The lead hires three workers.
    for (wk, pay) in [(&researcher, 30), (&translator, 20), (&junk, 10)] {
        w.hire(
            &lead,
            &root,
            &wk.id,
            pay * USD,
            0,
            NearToken::from_yoctonear(0),
        )
        .await
        .unwrap();
    }
    let (t1, t2, t3) = (child(&root, 1), child(&root, 2), child(&root, 3));
    let jobs = [
        ("job-1", &root),
        ("t1 researcher", &t1),
        ("t2 translator", &t2),
        ("t3 junk", &t3),
    ];
    story.step(&w, "The lead agent hires three workers",
        "Each hire creates a sub-account (t1, t2, t3) running the same code by hash and moves that job's pay into it: 30, 20 and 10 fUSD.",
        &jobs, &people).await;

    // 3. Cheat: the translator submits a genuine receipt made for t1's task.
    let stolen = receipt(
        t1.as_str(),
        SPEC,
        1,
        "NEAR global contracts are deployed once by hash",
    );
    let err = w
        .call(
            &translator,
            &t2,
            "deliver",
            stolen.clone(),
            NearToken::from_yoctonear(0),
        )
        .await
        .unwrap_err();
    assert!(err.contains("is not this task"), "{err}");
    story.step(&w, "A cheater copies a real AI receipt",
        &format!("The translator submits the researcher's genuine NEAR AI receipt on its own job. The contract refuses: {err}"),
        &jobs, &people).await;

    // 4. The researcher delivers with its own receipt: verified, checked (8).
    w.ok(&researcher, &t1, "deliver", stolen).await;
    let j1 = w.job(&t1).await;
    assert_eq!(j1["status"], "Checked");
    assert_eq!(j1["verified"], true);
    assert_eq!(j1["quality"], 8);
    story.step(&w, "The researcher proves its work came from NEAR AI",
        "The contract recomputes sha256(request), checks the HT1 tag binds it to t1 and delivery 1, and ecrecovers the NEAR AI Cloud signature. Then it starts one quality check itself: 8/10.",
        &jobs, &people).await;

    // 5. The lead's treasury buys the note: the researcher is paid now.
    let quote = num(&w.view(&t1, "quote_note", json!({})).await);
    assert_eq!(quote, 29_700_000);
    w.send(&treasury, &t1, quote, Some("buy_note")).await;
    assert_eq!(w.ft(&researcher.id).await, quote);
    assert_eq!(w.job(&t1).await["payee"], treasury.id.to_string());
    story.step(&w, "Paid now, not in 72 hours",
        "The lead's treasury buys the researcher's Job Note for 29.70 (1% off). The researcher is paid in the same transaction; the treasury now collects the 30 at settle.",
        &jobs, &people).await;

    // 6. The translator delivers without a receipt: paid at settle only.
    w.ok(
        &translator,
        &t2,
        "deliver",
        plain("Los contratos globales de NEAR se despliegan una vez."),
    )
    .await;
    assert_eq!(w.job(&t2).await["verified"], false);
    assert!(w.view(&t2, "quote_note", json!({})).await.is_null());
    story.step(&w, "No receipt, no early pay",
        "The translator delivers without a NEAR AI receipt. It will be paid at settle, but its note is not sellable.",
        &jobs, &people).await;

    // 7. Junk passes the first check (7) but the dispute panels say 1 and 2.
    let junk_text = "lorem ipsum lorem ipsum";
    let junk_delivery = plain(junk_text);
    let junk_hash = junk_delivery["resp_hash"].as_str().unwrap().to_string();
    w.ok(
        &checker,
        &checker.id,
        "set_scores",
        json!({ "deliverable_hash": junk_hash, "scores": [7, 1, 2] }),
    )
    .await;
    w.ok(&junk, &t3, "deliver", junk_delivery.clone()).await;
    assert_eq!(w.job(&t3).await["status"], "Checked");
    // The lead sends the delivered bytes so the panels can read them.
    w.ok(
        &lead,
        &t3,
        "dispute",
        json!({ "resp": junk_delivery["resp"] }),
    )
    .await;
    assert_eq!(w.job(&t3).await["status"], "Refunded");
    story.step(&w, "Junk is disputed",
        "The junk passed one check (7/10). The lead disputes: two more checks with fresh seeds say 1 and 2. Two of three votes say junk, so t3's pay goes back up the tree.",
        &jobs, &people).await;

    // 8. Settle bottom-up: the lead approves the sub-jobs, the client the root.
    for t in [&t1, &t2, &t3] {
        w.ok(&lead, t, "approve", json!({})).await;
        w.ok(&lead, t, "settle", json!({})).await;
        assert!(w.account_state(t).await.is_none(), "{t} deleted");
    }
    w.ok(&client, &root, "approve", json!({})).await;
    w.ok(&lead, &root, "settle", json!({})).await;
    assert!(w.account_state(&root).await.is_none(), "root deleted");
    story.step(&w, "The tree folds",
        "Settle runs bottom-up: each account pays its note holder, returns the rest to its parent, proves its token balance is zero, and deletes itself. Storage NEAR flows back up.",
        &jobs, &people).await;

    // 9. Money rule: 100 in, 100 out.
    let cl = w.ft(&client.id).await;
    let (r, tr, tl, jk, ld) = (
        w.ft(&researcher.id).await,
        w.ft(&treasury.id).await,
        w.ft(&translator.id).await,
        w.ft(&junk.id).await,
        w.ft(&lead.id).await,
    );
    eprintln!("researcher {r}, treasury {tr}, translator {tl}, junk {jk}, lead {ld}");
    assert_eq!(r, 29_700_000);
    assert_eq!(tr, 50 * USD + 300_000, "treasury earned the 1% fee");
    assert_eq!(tl, 20 * USD);
    assert_eq!(jk, 0);
    assert_eq!(ld, 10 * USD, "the lead's fee, set by the client at post");
    assert_eq!(
        cl,
        40 * USD,
        "client gets back the unspent 30 + refunded 10"
    );
    assert_eq!(r + (tr - 50 * USD) + tl + jk + ld + cl, 100 * USD);
    story.step(&w, "100 in, 100 out",
        "researcher 29.70 + treasury fee 0.30 + translator 20 + junk 0 + lead fee 10 + back to the client 40 (unspent 30 + refunded 10) = 100 fUSD. No account is left holding anything.",
        &jobs, &people).await;
    // Track records the jobs reported to the factory as they deleted.
    let rec = w
        .view(
            &hub.id,
            "get_record",
            json!({ "account_id": researcher.id }),
        )
        .await;
    assert_eq!(
        (
            rec["jobs"].as_u64(),
            rec["checked"].as_u64(),
            rec["paid"].as_u64()
        ),
        (Some(1), Some(1), Some(1)),
        "{rec}"
    );
    let rec = w
        .view(&hub.id, "get_record", json!({ "account_id": junk.id }))
        .await;
    assert_eq!(
        (rec["junk"].as_u64(), rec["paid"].as_u64()),
        (Some(1), Some(0)),
        "{rec}"
    );
    story.save();
    let (hub_near_end, _, _) = w.account_state(&hub.id).await.unwrap();
    eprintln!(
        "hub NEAR: {} -> {}",
        near_str(hub_near_start),
        near_str(hub_near_end)
    );
}
