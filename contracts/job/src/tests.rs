//! Unit tests in near-sdk's mocked runtime. Each method's checks plus the
//! success and failure paths of every callback.
//!
//! Not testable here (sandbox day): real global-contract deployment and
//! account creation, the `#[private]` wrapper itself (we test the
//! `assert_self` check it repeats), real token behaviour, delete + storage
//! refund, and gas use.

// Tests call methods for their side effects and inspect receipts instead.
#![allow(unused_must_use)]

use super::*;
use near_sdk::mock::MockAction;
use near_sdk::test_utils::{get_created_receipts, get_logs, VMContextBuilder};
use near_sdk::{test_vm_config, testing_env, PromiseResult, RuntimeFeesConfig};

const FACTORY: &str = "hiretree.test";
const ROOT: &str = "job-1.hiretree.test";
const T1: &str = "t1.job-1.hiretree.test";
const T2: &str = "t2.job-1.hiretree.test";
const CLIENT: &str = "client.test";
const LEAD: &str = "lead.test";
const WORKER: &str = "worker.test";
const WORKER2: &str = "worker2.test";
const TOKEN: &str = "usdc.test";
const EVE: &str = "eve.test";

const USD: u128 = 1_000_000;
const WINDOW: u64 = 72 * 3600;
const T0: u64 = 1_000 * NS_PER_SEC;
const AFTER: u64 = T0 + (WINDOW + 1) * NS_PER_SEC;

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn spec() -> String {
    "ab".repeat(32)
}

fn signer() -> String {
    "11".repeat(20)
}

fn code_hash() -> Base58CryptoHash {
    [7u8; 32].into()
}

struct Ctx<'a> {
    current: &'a str,
    pred: &'a str,
    now: u64,
    deposit: NearToken,
    gas: Gas,
    results: Vec<PromiseResult>,
}

impl<'a> Ctx<'a> {
    fn new(current: &'a str, pred: &'a str) -> Self {
        Ctx {
            current,
            pred,
            now: T0,
            deposit: NearToken::from_yoctonear(0),
            gas: Gas::from_tgas(300),
            results: vec![],
        }
    }
    fn at(mut self, now: u64) -> Self {
        self.now = now;
        self
    }
    fn deposit(mut self, d: NearToken) -> Self {
        self.deposit = d;
        self
    }
    fn gas(mut self, g: Gas) -> Self {
        self.gas = g;
        self
    }
    fn results(mut self, r: Vec<PromiseResult>) -> Self {
        self.results = r;
        self
    }
    fn set(self) {
        testing_env!(
            VMContextBuilder::new()
                .current_account_id(acc(self.current))
                .predecessor_account_id(acc(self.pred))
                .block_timestamp(self.now)
                .attached_deposit(self.deposit)
                .prepaid_gas(self.gas)
                .account_balance(NearToken::from_near(10))
                .build(),
            test_vm_config(),
            RuntimeFeesConfig::test(),
            Default::default(),
            self.results
        );
    }
}

fn ok(bytes: &[u8]) -> PromiseResult {
    PromiseResult::Successful(bytes.to_vec())
}

fn u128_json(x: u128) -> Vec<u8> {
    format!("\"{x}\"").into_bytes()
}

fn value(r: PromiseOrValue<U128>) -> u128 {
    match r {
        PromiseOrValue::Value(v) => v.0,
        PromiseOrValue::Promise(_) => panic!("expected a value"),
    }
}

/// (receiver, method) of every function call created in this context.
fn calls() -> Vec<(String, String)> {
    get_created_receipts()
        .into_iter()
        .flat_map(|r| {
            let to = r.receiver_id.to_string();
            r.actions.into_iter().filter_map(move |a| match a {
                MockAction::FunctionCallWeight { method_name, .. } => {
                    Some((to.clone(), String::from_utf8(method_name).unwrap()))
                }
                _ => None,
            })
        })
        .collect()
}

fn call_args(method: &str) -> serde_json::Value {
    for r in get_created_receipts() {
        for a in r.actions {
            if let MockAction::FunctionCallWeight {
                method_name, args, ..
            } = a
            {
                if method_name == method.as_bytes() {
                    return serde_json::from_slice(&args).unwrap();
                }
            }
        }
    }
    panic!("no call to {method}");
}

fn events(name: &str) -> Vec<serde_json::Value> {
    get_logs()
        .iter()
        .filter_map(|l| l.strip_prefix("EVENT_JSON:"))
        .map(|j| serde_json::from_str::<serde_json::Value>(j).unwrap())
        .filter(|v| v["event"] == name)
        .map(|v| v["data"][0].clone())
        .collect()
}

fn new_root() -> Job {
    Ctx::new(ROOT, FACTORY).set();
    Job::new(
        Some(acc(CLIENT)),
        acc(LEAD),
        None,
        acc(TOKEN),
        U128(0),
        spec(),
        WINDOW,
        signer(),
        code_hash(),
        None,
        None,
    )
}

fn funded_root(amount: u128) -> Job {
    let mut j = new_root();
    Ctx::new(ROOT, TOKEN).set();
    assert_eq!(
        value(j.ft_on_transfer(acc(CLIENT), U128(amount), "fund".into())),
        0
    );
    j
}

fn new_sub(pay: u128) -> Job {
    Ctx::new(T1, ROOT).set();
    Job::new(
        None,
        acc(LEAD),
        Some(acc(WORKER)),
        acc(TOKEN),
        U128(pay),
        spec(),
        WINDOW,
        signer(),
        code_hash(),
        None,
        None,
    )
}

fn funded_sub(pay: u128, total: u128) -> Job {
    let mut j = new_sub(pay);
    Ctx::new(T1, TOKEN).set();
    assert_eq!(
        value(j.ft_on_transfer(acc(ROOT), U128(total), "fund".into())),
        0
    );
    j
}

fn hire_one(j: &mut Job, worker: &str, amount: u128) -> u32 {
    let idx = j.children.len() as u32;
    Ctx::new(ROOT, LEAD).deposit(CHILD_BASE_NEAR).set();
    j.hire(acc(worker), U128(amount), spec(), None);
    idx
}

/// Hire and run both callbacks with success.
fn hired(j: &mut Job, worker: &str, amount: u128) -> u32 {
    let idx = hire_one(j, worker, amount);
    Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
    assert!(matches!(
        j.on_child_created(idx),
        PromiseOrValue::Promise(_)
    ));
    Ctx::new(ROOT, ROOT)
        .results(vec![ok(&u128_json(amount))])
        .set();
    assert!(j.on_child_funded(idx));
    idx
}

fn delivered_sub(pay: u128, total: u128) -> Job {
    let mut j = funded_sub(pay, total);
    Ctx::new(T1, WORKER).set();
    j.deliver("cd".repeat(32), None, None, None);
    j
}

// ================================================================ init

#[test]
fn root_init_sets_fields() {
    let j = new_root();
    let v = j.get_job();
    assert!(v.root);
    assert_eq!(v.parent, acc(FACTORY));
    assert_eq!(v.client, Some(acc(CLIENT)));
    assert_eq!(v.manager, acc(LEAD));
    assert_eq!(v.status, "Open");
    assert_eq!(v.window_end_ns.0, T0 + WINDOW * NS_PER_SEC);
    assert_eq!(v.spec_hash, spec());
    assert_eq!(events("job_created").len(), 1);
}

#[test]
fn sub_init_sets_worker_as_payee() {
    let j = new_sub(30 * USD);
    let v = j.get_job();
    assert!(!v.root);
    assert_eq!(v.parent, acc(ROOT));
    assert_eq!(v.worker, Some(acc(WORKER)));
    assert_eq!(v.payee, Some(acc(WORKER)));
    assert_eq!(v.amount.0, 30 * USD);
}

#[test]
#[should_panic(expected = "sub-account of its creator")]
fn init_must_be_called_by_parent_account() {
    Ctx::new(ROOT, EVE).set();
    Job::new(
        Some(acc(CLIENT)),
        acc(LEAD),
        None,
        acc(TOKEN),
        U128(0),
        spec(),
        WINDOW,
        signer(),
        code_hash(),
        None,
        None,
    );
}

#[test]
#[should_panic(expected = "root job has no own pay")]
fn root_with_amount_refused() {
    Ctx::new(ROOT, FACTORY).set();
    Job::new(
        Some(acc(CLIENT)),
        acc(LEAD),
        None,
        acc(TOKEN),
        U128(1),
        spec(),
        WINDOW,
        signer(),
        code_hash(),
        None,
        None,
    );
}

#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn sub_with_zero_amount_refused() {
    new_sub(0);
}

#[test]
#[should_panic(expected = "exactly one of client")]
fn client_and_worker_both_refused() {
    Ctx::new(ROOT, FACTORY).set();
    Job::new(
        Some(acc(CLIENT)),
        acc(LEAD),
        Some(acc(WORKER)),
        acc(TOKEN),
        U128(0),
        spec(),
        WINDOW,
        signer(),
        code_hash(),
        None,
        None,
    );
}

#[test]
#[should_panic(expected = "spec_hash must be 64 hex chars")]
fn bad_spec_hash_refused() {
    Ctx::new(ROOT, FACTORY).set();
    Job::new(
        Some(acc(CLIENT)),
        acc(LEAD),
        None,
        acc(TOKEN),
        U128(0),
        "xyz".into(),
        WINDOW,
        signer(),
        code_hash(),
        None,
        None,
    );
}

#[test]
#[should_panic(expected = "signer must be a 20-byte hex address")]
fn bad_signer_refused() {
    Ctx::new(ROOT, FACTORY).set();
    Job::new(
        Some(acc(CLIENT)),
        acc(LEAD),
        None,
        acc(TOKEN),
        U128(0),
        spec(),
        WINDOW,
        "1234".into(),
        code_hash(),
        None,
        None,
    );
}

#[test]
#[should_panic(expected = "window_secs out of range")]
fn window_too_long_refused() {
    Ctx::new(ROOT, FACTORY).set();
    Job::new(
        Some(acc(CLIENT)),
        acc(LEAD),
        None,
        acc(TOKEN),
        U128(0),
        spec(),
        MAX_WINDOW_SECS + 1,
        signer(),
        code_hash(),
        None,
        None,
    );
}

// ================================================================ ft_on_transfer

#[test]
fn client_funds_root() {
    let j = funded_root(100 * USD);
    let v = j.get_job();
    assert_eq!(v.funded.0, 100 * USD);
    assert_eq!(v.ft_balance.0, 100 * USD);
    assert_eq!(v.unallocated.0, 100 * USD);
    assert_eq!(events("funded").len(), 1);
}

#[test]
fn wrong_token_is_refunded_without_state_change() {
    let mut j = new_root();
    Ctx::new(ROOT, "fake-usdc.test").set();
    assert_eq!(
        value(j.ft_on_transfer(acc(CLIENT), U128(5), "fund".into())),
        5
    );
    assert_eq!(j.get_job().funded.0, 0);
    assert_eq!(events("refund")[0]["reason"], "wrong token");
}

#[test]
fn factory_parent_can_fund_root() {
    let mut j = new_root();
    Ctx::new(ROOT, TOKEN).set();
    assert_eq!(
        value(j.ft_on_transfer(acc(FACTORY), U128(5 * USD), "fund".into())),
        0
    );
    assert_eq!(j.get_job().funded.0, 5 * USD);
}

#[test]
fn stranger_funding_root_is_refunded() {
    let mut j = new_root();
    Ctx::new(ROOT, TOKEN).set();
    assert_eq!(value(j.ft_on_transfer(acc(EVE), U128(5), "fund".into())), 5);
    assert_eq!(j.get_job().ft_balance.0, 0);
}

#[test]
fn unknown_msg_is_refunded() {
    let mut j = new_root();
    Ctx::new(ROOT, TOKEN).set();
    assert_eq!(
        value(j.ft_on_transfer(acc(CLIENT), U128(5), "gift".into())),
        5
    );
    assert_eq!(events("refund")[0]["reason"], "unknown msg");
}

#[test]
fn zero_amount_is_refunded() {
    let mut j = new_root();
    Ctx::new(ROOT, TOKEN).set();
    assert_eq!(
        value(j.ft_on_transfer(acc(CLIENT), U128(0), "fund".into())),
        0
    );
    assert_eq!(events("refund").len(), 1);
}

#[test]
fn manager_posts_bond_on_root() {
    let mut j = funded_root(100 * USD);
    Ctx::new(ROOT, TOKEN).set();
    assert_eq!(
        value(j.ft_on_transfer(acc(LEAD), U128(10 * USD), "bond".into())),
        0
    );
    let v = j.get_job();
    assert_eq!(v.bond.0, 10 * USD);
    assert_eq!(v.ft_balance.0, 110 * USD);
    // The bond is never hireable.
    assert_eq!(v.unallocated.0, 100 * USD);
}

#[test]
fn bond_by_stranger_or_on_sub_is_refunded() {
    let mut j = new_root();
    Ctx::new(ROOT, TOKEN).set();
    assert_eq!(value(j.ft_on_transfer(acc(EVE), U128(5), "bond".into())), 5);
    let mut s = new_sub(USD);
    Ctx::new(T1, TOKEN).set();
    assert_eq!(
        value(s.ft_on_transfer(acc(LEAD), U128(5), "bond".into())),
        5
    );
}

#[test]
fn buy_note_without_a_treasury_or_check_is_refunded() {
    let mut j = delivered_sub(30 * USD, 30 * USD);
    Ctx::new(T1, TOKEN).set();
    assert_eq!(
        value(j.ft_on_transfer(acc(LEAD), U128(29_700_000), "buy_note".into())),
        29_700_000
    );
    let v = j.get_job();
    assert!(!v.note_sold);
    assert_eq!(v.payee, Some(acc(WORKER)));
    assert!(events("refund")[0]["reason"]
        .as_str()
        .unwrap()
        .contains("only the lead's treasury"));
}

#[test]
fn sub_accepts_funding_only_from_parent() {
    let mut j = new_sub(USD);
    Ctx::new(T1, TOKEN).set();
    assert_eq!(
        value(j.ft_on_transfer(acc(LEAD), U128(USD), "fund".into())),
        USD
    );
    assert_eq!(
        value(j.ft_on_transfer(acc(ROOT), U128(USD), "fund".into())),
        0
    );
    assert_eq!(j.get_job().funded.0, USD);
}

#[test]
fn funding_after_window_is_refunded() {
    let mut j = new_root();
    Ctx::new(ROOT, TOKEN).at(AFTER).set();
    assert_eq!(
        value(j.ft_on_transfer(acc(CLIENT), U128(5), "fund".into())),
        5
    );
}

// ================================================================ hire

#[test]
fn hire_creates_child_from_global_contract() {
    let mut j = funded_root(100 * USD);
    Ctx::new(ROOT, LEAD)
        .deposit(NearToken::from_millinear(50))
        .set();
    j.hire(acc(WORKER), U128(30 * USD), spec(), Some(U128(5 * USD)));
    let receipts = get_created_receipts();
    let create = receipts
        .iter()
        .find(|r| r.receiver_id == acc(T1))
        .expect("receipt to t1");
    let mut saw = (false, false, false, false);
    for a in &create.actions {
        match a {
            MockAction::CreateAccount { .. } => saw.0 = true,
            MockAction::Transfer { deposit, .. } => {
                assert_eq!(*deposit, NearToken::from_millinear(50));
                saw.1 = true;
            }
            MockAction::UseGlobalContract { contract_id, .. } => {
                assert!(format!("{contract_id:?}").contains("CodeHash"));
                saw.2 = true;
            }
            MockAction::FunctionCallWeight { method_name, .. } => {
                assert_eq!(method_name, b"new");
                saw.3 = true;
            }
            _ => {}
        }
    }
    assert_eq!(saw, (true, true, true, true));
    let init = call_args("new");
    assert_eq!(init["worker"], WORKER);
    assert_eq!(init["manager"], LEAD);
    assert_eq!(init["amount"], (30 * USD).to_string());
    assert!(init["client"].is_null());
    assert!(calls().contains(&(ROOT.into(), "on_child_created".into())));
    // State before promises: allocated now, pending.
    let v = j.get_job();
    assert_eq!(v.allocated.0, 35 * USD);
    assert_eq!(v.ft_balance.0, 65 * USD);
    assert_eq!(v.children_pending, 1);
    assert_eq!(v.children[0].state, ChildState::Creating);
}

#[test]
fn children_get_increasing_names() {
    let mut j = funded_root(100 * USD);
    hire_one(&mut j, WORKER, USD);
    hire_one(&mut j, WORKER2, USD);
    let v = j.get_job();
    assert_eq!(v.children[0].account, acc(T1));
    assert_eq!(v.children[1].account, acc(T2));
}

#[test]
#[should_panic(expected = "attach at least 0.03 NEAR")]
fn hire_without_storage_deposit_refused() {
    let mut j = funded_root(100 * USD);
    Ctx::new(ROOT, LEAD).set();
    j.hire(acc(WORKER), U128(USD), spec(), None);
}

#[test]
#[should_panic(expected = "only the manager can hire")]
fn non_manager_cannot_hire() {
    let mut j = funded_root(100 * USD);
    Ctx::new(ROOT, CLIENT).set();
    j.hire(acc(WORKER), U128(USD), spec(), None);
}

#[test]
#[should_panic(expected = "exceeds unallocated funds")]
fn hire_over_budget_refused() {
    let mut j = funded_root(10 * USD);
    hire_one(&mut j, WORKER, 6 * USD);
    hire_one(&mut j, WORKER2, 5 * USD);
}

#[test]
#[should_panic(expected = "exceeds unallocated funds")]
fn bond_cannot_be_hired_with() {
    let mut j = funded_root(10 * USD);
    Ctx::new(ROOT, TOKEN).set();
    j.ft_on_transfer(acc(LEAD), U128(10 * USD), "bond".into());
    hire_one(&mut j, WORKER, 11 * USD);
}

#[test]
#[should_panic(expected = "amount must be greater than zero")]
fn hire_zero_refused() {
    let mut j = funded_root(10 * USD);
    hire_one(&mut j, WORKER, 0);
}

#[test]
#[should_panic(expected = "amount overflow")]
fn hire_overflow_refused() {
    let mut j = funded_root(10 * USD);
    Ctx::new(ROOT, LEAD).deposit(CHILD_BASE_NEAR).set();
    j.hire(acc(WORKER), U128(u128::MAX), spec(), Some(U128(1)));
}

#[test]
#[should_panic(expected = "not enough gas for hire")]
fn hire_with_low_gas_refused() {
    let mut j = funded_root(10 * USD);
    Ctx::new(ROOT, LEAD)
        .gas(Gas::from_tgas(100))
        .deposit(CHILD_BASE_NEAR)
        .set();
    j.hire(acc(WORKER), U128(USD), spec(), None);
}

#[test]
#[should_panic(expected = "window is closed")]
fn hire_after_approve_refused() {
    let mut j = funded_root(10 * USD);
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    hire_one(&mut j, WORKER, USD);
}

#[test]
#[should_panic(expected = "too many children")]
fn hire_capped_at_max_children() {
    let mut j = funded_root(100 * USD);
    for _ in 0..=MAX_CHILDREN {
        hire_one(&mut j, WORKER, 1);
    }
}

#[test]
#[should_panic(expected = "worker cannot be the job itself")]
fn job_cannot_hire_itself() {
    let mut j = funded_root(10 * USD);
    hire_one(&mut j, ROOT, USD);
}

// ================================================================ hire callbacks

#[test]
fn create_failure_rolls_back() {
    let mut j = funded_root(100 * USD);
    let idx = hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, ROOT)
        .results(vec![PromiseResult::Failed])
        .set();
    assert!(matches!(
        j.on_child_created(idx),
        PromiseOrValue::Value(false)
    ));
    let v = j.get_job();
    assert_eq!(v.allocated.0, 0);
    assert_eq!(v.ft_balance.0, 100 * USD);
    assert_eq!(v.children_pending, 0);
    assert_eq!(v.children[0].state, ChildState::Failed);
    assert_eq!(events("hire_failed")[0]["stage"], "create");
    assert!(calls().is_empty(), "no token calls after a failed create");
}

#[test]
fn create_success_registers_and_funds_child() {
    let mut j = funded_root(100 * USD);
    let idx = hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
    assert!(matches!(
        j.on_child_created(idx),
        PromiseOrValue::Promise(_)
    ));
    let c = calls();
    assert!(c.contains(&(TOKEN.into(), "storage_deposit".into())));
    assert!(c.contains(&(TOKEN.into(), "ft_transfer_call".into())));
    assert!(c.contains(&(ROOT.into(), "on_child_funded".into())));
    let args = call_args("ft_transfer_call");
    assert_eq!(args["receiver_id"], T1);
    assert_eq!(args["msg"], "fund");
    assert_eq!(args["amount"], (30 * USD).to_string());
    assert_eq!(j.get_job().children[0].state, ChildState::Funding);
}

#[test]
fn fund_success_opens_child() {
    let mut j = funded_root(100 * USD);
    hired(&mut j, WORKER, 30 * USD);
    let v = j.get_job();
    assert_eq!(v.children_open, 1);
    assert_eq!(v.children_pending, 0);
    assert_eq!(v.children[0].state, ChildState::Open);
    assert_eq!(v.ft_balance.0, 70 * USD);
    assert_eq!(events("hired").len(), 1);
}

#[test]
fn fund_failure_rolls_back_and_closes_child() {
    let mut j = funded_root(100 * USD);
    let idx = hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
    j.on_child_created(idx);
    Ctx::new(ROOT, ROOT)
        .results(vec![PromiseResult::Failed])
        .set();
    assert!(!j.on_child_funded(idx));
    let v = j.get_job();
    assert_eq!(v.allocated.0, 0);
    assert_eq!(v.ft_balance.0, 100 * USD);
    assert_eq!(v.children_open, 0);
    assert_eq!(v.children_pending, 0);
    assert_eq!(v.children[0].state, ChildState::Failed);
    assert!(calls().contains(&(T1.into(), "close_unfunded".into())));
}

#[test]
fn fund_refused_by_child_rolls_back() {
    let mut j = funded_root(100 * USD);
    let idx = hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
    j.on_child_created(idx);
    // ft_transfer_call reports 0 used: the child refunded everything.
    Ctx::new(ROOT, ROOT).results(vec![ok(&u128_json(0))]).set();
    assert!(!j.on_child_funded(idx));
    assert_eq!(j.get_job().ft_balance.0, 100 * USD);
}

#[test]
fn partial_funding_keeps_books_right() {
    let mut j = funded_root(100 * USD);
    let idx = hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
    j.on_child_created(idx);
    Ctx::new(ROOT, ROOT)
        .results(vec![ok(&u128_json(10 * USD))])
        .set();
    assert!(j.on_child_funded(idx));
    let v = j.get_job();
    assert_eq!(v.allocated.0, 10 * USD);
    assert_eq!(v.ft_balance.0, 90 * USD);
    assert_eq!(v.children[0].total.0, 10 * USD);
    assert_eq!(events("anomaly").len(), 1);
}

#[test]
#[should_panic(expected = "child is not being funded")]
fn funding_callback_cannot_run_twice() {
    let mut j = funded_root(100 * USD);
    let idx = hired(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, ROOT)
        .results(vec![ok(&u128_json(30 * USD))])
        .set();
    j.on_child_funded(idx);
}

#[test]
#[should_panic(expected = "child is not being created")]
fn create_callback_for_unknown_child_panics() {
    let mut j = funded_root(100 * USD);
    Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
    j.on_child_created(3);
}

#[test]
#[should_panic(expected = "callback: caller must be this contract")]
fn callbacks_reject_outside_callers() {
    let mut j = funded_root(100 * USD);
    let idx = hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, EVE)
        .results(vec![PromiseResult::Failed])
        .set();
    j.on_child_created(idx);
}

#[test]
fn close_unfunded_deletes_to_parent() {
    let mut j = new_sub(USD);
    Ctx::new(T1, ROOT).set();
    j.close_unfunded();
    assert_eq!(j.get_job().status, "Settled");
    let del = get_created_receipts()
        .into_iter()
        .flat_map(|r| r.actions)
        .find_map(|a| match a {
            MockAction::DeleteAccount { beneficiary_id, .. } => Some(beneficiary_id),
            _ => None,
        });
    assert_eq!(del, Some(acc(ROOT)));
}

#[test]
#[should_panic(expected = "only the parent")]
fn close_unfunded_parent_only() {
    let mut j = new_sub(USD);
    Ctx::new(T1, EVE).set();
    j.close_unfunded();
}

#[test]
#[should_panic(expected = "job has funds")]
fn close_unfunded_refuses_funded_job() {
    let mut j = funded_sub(USD, USD);
    Ctx::new(T1, ROOT).set();
    j.close_unfunded();
}

// ================================================================ deliver

#[test]
fn worker_delivers() {
    let j = delivered_sub(30 * USD, 30 * USD);
    let v = j.get_job();
    assert_eq!(v.status, "Delivered");
    assert_eq!(v.seq, 1);
    assert_eq!(v.deliverable_hash, Some("cd".repeat(32)));
    assert!(!v.verified);
    assert_eq!(events("delivered")[0]["seq"], 1);
}

#[test]
#[should_panic(expected = "only the worker can deliver")]
fn non_worker_cannot_deliver() {
    let mut j = funded_sub(USD, USD);
    Ctx::new(T1, EVE).set();
    j.deliver("cd".repeat(32), None, None, None);
}

#[test]
#[should_panic(expected = "only the worker can deliver")]
fn root_has_no_worker_to_deliver() {
    let mut j = funded_root(USD);
    Ctx::new(ROOT, LEAD).set();
    j.deliver("cd".repeat(32), None, None, None);
}

#[test]
#[should_panic(expected = "window is closed")]
fn deliver_after_window_refused() {
    let mut j = funded_sub(USD, USD);
    Ctx::new(T1, WORKER).at(AFTER).set();
    j.deliver("cd".repeat(32), None, None, None);
}

#[test]
#[should_panic(expected = "illegal transition")]
fn second_delivery_refused_while_delivered() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, WORKER).set();
    j.deliver("ef".repeat(32), None, None, None);
}

// ================================================================ receipts

mod receipt_tests {
    use super::*;
    use ht_core::receipt::{eip191_digest, eth_address, sha256, sha256_hex};
    use secp256k1::{Message, Secp256k1, SecretKey};

    pub const MODEL: &str = "deepseek-ai/DeepSeek-V4.1-Flash-1017";

    pub fn key(n: u8) -> SecretKey {
        SecretKey::from_slice(&[n; 32]).unwrap()
    }

    pub fn addr_hex(k: &SecretKey) -> String {
        let pk = k.public_key(&Secp256k1::new()).serialize_uncompressed();
        let pk: [u8; 64] = pk[1..].try_into().unwrap();
        hex::encode(eth_address(&pk))
    }

    /// r || s || v (v = 27 + recovery id), hex: the NEAR AI Cloud shape.
    pub fn sign(k: &SecretKey, text: &str) -> String {
        let d = eip191_digest(text.as_bytes());
        let (rid, rs) = Secp256k1::new()
            .sign_ecdsa_recoverable(&Message::from_slice(&d).unwrap(), k)
            .serialize_compact();
        let mut out = rs.to_vec();
        out.push(27 + rid.to_i32() as u8);
        hex::encode(out)
    }

    pub fn request(task: &str, seq: u32) -> Vec<u8> {
        format!(
            "{{\"model\":\"{MODEL}\",\"messages\":[{{\"role\":\"user\",\"content\":\"HT1:{task}:{}:{seq} Translate: hello\"}}]}}",
            spec()
        )
        .into_bytes()
    }

    pub const RESPONSE: &[u8] = b"{\"choices\":[{\"message\":{\"content\":\"hola\"}}]}";

    /// A receipt for `req`/`RESPONSE` signed by `k`.
    pub fn receipt(k: &SecretKey, req: &[u8]) -> (String, Base64VecU8, String, String) {
        let text = format!("{MODEL}:{}:{}", sha256_hex(req), sha256_hex(RESPONSE));
        (
            hex::encode(sha256(RESPONSE)),
            Base64VecU8(req.to_vec()),
            sign(k, &text),
            MODEL.to_string(),
        )
    }

    /// A funded sub-job whose pinned signer is key(1).
    pub fn pinned_sub(pay: u128) -> Job {
        Ctx::new(T1, ROOT).set();
        let mut j = Job::new(
            None,
            acc(LEAD),
            Some(acc(WORKER)),
            acc(TOKEN),
            U128(pay),
            spec(),
            WINDOW,
            addr_hex(&key(1)),
            code_hash(),
            None,
            None,
        );
        Ctx::new(T1, TOKEN).set();
        j.ft_on_transfer(acc(ROOT), U128(pay), "fund".into());
        j
    }

    pub fn deliver_r(j: &mut Job, r: (String, Base64VecU8, String, String)) {
        Ctx::new(T1, WORKER).set();
        j.deliver(r.0, Some(r.1), Some(r.2), Some(r.3));
    }

    #[test]
    fn valid_receipt_marks_delivery_verified() {
        let mut j = pinned_sub(USD);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        let v = j.get_job();
        // No checker pinned here: receipt-only mode, the receipt is the check.
        assert_eq!(v.status, "Checked");
        assert!(v.verified);
        assert_eq!(v.deliverable_hash, Some(hex::encode(sha256(RESPONSE))));
        assert_eq!(events("delivered")[0]["verified"], true);
    }

    #[test]
    fn v_as_0_or_1_also_accepted() {
        let mut j = pinned_sub(USD);
        let mut r = receipt(&key(1), &request(T1, 1));
        let mut sig = hex::decode(&r.2).unwrap();
        sig[64] -= 27;
        r.2 = format!("0x{}", hex::encode(sig));
        deliver_r(&mut j, r);
        assert!(j.get_job().verified);
    }

    #[test]
    #[should_panic(expected = "receipt: signer is not the pinned NEAR AI Cloud key")]
    fn receipt_from_other_key_rejected() {
        let mut j = pinned_sub(USD);
        deliver_r(&mut j, receipt(&key(2), &request(T1, 1)));
    }

    #[test]
    #[should_panic(expected = "receipt: signer is not the pinned NEAR AI Cloud key")]
    fn receipt_with_swapped_response_rejected() {
        let mut j = pinned_sub(USD);
        let mut r = receipt(&key(1), &request(T1, 1));
        r.0 = hex::encode(sha256(b"junk the worker wants paid for"));
        deliver_r(&mut j, r);
    }

    #[test]
    #[should_panic(expected = "receipt: signer is not the pinned NEAR AI Cloud key")]
    fn receipt_with_swapped_model_rejected() {
        let mut j = pinned_sub(USD);
        let mut r = receipt(&key(1), &request(T1, 1));
        r.3 = "some-other-model".into();
        deliver_r(&mut j, r);
    }

    #[test]
    #[should_panic(expected = "is not this task")]
    fn receipt_copied_from_other_task_rejected() {
        let mut j = pinned_sub(USD);
        // A genuine receipt, made for t2's task.
        deliver_r(&mut j, receipt(&key(1), &request(T2, 1)));
    }

    #[test]
    #[should_panic(expected = "request has no HT1 binding tag")]
    fn receipt_without_binding_tag_rejected() {
        let mut j = pinned_sub(USD);
        deliver_r(&mut j, receipt(&key(1), b"{\"messages\":[\"hello\"]}"));
    }

    #[test]
    #[should_panic(expected = "is not this task")]
    fn receipt_replayed_on_redelivery_rejected() {
        let mut j = pinned_sub(USD);
        let r = receipt(&key(1), &request(T1, 1));
        deliver_r(&mut j, r.clone());
        // The check sends it back to Open; the worker replays receipt #1.
        j.status = Status::Open;
        deliver_r(&mut j, r);
    }

    #[test]
    fn fresh_receipt_on_redelivery_accepted() {
        let mut j = pinned_sub(USD);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        j.status = Status::Open;
        deliver_r(&mut j, receipt(&key(1), &request(T1, 2)));
        assert_eq!(j.get_job().seq, 2);
        assert!(j.get_job().verified);
    }

    #[test]
    #[should_panic(expected = "receipt: bad signature")]
    fn malformed_signature_rejected() {
        let mut j = pinned_sub(USD);
        let mut r = receipt(&key(1), &request(T1, 1));
        r.2 = "00".repeat(64);
        deliver_r(&mut j, r);
    }

    #[test]
    #[should_panic(expected = "receipt: bad signature")]
    fn bad_v_rejected() {
        let mut j = pinned_sub(USD);
        let mut r = receipt(&key(1), &request(T1, 1));
        let mut sig = hex::decode(&r.2).unwrap();
        sig[64] = 29;
        r.2 = hex::encode(sig);
        deliver_r(&mut j, r);
    }

    #[test]
    #[should_panic(expected = "receipt: bad signature")]
    fn high_s_signature_rejected() {
        // s' = n - s with v flipped verifies under plain ECDSA; it must not here.
        let mut j = pinned_sub(USD);
        let mut r = receipt(&key(1), &request(T1, 1));
        let sig = hex::decode(&r.2).unwrap();
        let n = hex::decode("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141")
            .unwrap();
        let mut s_hi = [0u8; 32];
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let d = n[i] as i16 - sig[32 + i] as i16 - borrow;
            borrow = if d < 0 { 1 } else { 0 };
            s_hi[i] = (d + 256 * borrow) as u8;
        }
        let mut bad = sig[..32].to_vec();
        bad.extend_from_slice(&s_hi);
        bad.push(if sig[64] == 27 { 28 } else { 27 });
        r.2 = hex::encode(bad);
        deliver_r(&mut j, r);
    }

    #[test]
    #[should_panic(expected = "receipt needs req, sig and model together")]
    fn partial_receipt_rejected() {
        let mut j = pinned_sub(USD);
        let r = receipt(&key(1), &request(T1, 1));
        Ctx::new(T1, WORKER).set();
        j.deliver(r.0, Some(r.1), None, Some(r.3));
    }

    #[test]
    #[should_panic(expected = "bytes, max 8192")]
    fn oversized_request_rejected() {
        let mut j = pinned_sub(USD);
        let mut req = request(T1, 1);
        req.extend(std::iter::repeat_n(b' ', 9000));
        deliver_r(&mut j, receipt(&key(1), &req));
    }

    #[test]
    #[should_panic(expected = "model name must be 1-128 bytes")]
    fn oversized_model_rejected() {
        let mut j = pinned_sub(USD);
        let mut r = receipt(&key(1), &request(T1, 1));
        r.3 = "m".repeat(129);
        deliver_r(&mut j, r);
    }

    #[test]
    #[should_panic(expected = "only the worker can deliver")]
    fn stranger_cannot_deliver_a_stolen_receipt() {
        let mut j = pinned_sub(USD);
        let r = receipt(&key(1), &request(T1, 1));
        Ctx::new(T1, EVE).set();
        j.deliver(r.0, Some(r.1), Some(r.2), Some(r.3));
    }
}

#[test]
#[should_panic(expected = "resp_hash must be 64 hex chars")]
fn bad_deliverable_hash_refused() {
    let mut j = funded_sub(USD, USD);
    Ctx::new(T1, WORKER).set();
    j.deliver("abc".into(), None, None, None);
}

#[test]
fn quote_note_is_none_without_verified_receipt() {
    let j = delivered_sub(30 * USD, 30 * USD);
    assert_eq!(j.quote_note(), None);
}

// ================================================================ approve

#[test]
fn client_approves_root() {
    let mut j = funded_root(USD);
    Ctx::new(ROOT, CLIENT).at(T0 + 10).set();
    j.approve();
    let v = j.get_job();
    assert!(v.approved);
    assert_eq!(v.window_end_ns.0, T0 + 10);
}

#[test]
#[should_panic(expected = "only the client (root) or manager (sub-job) can approve")]
fn lead_cannot_approve_root() {
    let mut j = funded_root(USD);
    Ctx::new(ROOT, LEAD).set();
    j.approve();
}

#[test]
fn manager_approves_sub() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, LEAD).set();
    j.approve();
    assert!(j.get_job().approved);
}

#[test]
#[should_panic(expected = "job cannot be approved in this status")]
fn manager_cannot_approve_sub_before_delivery() {
    let mut j = funded_sub(USD, USD);
    Ctx::new(T1, LEAD).set();
    j.approve();
}

#[test]
#[should_panic(expected = "already approved")]
fn double_approve_refused() {
    let mut j = funded_root(USD);
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    j.approve();
}

// ================================================================ settle

#[test]
#[should_panic(expected = "window is still open")]
fn settle_before_window_refused() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, EVE).set();
    j.settle();
}

#[test]
#[should_panic(expected = "children still open")]
fn settle_with_open_children_refused() {
    let mut j = funded_root(100 * USD);
    hired(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, EVE).at(AFTER).set();
    j.settle();
}

#[test]
#[should_panic(expected = "children still open")]
fn settle_with_hire_in_flight_refused() {
    let mut j = funded_root(100 * USD);
    hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, EVE).at(AFTER).set();
    j.settle();
}

#[test]
#[should_panic(expected = "not enough gas for settle")]
fn settle_with_low_gas_refused() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, EVE).at(AFTER).gas(Gas::from_tgas(50)).set();
    j.settle();
}

fn transfers() -> Vec<(String, String)> {
    let mut out = vec![];
    for r in get_created_receipts() {
        for a in r.actions {
            if let MockAction::FunctionCallWeight {
                method_name, args, ..
            } = a
            {
                if method_name == b"ft_transfer" {
                    let v: serde_json::Value = serde_json::from_slice(&args).unwrap();
                    out.push((
                        v["receiver_id"].as_str().unwrap().into(),
                        v["amount"].as_str().unwrap().into(),
                    ));
                }
            }
        }
    }
    out
}

fn attempt(j: &Job) -> u32 {
    j.payout.as_ref().expect("payout in flight").attempt
}

fn res(ok_: bool) -> PromiseResult {
    if ok_ {
        ok(b"")
    } else {
        PromiseResult::Failed
    }
}

/// Run the payee-leg callback with the given transfer outcome.
fn payee_cb(j: &mut Job, cur: &str, ok_: bool) -> PromiseOrValue<bool> {
    let a = attempt(j);
    Ctx::new(cur, cur).at(AFTER).results(vec![res(ok_)]).set();
    j.on_payee_paid(a)
}

fn rest_cb(j: &mut Job, cur: &str, ok_: bool) -> PromiseOrValue<bool> {
    let a = attempt(j);
    Ctx::new(cur, cur).at(AFTER).results(vec![res(ok_)]).set();
    j.on_rest_paid(a)
}

fn balance_cb(j: &mut Job, cur: &str, r: PromiseResult) -> PromiseOrValue<bool> {
    let a = attempt(j);
    Ctx::new(cur, cur).at(AFTER).results(vec![r]).set();
    j.on_balance_checked(a)
}

fn deleted() -> bool {
    get_created_receipts()
        .iter()
        .flat_map(|r| r.actions.iter())
        .any(|a| matches!(a, MockAction::DeleteAccount { .. }))
}

#[test]
fn settle_delivered_sub_registers_then_pays_worker_first() {
    let mut j = delivered_sub(30 * USD, 35 * USD);
    Ctx::new(T1, EVE).at(AFTER).set();
    j.settle();
    // One transfer at a time: only the payee leg is in flight.
    assert_eq!(transfers(), vec![(WORKER.into(), (30 * USD).to_string())]);
    let reg = call_args("storage_deposit");
    assert_eq!(reg["account_id"], WORKER);
    assert_eq!(reg["registration_only"], true);
    assert!(calls().contains(&(T1.into(), "on_payee_paid".into())));
    let v = j.get_job();
    assert_eq!(v.status, "Settling");
    assert_eq!(v.ft_balance.0, 5 * USD);
    assert_eq!(v.payout.unwrap().leg, Leg::Payee);
}

#[test]
fn payee_paid_then_rest_leg_returns_to_parent() {
    let mut j = settling_sub();
    assert!(matches!(
        payee_cb(&mut j, T1, true),
        PromiseOrValue::Promise(_)
    ));
    assert_eq!(transfers(), vec![(ROOT.into(), (5 * USD).to_string())]);
    assert!(calls().contains(&(T1.into(), "on_rest_paid".into())));
    let v = j.get_job();
    assert!(v.payee_paid);
    assert_eq!(v.ft_balance.0, 0);
    assert_eq!(v.payout.unwrap().leg, Leg::Rest);
}

#[test]
fn settle_undelivered_sub_returns_everything_to_parent() {
    let mut j = funded_sub(30 * USD, 30 * USD);
    Ctx::new(T1, EVE).at(AFTER).set();
    j.settle();
    assert_eq!(transfers(), vec![(ROOT.into(), (30 * USD).to_string())]);
    assert!(!calls().iter().any(|c| c.1 == "storage_deposit"));
}

#[test]
fn settle_root_sends_remainder_to_manager() {
    let mut j = funded_root(100 * USD);
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    Ctx::new(ROOT, EVE).set();
    j.settle();
    let args = call_args("ft_transfer");
    assert_eq!(args["receiver_id"], LEAD);
    assert_eq!(args["amount"], (100 * USD).to_string());
}

#[test]
fn settle_with_nothing_to_send_goes_to_balance_check() {
    let mut j = new_root();
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    Ctx::new(ROOT, EVE).set();
    j.settle();
    let c = calls();
    assert!(c.contains(&(TOKEN.into(), "ft_balance_of".into())));
    assert!(c.contains(&(ROOT.into(), "on_balance_checked".into())));
    assert!(!c.iter().any(|x| x.1 == "ft_transfer"));
}

#[test]
#[should_panic(expected = "illegal transition")]
fn double_settle_refused() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, EVE).at(AFTER).set();
    j.settle();
    j.settle();
}

fn settling_sub() -> Job {
    let mut j = delivered_sub(30 * USD, 35 * USD);
    Ctx::new(T1, EVE).at(AFTER).set();
    j.settle();
    j
}

#[test]
fn both_legs_ok_check_real_balance_before_delete() {
    let mut j = settling_sub();
    payee_cb(&mut j, T1, true);
    assert!(matches!(
        rest_cb(&mut j, T1, true),
        PromiseOrValue::Promise(_)
    ));
    let c = calls();
    assert!(c.contains(&(TOKEN.into(), "ft_balance_of".into())));
    assert!(c.contains(&(T1.into(), "on_balance_checked".into())));
    assert!(!deleted(), "nothing is deleted yet");
    let v = j.get_job();
    assert!(v.payee_paid);
    assert_eq!(v.returned_to_parent.0, 5 * USD);
    assert_eq!(v.status, "Settling");
}

#[test]
fn payee_failure_still_returns_rest_then_resettle_pays_only_payee() {
    let mut j = settling_sub();
    payee_cb(&mut j, T1, false);
    // The parent is not held up by the payee.
    assert_eq!(transfers(), vec![(ROOT.into(), (5 * USD).to_string())]);
    assert!(matches!(
        rest_cb(&mut j, T1, true),
        PromiseOrValue::Value(false)
    ));
    let v = j.get_job();
    assert_eq!(v.status, "Delivered");
    assert_eq!(v.ft_balance.0, 30 * USD);
    assert!(!v.payee_paid);
    assert!(v.payout.is_none());
    assert_eq!(v.returned_to_parent.0, 5 * USD);
    assert_eq!(events("settle_failed").len(), 1);
    // Settle again: only the payee transfer is left.
    Ctx::new(T1, EVE).at(AFTER).set();
    j.settle();
    assert_eq!(transfers(), vec![(WORKER.into(), (30 * USD).to_string())]);
    payee_cb(&mut j, T1, true);
    assert!(transfers().is_empty(), "no rest left to send");
    assert!(calls().contains(&(TOKEN.into(), "ft_balance_of".into())));
}

#[test]
fn rest_failure_never_pays_payee_twice() {
    let mut j = settling_sub();
    payee_cb(&mut j, T1, true);
    rest_cb(&mut j, T1, false);
    let v = j.get_job();
    assert_eq!(v.status, "Delivered");
    assert!(v.payee_paid);
    assert_eq!(v.ft_balance.0, 5 * USD);
    Ctx::new(T1, EVE).at(AFTER).set();
    j.settle();
    assert_eq!(transfers(), vec![(ROOT.into(), (5 * USD).to_string())]);
}

#[test]
#[should_panic(expected = "not settling")]
fn payee_callback_outside_settle_refused() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, T1).results(vec![ok(b"")]).set();
    j.on_payee_paid(1);
}

#[test]
#[should_panic(expected = "stale settle callback")]
fn wrong_leg_callback_refused() {
    let mut j = settling_sub();
    let a = attempt(&j);
    Ctx::new(T1, T1).at(AFTER).results(vec![ok(b"")]).set();
    j.on_rest_paid(a);
}

#[test]
#[should_panic(expected = "callback: caller must be this contract")]
fn leg_callbacks_reject_outside_callers() {
    let mut j = settling_sub();
    let a = attempt(&j);
    Ctx::new(T1, EVE).at(AFTER).results(vec![ok(b"")]).set();
    j.on_payee_paid(a);
}

fn paid_sub() -> Job {
    let mut j = settling_sub();
    payee_cb(&mut j, T1, true);
    rest_cb(&mut j, T1, true);
    j
}

#[test]
fn zero_real_balance_deletes_and_notifies_parent() {
    let mut j = paid_sub();
    assert!(matches!(
        balance_cb(&mut j, T1, ok(&u128_json(0))),
        PromiseOrValue::Promise(_)
    ));
    assert_eq!(j.get_job().status, "Settled");
    assert!(j.get_job().payout.is_none());
    let receipts = get_created_receipts();
    let del = receipts
        .iter()
        .find(|r| r.receiver_id == acc(T1))
        .and_then(|r| {
            r.actions.iter().find_map(|a| match a {
                MockAction::DeleteAccount { beneficiary_id, .. } => Some(beneficiary_id.clone()),
                _ => None,
            })
        });
    assert_eq!(del, Some(acc(ROOT)), "beneficiary is the parent");
    assert!(calls().contains(&(ROOT.into(), "child_closed".into())));
    assert_eq!(call_args("child_closed")["returned"], (5 * USD).to_string());
}

#[test]
fn root_delete_goes_to_factory_without_child_closed() {
    let mut j = funded_root(10 * USD);
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    Ctx::new(ROOT, EVE).set();
    j.settle();
    rest_cb(&mut j, ROOT, true);
    balance_cb(&mut j, ROOT, ok(&u128_json(0)));
    assert_eq!(j.get_job().status, "Settled");
    assert!(!calls().iter().any(|c| c.1 == "child_closed"));
    let del = get_created_receipts()
        .into_iter()
        .flat_map(|r| r.actions)
        .find_map(|a| match a {
            MockAction::DeleteAccount { beneficiary_id, .. } => Some(beneficiary_id),
            _ => None,
        });
    assert_eq!(del, Some(acc(FACTORY)));
}

#[test]
fn stray_tokens_block_delete_and_are_settled_next() {
    let mut j = paid_sub();
    assert!(matches!(
        balance_cb(&mut j, T1, ok(&u128_json(7))),
        PromiseOrValue::Value(false)
    ));
    let v = j.get_job();
    assert_eq!(v.status, "Delivered");
    assert_eq!(v.ft_balance.0, 7);
    assert!(get_created_receipts().is_empty(), "no delete");
    // Next settle sends the stray 7 to the parent; the payee is not paid twice.
    Ctx::new(T1, EVE).at(AFTER).set();
    j.settle();
    assert_eq!(transfers(), vec![(ROOT.into(), "7".into())]);
}

#[test]
fn failed_balance_check_blocks_delete() {
    let mut j = paid_sub();
    assert!(matches!(
        balance_cb(&mut j, T1, PromiseResult::Failed),
        PromiseOrValue::Value(false)
    ));
    assert_eq!(j.get_job().status, "Delivered");
    assert!(get_created_receipts().is_empty());
}

#[test]
#[should_panic(expected = "not settling")]
fn balance_check_outside_settle_refused() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, T1).results(vec![ok(&u128_json(0))]).set();
    j.on_balance_checked(1);
}

// ================================================================ resume_settle

const STUCK: u64 = AFTER + (RESUME_AFTER_SECS + 1) * NS_PER_SEC;

/// Resume a stuck settle and answer the balance query with `real`.
fn resume(j: &mut Job, cur: &str, real: PromiseResult) -> PromiseOrValue<bool> {
    let now = STUCK.max(j.payout.as_ref().unwrap().since_ns + (RESUME_AFTER_SECS + 1) * NS_PER_SEC);
    Ctx::new(cur, EVE).at(now).set();
    j.resume_settle();
    assert!(calls().contains(&(TOKEN.into(), "ft_balance_of".into())));
    let a = attempt(j);
    Ctx::new(cur, cur).at(now).results(vec![real]).set();
    j.on_resume_balance(a)
}

#[test]
#[should_panic(expected = "not stuck yet")]
fn resume_before_timeout_refused() {
    let mut j = settling_sub();
    Ctx::new(T1, EVE).at(AFTER + NS_PER_SEC).set();
    j.resume_settle();
}

#[test]
#[should_panic(expected = "not settling")]
fn resume_outside_settle_refused() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, EVE).at(STUCK).set();
    j.resume_settle();
}

#[test]
fn resume_payee_leg_that_happened_moves_on_without_paying_twice() {
    let mut j = settling_sub();
    // The payee callback never ran. Real balance 5 = ledger base: it was paid.
    assert!(matches!(
        resume(&mut j, T1, ok(&u128_json(5 * USD))),
        PromiseOrValue::Promise(_)
    ));
    let v = j.get_job();
    assert!(v.payee_paid);
    assert_eq!(transfers(), vec![(ROOT.into(), (5 * USD).to_string())]);
    assert_eq!(v.payout.unwrap().leg, Leg::Rest);
}

#[test]
fn resume_payee_leg_that_failed_keeps_payee_tokens() {
    let mut j = settling_sub();
    resume(&mut j, T1, ok(&u128_json(35 * USD)));
    let v = j.get_job();
    assert!(!v.payee_paid);
    assert_eq!(
        v.ft_balance.0,
        30 * USD,
        "the payee's 30 stay here; the rest leg is in flight"
    );
    // Rest leg still runs; afterwards the payee's 30 make settle fail and rerun.
    rest_cb(&mut j, T1, true);
    let v = j.get_job();
    assert_eq!(v.status, "Delivered");
    assert_eq!(v.ft_balance.0, 30 * USD);
}

#[test]
fn resume_counts_extra_tokens_as_stray() {
    let mut j = settling_sub();
    // 35 + 7 stray: the payee leg did not happen and 7 stray arrived.
    resume(&mut j, T1, ok(&u128_json(42 * USD)));
    rest_cb(&mut j, T1, true);
    let v = j.get_job();
    assert!(!v.payee_paid);
    assert_eq!(v.ft_balance.0, 37 * USD);
}

#[test]
#[should_panic(expected = "stale settle callback")]
fn late_original_callback_cannot_act_after_resume() {
    let mut j = settling_sub();
    let old = attempt(&j);
    resume(&mut j, T1, ok(&u128_json(5 * USD)));
    Ctx::new(T1, T1).at(STUCK).results(vec![ok(b"")]).set();
    j.on_payee_paid(old);
}

#[test]
fn resume_rest_leg_then_balance_leg_deletes() {
    let mut j = settling_sub();
    payee_cb(&mut j, T1, true);
    // Rest callback lost; real balance 0 means the rest went out.
    resume(&mut j, T1, ok(&u128_json(0)));
    assert_eq!(j.get_job().returned_to_parent.0, 5 * USD);
    assert_eq!(j.get_job().payout.unwrap().leg, Leg::Balance);
    // Balance callback lost too; resume finishes the delete.
    assert!(matches!(
        resume(&mut j, T1, ok(&u128_json(0))),
        PromiseOrValue::Promise(_)
    ));
    assert_eq!(j.get_job().status, "Settled");
    assert!(deleted());
}

#[test]
fn resume_with_no_token_answer_stays_stuck_and_can_retry() {
    let mut j = settling_sub();
    assert!(matches!(
        resume(&mut j, T1, PromiseResult::Failed),
        PromiseOrValue::Value(false)
    ));
    assert_eq!(j.get_job().status, "Settling");
    assert_eq!(events("settle_resume_failed").len(), 1);
}

// ================================================================ child_closed

#[test]
fn known_child_closes_and_returns_funds() {
    let mut j = funded_root(100 * USD);
    hired(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, T1).set();
    j.child_closed(U128(5 * USD));
    let v = j.get_job();
    assert_eq!(v.children_open, 0);
    assert_eq!(v.children[0].state, ChildState::Closed);
    assert_eq!(v.ft_balance.0, 75 * USD);
}

#[test]
#[should_panic(expected = "not an open child")]
fn unknown_caller_cannot_close_a_child() {
    let mut j = funded_root(100 * USD);
    hired(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, EVE).set();
    j.child_closed(U128(1_000 * USD));
}

#[test]
#[should_panic(expected = "not an open child")]
fn child_cannot_close_twice() {
    let mut j = funded_root(100 * USD);
    hired(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, T1).set();
    j.child_closed(U128(0));
    j.child_closed(U128(0));
}

#[test]
#[should_panic(expected = "not an open child")]
fn failed_child_cannot_close() {
    let mut j = funded_root(100 * USD);
    let idx = hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, ROOT)
        .results(vec![PromiseResult::Failed])
        .set();
    j.on_child_created(idx);
    Ctx::new(ROOT, T1).set();
    j.child_closed(U128(30 * USD));
}

// ================================================================ two-level fold

/// Root hires t1 (pay 30, sub-budget 10); the lead hires t1's own child
/// (pay 8). Everything delivers and settles bottom-up; the books close.
#[test]
fn two_level_tree_folds_bottom_up() {
    const GC: &str = "t1.t1.job-1.hiretree.test";
    let mut root = funded_root(100 * USD);
    Ctx::new(ROOT, LEAD).deposit(CHILD_BASE_NEAR).set();
    root.hire(acc(WORKER), U128(30 * USD), spec(), Some(U128(10 * USD)));
    Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
    root.on_child_created(0);
    Ctx::new(ROOT, ROOT)
        .results(vec![ok(&u128_json(40 * USD))])
        .set();
    assert!(root.on_child_funded(0));

    let mut t1 = funded_sub(30 * USD, 40 * USD);
    assert_eq!(t1.get_job().unallocated.0, 10 * USD);
    Ctx::new(T1, LEAD).deposit(CHILD_BASE_NEAR).set();
    t1.hire(acc(WORKER2), U128(8 * USD), spec(), None);
    Ctx::new(T1, T1).results(vec![ok(b"")]).set();
    t1.on_child_created(0);
    Ctx::new(T1, T1)
        .results(vec![ok(&u128_json(8 * USD))])
        .set();
    assert!(t1.on_child_funded(0));

    // Grandchild: delivers, settles: 8 to worker2, 0 back.
    Ctx::new(GC, T1).set();
    let mut gc = Job::new(
        None,
        acc(LEAD),
        Some(acc(WORKER2)),
        acc(TOKEN),
        U128(8 * USD),
        spec(),
        WINDOW,
        signer(),
        code_hash(),
        None,
        None,
    );
    Ctx::new(GC, TOKEN).set();
    gc.ft_on_transfer(acc(T1), U128(8 * USD), "fund".into());
    Ctx::new(GC, WORKER2).set();
    gc.deliver("01".repeat(32), None, None, None);
    Ctx::new(GC, EVE).at(AFTER).set();
    gc.settle();
    payee_cb(&mut gc, GC, true);
    balance_cb(&mut gc, GC, ok(&u128_json(0)));
    assert_eq!(gc.get_job().status, "Settled");
    Ctx::new(T1, GC).at(AFTER).set();
    t1.child_closed(U128(0));

    // t1's worker delivers inside the window; settle pays 30 and returns 2.
    Ctx::new(T1, WORKER).set();
    t1.deliver("02".repeat(32), None, None, None);
    Ctx::new(T1, EVE).at(AFTER).set();
    t1.settle();
    assert_eq!(t1.get_job().ft_balance.0, 2 * USD);
    payee_cb(&mut t1, T1, true);
    rest_cb(&mut t1, T1, true);
    balance_cb(&mut t1, T1, ok(&u128_json(0)));
    assert_eq!(t1.get_job().status, "Settled");
    Ctx::new(ROOT, T1).at(AFTER).set();
    root.child_closed(U128(2 * USD));

    // Root: 60 unspent + 2 returned = 62 to the lead.
    assert_eq!(root.get_job().ft_balance.0, 62 * USD);
    Ctx::new(ROOT, EVE).at(AFTER).set();
    root.settle();
    let args = call_args("ft_transfer");
    assert_eq!(args["receiver_id"], LEAD);
    assert_eq!(args["amount"], (62 * USD).to_string());
    // Money rule: 8 (worker2) + 30 (worker) + 62 (lead) == 100 funded.
}

// ================================================================ checks, notes, disputes

mod check_note_dispute {
    use super::receipt_tests::{addr_hex, deliver_r, key, receipt, request};
    use super::*;

    pub const CHECKER: &str = "checker.test";
    pub const TREASURY: &str = "treasury.lead.test";
    const PAY: u128 = 30 * USD;
    const QUOTE: u128 = 29_700_000; // 30 USD - 1%

    /// A funded sub-job with a pinned signer (key 1), checker and treasury.
    pub fn sub_full(checker: bool) -> Job {
        Ctx::new(T1, ROOT).set();
        let mut j = Job::new(
            None,
            acc(LEAD),
            Some(acc(WORKER)),
            acc(TOKEN),
            U128(PAY),
            spec(),
            WINDOW,
            addr_hex(&key(1)),
            code_hash(),
            checker.then(|| acc(CHECKER)),
            Some(acc(TREASURY)),
        );
        Ctx::new(T1, TOKEN).set();
        j.ft_on_transfer(acc(ROOT), U128(PAY), "fund".into());
        j
    }

    fn score(j: &mut Job, seq: u32, r: PromiseResult) {
        Ctx::new(T1, T1).results(vec![r]).set();
        j.on_check(seq);
    }

    fn num(n: u8) -> PromiseResult {
        ok(n.to_string().as_bytes())
    }

    /// Verified delivery, check passed with 8.
    pub fn checked() -> Job {
        let mut j = sub_full(true);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        score(&mut j, 1, num(8));
        assert_eq!(j.get_job().status, "Checked");
        j
    }

    fn buy(j: &mut Job, from: &str, amount: u128) -> PromiseOrValue<U128> {
        Ctx::new(T1, TOKEN).set();
        j.ft_on_transfer(acc(from), U128(amount), "buy_note".into())
    }

    fn note_paid(j: &mut Job, r: PromiseResult) -> U128 {
        Ctx::new(T1, T1).results(vec![r]).set();
        j.on_note_paid(acc(WORKER), U128(QUOTE))
    }

    // ------------------------------------------------ quality check

    #[test]
    fn deliver_schedules_exactly_one_check() {
        let mut j = sub_full(true);
        Ctx::new(T1, WORKER).set();
        j.deliver("cd".repeat(32), None, None, None);
        let c = calls();
        assert_eq!(c.iter().filter(|x| x.1 == "check").count(), 1);
        assert!(c.contains(&(CHECKER.into(), "check".into())));
        assert!(c.contains(&(T1.into(), "on_check".into())));
        let a = call_args("check");
        assert_eq!(a["job"], T1);
        assert_eq!(a["seq"], 1);
        assert_eq!(a["panel"], 0);
        assert_eq!(a["deliverable_hash"], "cd".repeat(32));
        assert_eq!(a["spec_hash"], spec());
        assert_eq!(a["seed"].as_str().unwrap().len(), 64);
        assert_eq!(j.get_job().status, "Delivered");
    }

    #[test]
    fn check_pass_marks_checked() {
        let j = checked();
        assert_eq!(j.get_job().quality, Some(8));
        assert_eq!(events("checked")[0]["score"], 8);
    }

    #[test]
    fn check_fail_reopens_for_redelivery() {
        let mut j = sub_full(true);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        score(&mut j, 1, num(3));
        let v = j.get_job();
        assert_eq!(v.status, "Open");
        assert_eq!(v.quality, Some(3));
        deliver_r(&mut j, receipt(&key(1), &request(T1, 2)));
        assert_eq!(j.get_job().seq, 2);
    }

    #[test]
    #[should_panic(expected = "delivery limit reached")]
    fn deliveries_are_capped() {
        let mut j = sub_full(true);
        for seq in 1..=MAX_DELIVERIES {
            Ctx::new(T1, WORKER).set();
            j.deliver("cd".repeat(32), None, None, None);
            score(&mut j, seq, num(1));
        }
        Ctx::new(T1, WORKER).set();
        j.deliver("cd".repeat(32), None, None, None);
    }

    #[test]
    fn check_failure_leaves_delivered() {
        let mut j = sub_full(true);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        score(&mut j, 1, PromiseResult::Failed);
        let v = j.get_job();
        assert_eq!(v.status, "Delivered");
        assert_eq!(v.quality, None);
        assert_eq!(events("check_unavailable").len(), 1);
        assert_eq!(j.quote_note(), None);
    }

    #[test]
    fn out_of_range_score_counts_as_no_answer() {
        let mut j = sub_full(true);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        score(&mut j, 1, num(11));
        assert_eq!(j.get_job().status, "Delivered");
        assert_eq!(j.get_job().quality, None);
    }

    #[test]
    #[should_panic(expected = "stale check")]
    fn stale_check_answer_refused() {
        let mut j = sub_full(true);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        score(&mut j, 2, num(9));
    }

    #[test]
    #[should_panic(expected = "callback: caller must be this contract")]
    fn check_callback_rejects_outside_callers() {
        let mut j = sub_full(true);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        Ctx::new(T1, WORKER).results(vec![num(10)]).set();
        j.on_check(1);
    }

    #[test]
    fn no_checker_verified_delivery_is_checked() {
        let mut j = sub_full(false);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        let v = j.get_job();
        assert_eq!(v.status, "Checked");
        assert_eq!(v.quality, None);
        assert!(!calls().iter().any(|c| c.1 == "check"));
    }

    #[test]
    fn no_checker_unverified_stays_delivered() {
        let mut j = sub_full(false);
        Ctx::new(T1, WORKER).set();
        j.deliver("cd".repeat(32), None, None, None);
        assert_eq!(j.get_job().status, "Delivered");
    }

    // ------------------------------------------------ job note

    #[test]
    fn quote_after_verified_and_checked() {
        assert_eq!(checked().quote_note(), Some(U128(QUOTE)));
    }

    #[test]
    fn quote_none_when_unchecked_or_unverified() {
        let mut j = sub_full(true);
        Ctx::new(T1, WORKER).set();
        j.deliver("cd".repeat(32), None, None, None);
        score(&mut j, 1, num(9));
        assert_eq!(j.get_job().status, "Checked");
        assert_eq!(j.quote_note(), None, "unverified");
        let mut j = sub_full(true);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        assert_eq!(j.quote_note(), None, "unchecked");
    }

    #[test]
    fn treasury_buys_note_and_worker_is_paid_now() {
        let mut j = checked();
        assert!(matches!(
            buy(&mut j, TREASURY, QUOTE),
            PromiseOrValue::Promise(_)
        ));
        let v = j.get_job();
        assert_eq!(v.payee, Some(acc(TREASURY)));
        assert!(v.note_sold);
        assert!(v.note_pending);
        assert_eq!(call_args("storage_deposit")["account_id"], WORKER);
        assert_eq!(transfers(), vec![(WORKER.into(), QUOTE.to_string())]);
        assert!(calls().contains(&(T1.into(), "on_note_paid".into())));
        assert_eq!(note_paid(&mut j, ok(b"")), U128(0));
        let v = j.get_job();
        assert!(!v.note_pending);
        assert_eq!(v.ft_balance.0, PAY, "tokens in == tokens out");
        assert_eq!(j.quote_note(), None);
        assert_eq!(events("note_sold").len(), 1);
    }

    #[test]
    fn bought_note_pays_treasury_at_settle() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        note_paid(&mut j, ok(b""));
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        assert_eq!(transfers(), vec![(TREASURY.into(), PAY.to_string())]);
    }

    #[test]
    fn stranger_cannot_buy_note() {
        let mut j = checked();
        assert_eq!(value(buy(&mut j, EVE, QUOTE)), QUOTE);
        assert!(!j.get_job().note_sold);
        assert_eq!(j.get_job().payee, Some(acc(WORKER)));
    }

    #[test]
    fn lead_itself_cannot_buy_only_its_treasury() {
        let mut j = checked();
        assert_eq!(value(buy(&mut j, LEAD, QUOTE)), QUOTE);
    }

    #[test]
    fn wrong_price_refunded() {
        let mut j = checked();
        assert_eq!(value(buy(&mut j, TREASURY, QUOTE - 1)), QUOTE - 1);
        assert_eq!(value(buy(&mut j, TREASURY, PAY)), PAY);
        assert!(!j.get_job().note_sold);
    }

    #[test]
    fn unchecked_note_refused() {
        let mut j = sub_full(true);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        assert_eq!(value(buy(&mut j, TREASURY, QUOTE)), QUOTE);
    }

    #[test]
    fn unverified_note_refused() {
        let mut j = sub_full(true);
        Ctx::new(T1, WORKER).set();
        j.deliver("cd".repeat(32), None, None, None);
        score(&mut j, 1, num(9));
        assert_eq!(value(buy(&mut j, TREASURY, QUOTE)), QUOTE);
    }

    #[test]
    fn failed_worker_payment_undoes_sale() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        assert_eq!(note_paid(&mut j, PromiseResult::Failed), U128(QUOTE));
        let v = j.get_job();
        assert!(!v.note_sold);
        assert!(!v.note_pending);
        assert_eq!(v.payee, Some(acc(WORKER)));
        assert_eq!(j.quote_note(), Some(U128(QUOTE)), "sellable again");
    }

    #[test]
    fn second_sale_refused() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        assert_eq!(value(buy(&mut j, TREASURY, QUOTE)), QUOTE, "in flight");
        note_paid(&mut j, ok(b""));
        assert_eq!(value(buy(&mut j, TREASURY, QUOTE)), QUOTE, "sold");
    }

    #[test]
    #[should_panic(expected = "note sale in flight")]
    fn settle_waits_for_note_sale() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
    }

    #[test]
    #[should_panic(expected = "callback: caller must be this contract")]
    fn note_callback_rejects_outside_callers() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        Ctx::new(T1, EVE).results(vec![ok(b"")]).set();
        j.on_note_paid(acc(WORKER), U128(QUOTE));
    }

    // ------------------------------------------------ dispute

    fn disputed() -> Job {
        let mut j = checked();
        Ctx::new(T1, LEAD).set();
        j.dispute();
        j
    }

    fn verdict(j: &mut Job, a: PromiseResult, b: PromiseResult) {
        let attempt = j.dispute_attempt;
        Ctx::new(T1, T1).results(vec![a, b]).set();
        j.on_dispute(attempt);
    }

    #[test]
    fn manager_disputes_sub_job() {
        let j = disputed();
        assert_eq!(j.get_job().status, "Disputed");
        let panels: Vec<u64> = get_created_receipts()
            .into_iter()
            .flat_map(|r| r.actions)
            .filter_map(|a| match a {
                MockAction::FunctionCallWeight {
                    method_name, args, ..
                } if method_name == b"check" => {
                    let v: serde_json::Value = serde_json::from_slice(&args).unwrap();
                    v["panel"].as_u64()
                }
                _ => None,
            })
            .collect();
        assert_eq!(panels, vec![1, 2]);
        assert!(calls().contains(&(T1.into(), "on_dispute".into())));
    }

    #[test]
    #[should_panic(expected = "only the manager can dispute")]
    fn stranger_cannot_dispute() {
        let mut j = checked();
        Ctx::new(T1, EVE).set();
        j.dispute();
    }

    #[test]
    #[should_panic(expected = "window is closed")]
    fn dispute_after_window_refused() {
        let mut j = checked();
        Ctx::new(T1, LEAD).at(AFTER).set();
        j.dispute();
    }

    #[test]
    #[should_panic(expected = "already disputed")]
    fn second_dispute_refused() {
        let mut j = disputed();
        verdict(&mut j, num(9), num(9));
        Ctx::new(T1, LEAD).set();
        j.dispute();
    }

    #[test]
    #[should_panic(expected = "root disputes are not built")]
    fn root_dispute_not_built() {
        let mut j = funded_root(USD);
        Ctx::new(ROOT, CLIENT).set();
        j.dispute();
    }

    #[test]
    fn dispute_upheld_refunds_parent_at_settle() {
        let mut j = disputed();
        verdict(&mut j, num(2), num(4));
        assert_eq!(j.get_job().status, "Refunded");
        assert_eq!(events("dispute_decided")[0]["upheld"], true);
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        assert_eq!(transfers(), vec![(ROOT.into(), PAY.to_string())]);
    }

    #[test]
    fn manager_can_close_a_refunded_sub_early() {
        let mut j = disputed();
        verdict(&mut j, num(2), num(4));
        Ctx::new(T1, LEAD).set();
        j.approve();
        Ctx::new(T1, EVE).set();
        j.settle();
        assert_eq!(transfers(), vec![(ROOT.into(), PAY.to_string())]);
    }

    #[test]
    fn dispute_rejected_back_to_checked() {
        let mut j = disputed();
        verdict(&mut j, num(2), num(9));
        assert_eq!(j.get_job().status, "Checked");
        assert_eq!(events("dispute_decided")[0]["upheld"], false);
    }

    #[test]
    fn missing_votes_never_uphold() {
        let mut j = disputed();
        verdict(&mut j, PromiseResult::Failed, num(1));
        assert_eq!(j.get_job().status, "Checked");
    }

    #[test]
    fn dispute_after_own_note_purchase_only_hurts_treasury() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        note_paid(&mut j, ok(b""));
        Ctx::new(T1, LEAD).set();
        j.dispute();
        verdict(&mut j, num(0), num(0));
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        // The worker keeps the 29.7 it was paid; the treasury gets nothing;
        // the pay returns up the tree.
        assert_eq!(transfers(), vec![(ROOT.into(), PAY.to_string())]);
    }

    #[test]
    #[should_panic(expected = "note sale in flight")]
    fn dispute_waits_for_note_sale() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        Ctx::new(T1, LEAD).set();
        j.dispute();
    }

    #[test]
    fn stuck_dispute_resumes_as_rejected() {
        let mut j = disputed();
        let old = j.dispute_attempt;
        Ctx::new(T1, EVE)
            .at(T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC)
            .set();
        j.resume_dispute();
        assert_eq!(j.get_job().status, "Checked");
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Ctx::new(T1, T1).results(vec![num(0), num(0)]).set();
            j.on_dispute(old);
        }));
        assert!(r.is_err(), "late verdict refused");
    }

    #[test]
    #[should_panic(expected = "dispute is not stuck yet")]
    fn resume_dispute_too_early() {
        let mut j = disputed();
        Ctx::new(T1, EVE).set();
        j.resume_dispute();
    }

    #[test]
    fn hire_passes_checker_and_treasury_down() {
        Ctx::new(ROOT, FACTORY).set();
        let mut r = Job::new(
            Some(acc(CLIENT)),
            acc(LEAD),
            None,
            acc(TOKEN),
            U128(0),
            spec(),
            WINDOW,
            signer(),
            code_hash(),
            Some(acc(CHECKER)),
            Some(acc(TREASURY)),
        );
        Ctx::new(ROOT, TOKEN).set();
        r.ft_on_transfer(acc(CLIENT), U128(100 * USD), "fund".into());
        Ctx::new(ROOT, LEAD).deposit(CHILD_BASE_NEAR).set();
        r.hire(acc(WORKER), U128(USD), spec(), None);
        let a = call_args("new");
        assert_eq!(a["checker"], CHECKER);
        assert_eq!(a["treasury"], TREASURY);
    }
}
