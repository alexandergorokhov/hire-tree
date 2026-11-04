//! Hire Tree on a real local NEAR node (near-sandbox).
//!
//! - G4: the job wasm is registered once as a global contract (by code hash);
//!   the root job and every sub-job created by `hire` run it by hash.
//! - G1: a job deletes itself only after its settle transfers succeeded and
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

    // 1. The worker cannot receive tokens: the payee transfer fails, the
    //    rest (5) reaches the parent, and the account is NOT deleted.
    w.ok(&lead, &t1, "settle", json!({})).await;
    assert!(
        w.account_state(&t1).await.is_some(),
        "kept after a failed transfer"
    );
    let j = w.job(&t1).await;
    assert_eq!(j["status"], "Delivered");
    assert_eq!(j["payee_paid"], false);
    assert_eq!(num(&j["ft_balance"]), 30 * USD);
    assert_eq!(num(&j["returned_to_parent"]), 5 * USD);
    assert_eq!(w.ft(&t1).await, 30 * USD);

    // 2. Someone sends 1 unit straight to the job (no ft_transfer_call).
    w.send(&client, &t1, 1, None).await;

    // 3. The worker registers; settle pays it, but the real balance is 1:
    //    still no delete, the stray unit goes on the books.
    w.register(&worker).await;
    w.ok(&lead, &t1, "settle", json!({})).await;
    assert_eq!(w.ft(&worker.id).await, 30 * USD);
    assert!(
        w.account_state(&t1).await.is_some(),
        "kept while real balance is 1"
    );
    let j = w.job(&t1).await;
    assert_eq!(j["payee_paid"], true);
    assert_eq!(num(&j["ft_balance"]), 1);

    // 4. Settle again: the 1 unit goes to the parent; balance 0; deleted.
    let (t1_near, _, _) = w.account_state(&t1).await.unwrap();
    let (root_near_before, _, _) = w.account_state(&root).await.unwrap();
    w.ok(&lead, &t1, "settle", json!({})).await;
    assert!(w.account_state(&t1).await.is_none(), "t1 deleted");
    let (root_near_after, _, _) = w.account_state(&root).await.unwrap();
    let refund = root_near_after.saturating_sub(root_near_before);
    eprintln!(
        "t1 held {} before delete; parent gained {}",
        near_str(t1_near),
        near_str(refund)
    );
    // The parent gets the deleted account's NEAR (less the gas of the
    // child_closed call it ran, which is under 0.01 NEAR).
    assert!(refund + NearToken::from_millinear(10).as_yoctonear() >= t1_near);

    let r = w.job(&root).await;
    assert_eq!(r["children_open"], 0);
    assert_eq!(r["children"][0]["state"], "Closed");
    // 100 funded - 35 hired + 5 returned at the first settle + the stray 1.
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
    assert_eq!(paid, [30 * USD, 20 * USD, 8 * USD, 52 * USD]);
    // Money rule: tree balances (all 0, accounts gone) + paid out == funding + bond.
    for id in [&root, &t1, &t2, &t11] {
        assert_eq!(w.ft(id).await, 0);
    }
    assert_eq!(paid.iter().sum::<u128>(), 100 * USD + 10 * USD);
    assert_eq!(w.ft(&client.id).await, 0);

    let factory_near_1 = w.account_state(&w.factory.id).await.unwrap().0;
    eprintln!(
        "factory: sent 3 NEAR to the root, got back {} on delete",
        near_str(factory_near_1 + NearToken::from_near(3).as_yoctonear() - factory_near_0)
    );
}
