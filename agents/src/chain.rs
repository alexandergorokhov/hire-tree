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
    /// Derives demo account keys (sha256 of the operator key).
    seed: [u8; 32],
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

/// Rate limits, timeouts, dropped connections: worth retrying.
fn transient(e: &str) -> bool {
    let e = e.to_lowercase();
    !(e.contains("unknownaccount")
        || e.contains("unknown_account")
        || e.contains("does not exist")
        || e.contains("methodnotfound")
        || e.contains("contractcodenotfound")
        || e.contains("no_contract_code")
        || e.contains("functioncallerror")
        || e.contains("execution error")
        || e.contains("wasm execution failed")
        || e.contains("smart contract panicked")
        || e.contains("panicked")
        || e.contains("guestpanic")
        || e.contains("failed to deserialize")
        || e.contains("wasmexecutionfailed"))
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
        let mut c = Chain::new(
            Some(sbx),
            net,
            funder,
            DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY,
            "near-sandbox (local node)",
            None,
        );
        c.deploy(signer, judge).await.unwrap();
        c
    }

    /// Testnet, operated by `HT_OPERATOR` with the key in `HT_OPERATOR_KEY`
    /// (env only). Every account the demo uses is a sub-account of the
    /// operator with a key derived from the operator key, so a rerun finds
    /// and reuses what an earlier (interrupted) run created.
    pub async fn testnet(signer: &str, judge: Option<&str>) -> Result<Chain, String> {
        let mut c = Chain::testnet_connect()?;
        c.deploy(signer, judge).await?;
        Ok(c)
    }

    /// Testnet without deploying: `usdc.<op>` and `hub.<op>` must exist.
    pub fn testnet_connect() -> Result<Chain, String> {
        let op =
            std::env::var("HT_OPERATOR").map_err(|_| "set HT_OPERATOR (e.g. hiretree.testnet)")?;
        let key =
            std::env::var("HT_OPERATOR_KEY").map_err(|_| "set HT_OPERATOR_KEY (ed25519:...)")?;
        let rpc = std::env::var("HT_RPC").unwrap_or_else(|_| "https://rpc.testnet.near.org".into());
        let net = NetworkConfig::from_rpc_url(
            "testnet",
            rpc.parse().map_err(|e| format!("HT_RPC: {e}"))?,
        );
        let funder = Acc {
            id: op.parse().map_err(|e| format!("HT_OPERATOR: {e}"))?,
            signer: Signer::from_secret_key(
                key.parse().map_err(|e| format!("HT_OPERATOR_KEY: {e:?}"))?,
            )
            .map_err(|e| format!("{e:?}"))?,
        };
        let mut c = Chain::new(
            None,
            net,
            funder,
            &key,
            "NEAR testnet",
            Some("https://testnet.nearblocks.io/txns/".into()),
        );
        c.usdc = format!("usdc.{op}").parse().unwrap();
        c.usdc_owner = Acc {
            id: c.usdc.clone(),
            signer: c.derived("usdc"),
        };
        c.hub = format!("hub.{op}").parse().unwrap();
        Ok(c)
    }

    fn new(
        sbx: Option<near_sandbox::Sandbox>,
        net: NetworkConfig,
        funder: Acc,
        seed: &str,
        name: &str,
        explorer: Option<String>,
    ) -> Chain {
        Chain {
            sbx,
            net,
            funder: funder.clone(),
            seed: Sha256::digest(seed.as_bytes()).into(),
            usdc: funder.id.clone(),
            usdc_owner: funder.clone(),
            hub: funder.id.clone(),
            network_name: name.into(),
            explorer,
            txs: Default::default(),
        }
    }

    pub fn is_testnet(&self) -> bool {
        self.sbx.is_none()
    }

    /// NEAR to fund an account with: generous on the local node, tight on
    /// testnet (where Alex pays from the faucet).
    fn near_for(&self, sandbox: u128, testnet_milli: u128) -> NearToken {
        if self.is_testnet() {
            NearToken::from_millinear(testnet_milli)
        } else {
            NearToken::from_near(sandbox)
        }
    }

    pub async fn has_code(&self, id: &AccountId) -> bool {
        match Account(id.clone()).view().fetch_from(&self.net).await {
            Ok(v) => !matches!(
                v.data.contract_state,
                near_api::types::account::ContractState::None
            ),
            Err(_) => false,
        }
    }

    /// Deploy (or find) mock-usdc, the global job code and the factory.
    pub async fn deploy(&mut self, signer: &str, judge: Option<&str>) -> Result<(), String> {
        let code = wasm("JOB_WASM", "hire_tree_job.wasm");
        let usdc = self.account_with("usdc", self.near_for(20, 3500)).await;
        if !self.has_code(&usdc.id).await {
            Contract::deploy(usdc.id.clone())
                .use_code(wasm("USDC_WASM", "mock_usdc.wasm"))
                .with_init_call("new", json!({ "owner_id": usdc.id }))
                .unwrap()
                .with_signer(usdc.signer.clone())
                .send_to(&self.net)
                .await
                .map_err(|e| format!("deploy usdc: {e:?}"))?
                .assert_success();
        }
        self.usdc = usdc.id.clone();
        self.usdc_owner = usdc.clone();
        // A global contract burns 10x the storage price per byte.
        let global_milli = (code.len() as u128 * 10 * 10 / 1000) + 1;
        let hub = self
            .account_with("hub", self.near_for(100, global_milli + 5500))
            .await;
        let code_hash: [u8; 32] = Sha256::digest(&code).into();
        let hash_s = near_api::types::CryptoHash(code_hash).to_string();
        self.ensure_global(&hub, code, code_hash, global_milli)
            .await?;
        if !self.has_code(&hub.id).await {
            Contract::deploy(hub.id.clone())
                .use_code(wasm("FACTORY_WASM", "hire_tree_factory.wasm"))
                .with_init_call(
                    "new",
                    json!({ "owner": hub.id, "token": usdc.id, "code_hash": hash_s,
                            "signer": signer, "checker": null }),
                )
                .unwrap()
                .with_signer(hub.signer.clone())
                .send_to(&self.net)
                .await
                .map_err(|e| format!("deploy factory: {e:?}"))?
                .assert_success();
        }
        self.hub = hub.id.clone();
        // Settings follow this run's model and signer (a rerun may change them).
        self.ok(&hub, &hub.id, "set_settings", json!({ "code_hash": hash_s, "signer": signer, "judge": judge, "clear_judge": judge.is_none() }))
            .await;
        self.register(&hub).await;
        Ok(())
    }

    /// Make sure the job code is a global contract, deploying it only when
    /// it is missing. A small probe account `g<hash>.<hub>` that uses the
    /// code by hash proves it exists (there is no RPC view for global code),
    /// so an interrupted run never pays the ~27 NEAR deploy twice, and a
    /// rebuilt wasm with a new hash is deployed before the factory points at it.
    async fn ensure_global(
        &self,
        hub: &Acc,
        code: Vec<u8>,
        hash: [u8; 32],
        milli: u128,
    ) -> Result<(), String> {
        let tag = hex::encode(&hash[..6]);
        let probe_id: AccountId = format!("g{tag}.{}", hub.id).parse().unwrap();
        if self.has_code(&probe_id).await {
            return Ok(());
        }
        let probe = Acc {
            id: probe_id.clone(),
            signer: self.derived(&format!("probe-{tag}")),
        };
        if !self.alive(&probe_id).await {
            Account::create_account(probe_id.clone())
                .fund_myself(hub.id.clone(), NearToken::from_millinear(50))
                .with_public_key(probe.signer.get_public_key().await.unwrap())
                .with_signer(hub.signer.clone())
                .send_to(&self.net)
                .await
                .map_err(|e| format!("create probe: {e:?}"))?
                .into_result()
                .map_err(|e| format!("create probe: {e:?}"))?;
        }
        let use_global = || async {
            Contract::deploy(probe_id.clone())
                .use_global_hash(near_api::types::CryptoHash(hash))
                .without_init_call()
                .with_signer(probe.signer.clone())
                .send_to(&self.net)
                .await
                .map_err(|e| format!("{e:?}"))?
                .into_result()
                .map(|_| ())
                .map_err(|e| format!("{e:?}"))
        };
        if use_global().await.is_ok() {
            return Ok(());
        }
        // Missing: top up the hub if needed, deploy, then prove it.
        let need = milli * 10u128.pow(21) + 10u128.pow(24);
        let have = self.near_of(&hub.id).await;
        if self.is_testnet() && have < need {
            near_api::Tokens::account(self.funder.id.clone())
                .send_to(hub.id.clone())
                .near(NearToken::from_yoctonear(need - have))
                .with_signer(self.funder.signer.clone())
                .send_to(&self.net)
                .await
                .map_err(|e| format!("top up hub: {e:?}"))?
                .into_result()
                .map_err(|e| format!("top up hub: {e:?}"))?;
        }
        let before = self.near_of(&hub.id).await;
        Contract::deploy_global_contract_code(code)
            .as_hash()
            .with_signer(hub.id.clone(), hub.signer.clone())
            .send_to(&self.net)
            .await
            .map_err(|e| format!("deploy global code: {e:?}"))?
            .into_result()
            .map_err(|e| format!("deploy global code: {e:?}"))?;
        eprintln!(
            "  global job code {}: cost {:.2} NEAR",
            near_api::types::CryptoHash(hash),
            (before - self.near_of(&hub.id).await) as f64 / 1e24
        );
        use_global()
            .await
            .map_err(|e| format!("global code still missing after deploy: {e}"))
    }

    pub async fn near_of(&self, id: &AccountId) -> u128 {
        Account(id.clone())
            .view()
            .fetch_from(&self.net)
            .await
            .map(|v| v.data.amount.as_yoctonear())
            .unwrap_or(0)
    }

    /// A new collective-purchase pool `name.<funder>` running the pool
    /// contract, registered on the token by `organiser`. Its deploy key is
    /// deleted afterwards, so nobody (not even the operator) can move the
    /// members' money except through the pool's own methods.
    pub async fn deploy_pool(
        &self,
        organiser: &Acc,
        name: &str,
        init: Value,
    ) -> Result<AccountId, String> {
        let code = wasm("POOL_WASM", "hire_tree_pool.wasm");
        let milli = code.len() as u128 / 100 + 400;
        let pool = self.account_with(name, self.near_for(5, milli)).await;
        if self.has_code(&pool.id).await {
            return Err(format!("{} already exists: pools are used once", pool.id));
        }
        Contract::deploy(pool.id.clone())
            .use_code(code)
            .with_init_call("new", init)
            .unwrap()
            .with_signer(pool.signer.clone())
            .send_to(&self.net)
            .await
            .map_err(|e| format!("deploy pool: {e:?}"))?
            .assert_success();
        self.call(
            organiser,
            &self.usdc,
            "storage_deposit",
            json!({ "account_id": pool.id, "registration_only": true }),
            NearToken::from_millinear(10),
        )
        .await?;
        let pk = pool
            .signer
            .get_public_key()
            .await
            .map_err(|e| format!("{e:?}"))?;
        Account(pool.id.clone())
            .delete_key(pk)
            .with_signer(pool.signer.clone())
            .send_to(&self.net)
            .await
            .map_err(|e| format!("delete pool key: {e:?}"))?
            .assert_success();
        Ok(pool.id)
    }

    /// A funded sub-account of the funder (sandbox: `near` NEAR; testnet:
    /// 1 NEAR). Reused if it already exists.
    pub async fn account(&self, name: &str, near: u128) -> Acc {
        let a = self.account_with(name, self.near_for(near, 1000)).await;
        // Reruns spend prepaid storage and hire deposits: keep 0.3 NEAR liquid.
        if self.is_testnet() && self.near_of(&a.id).await < 300 * 10u128.pow(21) {
            let _ = near_api::Tokens::account(self.funder.id.clone())
                .send_to(a.id.clone())
                .near(NearToken::from_millinear(700))
                .with_signer(self.funder.signer.clone())
                .send_to(&self.net)
                .await;
        }
        a
    }

    /// The key for `name` is derived from the operator key: reruns find it.
    pub fn derived(&self, name: &str) -> Arc<Signer> {
        let mut h = Sha256::new();
        h.update(self.seed);
        h.update(name.as_bytes());
        let sk = near_api::types::SecretKey::ED25519(
            near_api::types::crypto::secret_key::ED25519SecretKey::from_secret_key(
                h.finalize().into(),
            ),
        );
        Signer::from_secret_key(sk).unwrap()
    }

    pub async fn account_with(&self, name: &str, amount: NearToken) -> Acc {
        let id: AccountId = format!("{name}.{}", self.funder.id).parse().unwrap();
        let signer = self.derived(name);
        if !self.alive(&id).await {
            let pk = signer.get_public_key().await.unwrap();
            Account::create_account(id.clone())
                .fund_myself(self.funder.id.clone(), amount)
                .with_public_key(pk)
                .with_signer(self.funder.signer.clone())
                .send_to(&self.net)
                .await
                .unwrap()
                .assert_success();
        }
        Acc { id, signer }
    }

    /// Add a function-call access key to `owner`'s account and return the
    /// account as that key signs for it. NEAR limits such a key to ONE
    /// receiver account and a list of methods, and refuses any attached
    /// deposit: it can neither transfer NEAR, nor delete the account, nor
    /// call `ft_transfer` (which needs 1 yocto), nor touch any other
    /// contract. `label` makes the key unique per use. The owner's own key
    /// signs the AddKey; the returned key never holds more than `allowance`
    /// NEAR of gas.
    pub async fn grant_key(
        &self,
        owner: &Acc,
        label: &str,
        receiver: &AccountId,
        methods: &[&str],
        allowance: NearToken,
    ) -> Result<Acc, String> {
        let signer = self.derived(&format!("fc:{}:{receiver}:{label}", owner.id));
        let pk = signer
            .get_public_key()
            .await
            .map_err(|e| format!("{e:?}"))?;
        Account(owner.id.clone())
            .add_key(
                near_api::types::AccessKeyPermission::FunctionCall(
                    near_api::types::transaction::actions::FunctionCallPermission {
                        allowance: Some(allowance),
                        receiver_id: receiver.to_string(),
                        method_names: methods.iter().map(|m| m.to_string()).collect(),
                    },
                ),
                pk,
            )
            .with_signer(owner.signer.clone())
            .send_to(&self.net)
            .await
            .map_err(|e| format!("{e:?}"))?
            .into_result()
            .map_err(|e| format!("{e:?}"))?;
        Ok(Acc {
            id: owner.id.clone(),
            signer,
        })
    }

    /// Delete a key `grant_key` made, signed by the owner's own key.
    pub async fn revoke_key(&self, owner: &Acc, key: &Acc) -> Result<(), String> {
        let pk = key
            .signer
            .get_public_key()
            .await
            .map_err(|e| format!("{e:?}"))?;
        Account(owner.id.clone())
            .delete_key(pk)
            .with_signer(owner.signer.clone())
            .send_to(&self.net)
            .await
            .map_err(|e| format!("{e:?}"))?
            .into_result()
            .map_err(|e| format!("{e:?}"))?;
        Ok(())
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
        let gas = full.total_gas_burnt.as_gas();
        let res = full.into_result();
        self.txs.lock().unwrap().push(
            json!({ "who": who.id, "method": method, "on": contract, "tx": tx, "ok": res.is_ok(), "gas": gas }),
        );
        match res {
            Ok(ok) => {
                eprintln!("  {method} on {contract}: ok (tx {tx})");
                Ok(ok.json::<Value>().unwrap_or(Value::Null))
            }
            Err(e) => {
                let e = short_error(&format!("{e:?}"));
                eprintln!("  {method} on {contract}: FAILED (tx {tx}): {e}");
                Err(e)
            }
        }
    }

    pub async fn ok(&self, who: &Acc, contract: &AccountId, method: &str, args: Value) -> Value {
        self.call(who, contract, method, args, NearToken::from_yoctonear(0))
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    /// A view call; transport errors (rate limits, timeouts) are retried, so
    /// a snapshot never shows a guess.
    pub async fn view(
        &self,
        contract: &AccountId,
        method: &str,
        args: Value,
    ) -> Result<Value, String> {
        let mut last = String::new();
        for attempt in 0..6u64 {
            match Contract(contract.clone())
                .call_function(method, args.clone())
                .read_only::<Value>()
                .fetch_from(&self.net)
                .await
            {
                Ok(r) => return Ok(r.data),
                Err(e) => {
                    last = format!("{e:?}");
                    if !transient(&last) {
                        return Err(last);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500 << attempt)).await;
        }
        Err(last)
    }

    /// Does the account exist? Only "unknown account" means no; transport
    /// errors are retried and then fail loudly.
    pub async fn alive(&self, id: &AccountId) -> bool {
        for attempt in 0..6u64 {
            match Account(id.clone()).view().fetch_from(&self.net).await {
                Ok(_) => return true,
                Err(e) => {
                    let e = format!("{e:?}");
                    if !transient(&e) {
                        return false;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500 << attempt)).await;
        }
        panic!("RPC unreachable while checking {id}; set HT_RPC to another testnet RPC");
    }

    pub async fn ft(&self, id: &AccountId) -> u128 {
        let v = self
            .view(&self.usdc, "ft_balance_of", json!({ "account_id": id }))
            .await
            .unwrap_or_else(|e| panic!("ft_balance_of {id}: {e}"));
        num(&v)
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
        // Chain time, not the PC clock.
        let now = self.block_time_ns().await;
        if end_ns > now {
            eprintln!(
                "  waiting {}s for the job window to close",
                (end_ns - now) / 1_000_000_000
            );
            tokio::time::sleep(std::time::Duration::from_nanos(end_ns - now)).await;
        }
        while self.block_time_ns().await <= end_ns {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::transient;

    #[test]
    fn a_view_the_contract_refuses_is_not_retried() {
        // These used to be retried for 31.5 s each.
        for e in [
            "GuestPanic { panic_msg: \"list at most 16 payers\" }",
            "Failed to deserialize input from JSON. Error: `invalid value`",
            "WasmExecutionFailed",
            "Smart contract panicked: only jobs of this factory",
        ] {
            assert!(!transient(e), "{e}");
        }
        for e in [
            "429 Too Many Requests",
            "connection reset by peer",
            "timed out",
        ] {
            assert!(transient(e), "{e}");
        }
    }
}
