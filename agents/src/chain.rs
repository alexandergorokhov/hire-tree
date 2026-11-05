//! A NEAR network the agents act on: a local near-sandbox node (deployed
//! from `out/*.wasm`) or testnet (accounts and keys from the environment).

use near_api::types::{AccountId, NearGas, NearToken};
use near_api::{Account, Contract, NetworkConfig, Signer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const USD: u128 = 1_000_000;

/// An account the agents can sign for.
#[derive(Clone)]
pub struct Acc {
    pub id: AccountId,
    pub signer: Arc<Signer>,
}

pub struct Chain {
    pub sbx: Option<near_sandbox::Sandbox>,
    pub net: NetworkConfig,
    /// Funds new accounts (sandbox genesis; on testnet the operator).
    pub funder: Acc,
    pub usdc: AccountId,
    /// May mint fUSD (sandbox: the mock-usdc account; testnet: its owner).
    pub usdc_owner: Acc,
    /// The factory: posts jobs, keeps track records.
    pub hub: AccountId,
    pub network_name: String,
    pub explorer: Option<String>,
    /// Every transaction sent: {who, method, on, tx}.
    pub txs: std::sync::Mutex<Vec<Value>>,
}

pub fn out_dir() -> String {
    std::env::var("OUT_DIR_HT").unwrap_or_else(|_| format!("{}/../out", env!("CARGO_MANIFEST_DIR")))
}

fn wasm(var: &str, default: &str) -> Vec<u8> {
    let path = std::env::var(var).unwrap_or_else(|_| format!("{}/{default}", out_dir()));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e} (run `make build` first)"))
}

pub fn short_error(dump: &str) -> String {
    match dump.find("ExecutionError(\"") {
        Some(i) => {
            let rest = &dump[i + 16..];
            rest[..rest.find('"').unwrap_or(rest.len())].to_string()
        }
        None => dump.chars().take(600).collect(),
    }
}

pub fn num(v: &Value) -> u128 {
    v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0)
}

impl Chain {
    /// Start a local node and deploy mock-usdc, the global job code and the
    /// factory (pinning `signer` and, if given, the judge model).
    pub async fn sandbox(signer: &str, judge: Option<&str>) -> Chain {
        let sbx = near_sandbox::Sandbox::start_sandbox()
            .await
            .expect("start near-sandbox");
        let net = NetworkConfig::from_rpc_url("sandbox", sbx.rpc_addr.parse().unwrap());
        use near_sandbox::config::{DEFAULT_GENESIS_ACCOUNT, DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY};
        let funder = Acc {
            id: DEFAULT_GENESIS_ACCOUNT.as_str().parse().unwrap(),
            signer: Signer::from_secret_key(DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse().unwrap())
                .unwrap(),
        };
        let mut c = Chain {
            sbx: Some(sbx),
            net,
            funder: funder.clone(),
            usdc: funder.id.clone(),
            usdc_owner: funder.clone(),
            hub: funder.id.clone(),
            network_name: "near-sandbox (local node)".into(),
            explorer: None,
            txs: Default::default(),
        };
        let usdc = c.account("usdc", 20).await;
        Contract::deploy(usdc.id.clone())
            .use_code(wasm("USDC_WASM", "mock_usdc.wasm"))
            .with_init_call("new", json!({ "owner_id": usdc.id }))
            .unwrap()
            .with_signer(usdc.signer.clone())
            .send_to(&c.net)
            .await
            .unwrap()
            .assert_success();
        c.usdc = usdc.id.clone();
        c.usdc_owner = usdc.clone();
        let hub = c.account("hub", 100).await;
        let code = wasm("JOB_WASM", "hire_tree_job.wasm");
        let code_hash: [u8; 32] = Sha256::digest(&code).into();
        Contract::deploy_global_contract_code(code)
            .as_hash()
            .with_signer(hub.id.clone(), hub.signer.clone())
            .send_to(&c.net)
            .await
            .unwrap()
            .assert_success();
        Contract::deploy(hub.id.clone())
            .use_code(wasm("FACTORY_WASM", "hire_tree_factory.wasm"))
            .with_init_call(
                "new",
                json!({ "owner": hub.id, "token": usdc.id,
                        "code_hash": near_api::types::CryptoHash(code_hash).to_string(),
                        "signer": signer, "checker": null }),
            )
            .unwrap()
            .with_signer(hub.signer.clone())
            .send_to(&c.net)
            .await
            .unwrap()
            .assert_success();
        if let Some(m) = judge {
            c.ok(&hub, &hub.id, "set_settings", json!({ "judge": m }))
                .await;
        }
        c.hub = hub.id.clone();
        c.register(&hub).await;
        c
    }

    /// A new funded sub-account of the funder.
    pub async fn account(&self, name: &str, near: u128) -> Acc {
        let id: AccountId = format!("{name}.{}", self.funder.id).parse().unwrap();
        let sk = near_api::signer::generate_secret_key().unwrap();
        Account::create_account(id.clone())
            .fund_myself(self.funder.id.clone(), NearToken::from_near(near))
            .with_public_key(sk.public_key())
            .with_signer(self.funder.signer.clone())
            .send_to(&self.net)
            .await
            .unwrap()
            .assert_success();
        Acc {
            id,
            signer: Signer::from_secret_key(sk).unwrap(),
        }
    }

    pub async fn call(
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
        let full = r.into_full().ok_or("transaction still pending")?;
        let tx = full.outcome().transaction_hash.to_string();
        self.txs
            .lock()
            .unwrap()
            .push(json!({ "who": who.id, "method": method, "on": contract, "tx": tx }));
        match full.into_result() {
            Ok(ok) => {
                eprintln!("  {method} on {contract}: ok (tx {tx})");
                Ok(ok.json::<Value>().unwrap_or(Value::Null))
            }
            Err(e) => Err(short_error(&format!("{e:?}"))),
        }
    }

    pub async fn ok(&self, who: &Acc, contract: &AccountId, method: &str, args: Value) -> Value {
        self.call(who, contract, method, args, NearToken::from_yoctonear(0))
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    pub async fn view(
        &self,
        contract: &AccountId,
        method: &str,
        args: Value,
    ) -> Result<Value, String> {
        Contract(contract.clone())
            .call_function(method, args)
            .read_only::<Value>()
            .fetch_from(&self.net)
            .await
            .map(|r| r.data)
            .map_err(|e| format!("{e:?}"))
    }

    pub async fn alive(&self, id: &AccountId) -> bool {
        Account(id.clone())
            .view()
            .fetch_from(&self.net)
            .await
            .is_ok()
    }

    pub async fn ft(&self, id: &AccountId) -> u128 {
        self.view(&self.usdc, "ft_balance_of", json!({ "account_id": id }))
            .await
            .map(|v| num(&v))
            .unwrap_or(0)
    }

    pub async fn mint(&self, to: &AccountId, amount: u128) {
        self.ok(
            &self.usdc_owner,
            &self.usdc,
            "mint",
            json!({ "account_id": to, "amount": amount.to_string() }),
        )
        .await;
    }

    pub async fn register(&self, who: &Acc) {
        self.call(
            who,
            &self.usdc,
            "storage_deposit",
            json!({ "account_id": who.id, "registration_only": true }),
            NearToken::from_millinear(10),
        )
        .await
        .unwrap();
    }

    /// `ft_transfer_call` (with `msg`) or `ft_transfer`.
    pub async fn send(
        &self,
        from: &Acc,
        to: &AccountId,
        amount: u128,
        msg: Option<&str>,
    ) -> Result<Value, String> {
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
        self.call(from, &self.usdc, method, args, NearToken::from_yoctonear(1))
            .await
    }

    /// Sandbox only: move the clock (one block is about one second).
    /// Let `secs` of chain time pass (sandbox: skip blocks; testnet: wait).
    pub async fn fast_forward(&self, secs: u64) {
        match &self.sbx {
            Some(s) => {
                let target = self.block_time_ns().await + (secs + 5) * 1_000_000_000;
                // Skipped blocks advance time by less than a second each, so
                // skip until block time is really past the target.
                for _ in 0..50 {
                    if self.block_time_ns().await >= target {
                        return;
                    }
                    s.fast_forward(secs + 5).await.expect("fast_forward");
                }
                panic!("fast_forward: block time did not advance");
            }
            None => tokio::time::sleep(std::time::Duration::from_secs(secs + 5)).await,
        }
    }

    pub async fn block_time_ns(&self) -> u64 {
        near_api::Chain::block()
            .fetch_from(&self.net)
            .await
            .expect("block")
            .header
            .timestamp
    }

    /// Wait until block time is past `end_ns` (sandbox: skip `secs_hint`).
    pub async fn wait_past(&self, end_ns: u64, secs_hint: u64) {
        if self.sbx.is_some() {
            return self.fast_forward(secs_hint).await;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        if end_ns > now {
            let secs = (end_ns - now) / 1_000_000_000;
            eprintln!("  waiting {secs}s for the job window to close");
            self.fast_forward(secs + 10).await;
        }
    }
}
