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
        None,
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
        None,
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
    Ctx::new(ROOT, LEAD).deposit(HIRE_MIN_NEAR).set();
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
    j.deliver("cd".repeat(32), None, None, None, None);
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
        None,
        None,
        None,
    );
}

fn root_with_fee(fee: u128) -> Job {
    Ctx::new(ROOT, FACTORY).set();
    Job::new(
        Some(acc(CLIENT)),
        acc(LEAD),
        None,
        acc(TOKEN),
        U128(fee),
        spec(),
        WINDOW,
        signer(),
        code_hash(),
        None,
        None,
        None,
        None,
        None,
    )
}

/// A root with this fee, funded with `budget` by the client.
fn root_funded_with_fee(fee: u128, budget: u128) -> Job {
    let mut j = root_with_fee(fee);
    Ctx::new(ROOT, TOKEN).set();
    j.ft_on_transfer(acc(CLIENT), U128(budget), "fund".into());
    j
}

/// Child `n` (t{n}) tells the root it closed, sending back `returned`.
fn root_child_closed(j: &mut Job, n: u32, returned: u128, paid: bool) {
    let child = format!("t{n}.{ROOT}");
    Ctx::new(ROOT, &child).set();
    j.child_closed(U128(returned), Some(paid), None);
}

/// The client approves the root and anyone settles it.
fn settle_root(j: &mut Job) {
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    Ctx::new(ROOT, EVE).set();
    j.settle();
}

#[test]
fn root_fee_and_bond_go_to_the_manager_the_rest_back_to_the_client() {
    let mut j = root_funded_with_fee(10 * USD, 100 * USD);
    Ctx::new(ROOT, TOKEN).set();
    j.ft_on_transfer(acc(LEAD), U128(5 * USD), "bond".into());
    assert_eq!(j.get_job().unallocated.0, 90 * USD, "fee is never hireable");
    // Its one part was paid: the whole fee is earned.
    hired(&mut j, WORKER, 30 * USD);
    root_child_closed(&mut j, 1, 0, true);
    settle_root(&mut j);
    assert_eq!(call_args("storage_deposit")["account_id"], LEAD);
    assert_eq!(transfers(), vec![(LEAD.into(), (15 * USD).to_string())]);
    payee_cb(&mut j, ROOT, true);
    assert_eq!(transfers(), vec![(CLIENT.into(), (60 * USD).to_string())]);
}

#[test]
fn the_lead_fee_is_earned_pro_rata_to_the_paid_parts() {
    let mut j = root_funded_with_fee(10 * USD, 100 * USD);
    hired(&mut j, WORKER, 30 * USD);
    hired(&mut j, WORKER2, 10 * USD);
    root_child_closed(&mut j, 1, 0, true);
    root_child_closed(&mut j, 2, 10 * USD, false);
    settle_root(&mut j);
    // 10 * 30 / 40 = 7.5 to the lead; the other 2.5 goes back.
    assert_eq!(transfers(), vec![(LEAD.into(), 7_500_000.to_string())]);
    let e = &events("lead_fee")[0];
    assert_eq!(e["fee"], (10 * USD).to_string());
    assert_eq!(e["earned"], 7_500_000.to_string());
    assert_eq!(e["paid"], (30 * USD).to_string());
    assert_eq!(e["hired"], (40 * USD).to_string());
    payee_cb(&mut j, ROOT, true);
    assert_eq!(transfers(), vec![(CLIENT.into(), 62_500_000.to_string())]);
}

#[test]
fn the_earned_fee_rounds_down_in_the_clients_favour() {
    let mut j = root_funded_with_fee(10 * USD, 100 * USD);
    for (n, w) in [WORKER, WORKER2, EVE].iter().enumerate() {
        hired(&mut j, w, 10 * USD);
        root_child_closed(
            &mut j,
            n as u32 + 1,
            if n == 0 { 0 } else { 10 * USD },
            n == 0,
        );
    }
    settle_root(&mut j);
    assert_eq!(transfers(), vec![(LEAD.into(), 3_333_333.to_string())]);
    payee_cb(&mut j, ROOT, true);
    assert_eq!(transfers(), vec![(CLIENT.into(), 86_666_667.to_string())]);
}

#[test]
fn a_lead_whose_every_part_failed_earns_no_fee() {
    let mut j = root_funded_with_fee(10 * USD, 100 * USD);
    Ctx::new(ROOT, TOKEN).set();
    j.ft_on_transfer(acc(LEAD), U128(5 * USD), "bond".into());
    hired(&mut j, WORKER, 30 * USD);
    hired(&mut j, WORKER2, 20 * USD);
    // Fired and refunded: all of it came back.
    root_child_closed(&mut j, 1, 30 * USD, false);
    root_child_closed(&mut j, 2, 20 * USD, false);
    settle_root(&mut j);
    // Only the bond goes back to the lead; the whole fee to the client.
    assert_eq!(transfers(), vec![(LEAD.into(), (5 * USD).to_string())]);
    assert_eq!(events("lead_fee")[0]["earned"], "0");
    payee_cb(&mut j, ROOT, true);
    assert_eq!(transfers(), vec![(CLIENT.into(), (100 * USD).to_string())]);
}

#[test]
fn a_root_without_children_pays_no_lead_fee() {
    let mut j = root_funded_with_fee(10 * USD, 100 * USD);
    settle_root(&mut j);
    // No payee leg at all: the client is registered and gets everything.
    assert_eq!(call_args("storage_deposit")["account_id"], CLIENT);
    assert_eq!(transfers(), vec![(CLIENT.into(), (100 * USD).to_string())]);
    assert_eq!(events("lead_fee")[0]["hired"], "0");
}

#[test]
fn a_rolled_back_hire_does_not_count_for_the_fee() {
    let mut j = root_funded_with_fee(10 * USD, 100 * USD);
    let idx = hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, ROOT)
        .results(vec![PromiseResult::Failed])
        .set();
    j.on_child_created(idx);
    assert_eq!(j.get_job().children[0].state, ChildState::Failed);
    hired(&mut j, WORKER2, 20 * USD);
    root_child_closed(&mut j, 2, 0, true);
    settle_root(&mut j);
    assert_eq!(transfers(), vec![(LEAD.into(), (10 * USD).to_string())]);
    payee_cb(&mut j, ROOT, true);
    assert_eq!(transfers(), vec![(CLIENT.into(), (70 * USD).to_string())]);
}

#[test]
fn the_view_shows_the_lead_fee_earned_so_far() {
    let mut j = root_funded_with_fee(10 * USD, 100 * USD);
    assert_eq!(j.get_job().lead_fee_earned, Some(U128(0)));
    hired(&mut j, WORKER, 30 * USD);
    assert_eq!(j.get_job().lead_fee_earned, Some(U128(0)), "still open");
    root_child_closed(&mut j, 1, 0, true);
    let v = j.get_job();
    assert_eq!(v.lead_fee_earned, Some(U128(10 * USD)));
    assert_eq!(v.children[0].paid, Some(true));
    hired(&mut j, WORKER2, 10 * USD);
    root_child_closed(&mut j, 2, 10 * USD, false);
    let v = j.get_job();
    assert_eq!(v.lead_fee_earned, Some(U128(7_500_000)));
    assert_eq!(v.children[1].paid, Some(false));
    assert_eq!(funded_sub(USD, USD).get_job().lead_fee_earned, None);
}

#[test]
fn root_funding_below_the_fee_refunded() {
    let mut j = root_with_fee(10 * USD);
    Ctx::new(ROOT, TOKEN).set();
    assert!(matches!(
        j.ft_on_transfer(acc(CLIENT), U128(5 * USD), "fund".into()),
        PromiseOrValue::Value(U128(x)) if x == 5 * USD
    ));
    assert_eq!(j.get_job().funded.0, 0);
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
        None,
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
        None,
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
        None,
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
        None,
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
                // 50 attached, the job keeps 5 for registration + its record.
                assert_eq!(*deposit, NearToken::from_millinear(45));
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
#[should_panic(expected = "attach at least 0.035 NEAR")]
fn hire_without_storage_deposit_refused() {
    let mut j = funded_root(100 * USD);
    Ctx::new(ROOT, LEAD).set();
    j.hire(acc(WORKER), U128(USD), spec(), None);
}

#[test]
#[should_panic(expected = "only the manager or this job's worker can hire")]
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
    Ctx::new(ROOT, LEAD).deposit(HIRE_MIN_NEAR).set();
    j.hire(acc(WORKER), U128(u128::MAX), spec(), Some(U128(1)));
}

#[test]
#[should_panic(expected = "not enough gas for hire")]
fn hire_with_low_gas_refused() {
    let mut j = funded_root(10 * USD);
    Ctx::new(ROOT, LEAD)
        .gas(Gas::from_tgas(100))
        .deposit(HIRE_MIN_NEAR)
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
fn fund_failure_waits_then_resume_rolls_back_and_closes_child() {
    // Review F8: a failed ft_transfer_call is ambiguous (the token's resolve
    // step may have failed after moving the tokens), so nothing is undone
    // until resume_hire reads the child's real balance.
    let mut j = funded_root(100 * USD);
    let idx = hire_one(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
    j.on_child_created(idx);
    Ctx::new(ROOT, ROOT)
        .results(vec![PromiseResult::Failed])
        .set();
    assert!(!j.on_child_funded(idx));
    let v = j.get_job();
    assert_eq!(v.children_pending, 1, "still in flight");
    assert_eq!(v.allocated.0, 30 * USD);
    stuck_hire::resume_hire_with(&mut j, idx, ok(&u128_json(0)));
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
    j.deliver("cd".repeat(32), None, None, None, None);
}

#[test]
#[should_panic(expected = "only the worker can deliver")]
fn root_has_no_worker_to_deliver() {
    let mut j = funded_root(USD);
    Ctx::new(ROOT, LEAD).set();
    j.deliver("cd".repeat(32), None, None, None, None);
}

#[test]
#[should_panic(expected = "window is closed")]
fn deliver_after_window_refused() {
    let mut j = funded_sub(USD, USD);
    Ctx::new(T1, WORKER).at(AFTER).set();
    j.deliver("cd".repeat(32), None, None, None, None);
}

#[test]
#[should_panic(expected = "illegal transition")]
fn second_delivery_refused_while_delivered() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, WORKER).set();
    j.deliver("ef".repeat(32), None, None, None, None);
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
            None,
            None,
            None,
        );
        Ctx::new(T1, TOKEN).set();
        j.ft_on_transfer(acc(ROOT), U128(pay), "fund".into());
        j
    }

    pub const RESP2: &[u8] = b"{\"choices\":[{\"message\":{\"content\":\"hola!\"}}]}";

    /// A receipt over any response bytes.
    pub fn receipt_for(
        k: &SecretKey,
        req: &[u8],
        resp: &[u8],
    ) -> (String, Base64VecU8, String, String) {
        let text = format!("{MODEL}:{}:{}", sha256_hex(req), sha256_hex(resp));
        (
            hex::encode(sha256(resp)),
            Base64VecU8(req.to_vec()),
            sign(k, &text),
            MODEL.to_string(),
        )
    }

    pub fn deliver_with(j: &mut Job, r: (String, Base64VecU8, String, String), resp: &[u8]) {
        Ctx::new(T1, WORKER).set();
        j.deliver(
            r.0,
            Some(r.1),
            Some(r.2),
            Some(r.3),
            Some(Base64VecU8(resp.to_vec())),
        );
    }

    pub fn deliver_r(j: &mut Job, r: (String, Base64VecU8, String, String)) {
        Ctx::new(T1, WORKER).set();
        j.deliver(
            r.0,
            Some(r.1),
            Some(r.2),
            Some(r.3),
            Some(Base64VecU8(RESPONSE.to_vec())),
        );
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
        let junk: &[u8] = b"junk the worker wants paid for";
        let mut r = receipt(&key(1), &request(T1, 1));
        r.0 = hex::encode(sha256(junk));
        deliver_with(&mut j, r, junk);
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
        // The check sends it back to Open; the worker replays call #1's
        // tag on a genuinely signed new answer.
        j.status = Status::Open;
        deliver_with(&mut j, receipt_for(&key(1), &request(T1, 1), RESP2), RESP2);
    }

    #[test]
    fn fresh_receipt_on_redelivery_accepted() {
        let mut j = pinned_sub(USD);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        j.status = Status::Open;
        deliver_with(&mut j, receipt_for(&key(1), &request(T1, 2), RESP2), RESP2);
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
        j.deliver(r.0, Some(r.1), None, Some(r.3), None);
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
        j.deliver(r.0, Some(r.1), Some(r.2), Some(r.3), None);
    }
}

#[test]
#[should_panic(expected = "resp_hash must be 64 hex chars")]
fn bad_deliverable_hash_refused() {
    let mut j = funded_sub(USD, USD);
    Ctx::new(T1, WORKER).set();
    j.deliver("abc".into(), None, None, None, None);
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
fn settle_root_sends_remainder_back_to_client() {
    let mut j = funded_root(100 * USD);
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    Ctx::new(ROOT, EVE).set();
    j.settle();
    let args = call_args("ft_transfer");
    assert_eq!(args["receiver_id"], CLIENT);
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

fn delete_to() -> Option<AccountId> {
    get_created_receipts()
        .into_iter()
        .flat_map(|r| r.actions)
        .find_map(|a| match a {
            MockAction::DeleteAccount { beneficiary_id, .. } => Some(beneficiary_id),
            _ => None,
        })
}

fn close_attempt(j: &Job) -> u32 {
    j.get_job()
        .close_pending
        .expect("parent notice pending")
        .attempt
}

/// The parent's answer to `child_closed` reaches the child's callback.
fn ack(j: &mut Job, ok_: bool) -> PromiseOrValue<bool> {
    let a = close_attempt(j);
    ack_at(j, a, ok_)
}

fn ack_at(j: &mut Job, a: u32, ok_: bool) -> PromiseOrValue<bool> {
    Ctx::new(T1, T1).at(AFTER).results(vec![res(ok_)]).set();
    j.on_parent_ack(a)
}

/// A settled sub-job has told its parent and waits for the answer: nothing
/// is deleted yet. On a successful answer it deletes itself to the parent.
fn deletes_after_parent_ack(j: &mut Job) -> bool {
    assert!(!deleted(), "no delete before the parent's answer");
    assert!(calls().contains(&(ROOT.into(), "child_closed".into())));
    assert!(matches!(ack(j, true), PromiseOrValue::Promise(_)));
    delete_to() == Some(acc(ROOT))
}

#[test]
fn zero_real_balance_notifies_the_parent_then_deletes_on_its_answer() {
    let mut j = paid_sub();
    assert!(matches!(
        balance_cb(&mut j, T1, ok(&u128_json(0))),
        PromiseOrValue::Promise(_)
    ));
    assert_eq!(j.get_job().status, "Settled");
    assert!(j.get_job().payout.is_none());
    // The notice and its callback are one chain; the delete waits for it.
    let c = calls();
    assert!(c.contains(&(ROOT.into(), "child_closed".into())));
    assert!(c.contains(&(T1.into(), "on_parent_ack".into())));
    assert_eq!(call_args("child_closed")["returned"], (5 * USD).to_string());
    assert!(!deleted(), "the child stays until the parent has answered");
    let cp = j.get_job().close_pending.unwrap();
    assert!(cp.in_flight);
    // The parent answered: delete, beneficiary the parent.
    assert!(matches!(ack(&mut j, true), PromiseOrValue::Promise(_)));
    assert_eq!(delete_to(), Some(acc(ROOT)), "beneficiary is the parent");
}

#[test]
fn a_failed_parent_notice_keeps_the_child_alive_until_resume_close() {
    let mut j = paid_sub();
    balance_cb(&mut j, T1, ok(&u128_json(0)));
    // child_closed ran out of gas on the parent. The resend carries the
    // caller's spare gas (`a_resent_notice_gives_child_closed_the_callers_
    // spare_gas`), so this time it can succeed.
    assert!(matches!(ack(&mut j, false), PromiseOrValue::Value(false)));
    assert!(
        !deleted(),
        "never delete without the parent's acknowledgement"
    );
    let v = j.get_job();
    assert_eq!(v.status, "Settled");
    let cp = v.close_pending.unwrap();
    assert!(!cp.in_flight);
    assert_eq!(events("close_pending").len(), 1);
    // Anyone may send the notice again at once; it is the same notice.
    let first = cp.attempt;
    Ctx::new(T1, EVE).at(AFTER).gas(GAS_RESUME_CLOSE_MIN).set();
    j.resume_close();
    let c = calls();
    assert!(c.contains(&(ROOT.into(), "child_closed".into())));
    assert!(c.contains(&(T1.into(), "on_parent_ack".into())));
    assert!(!deleted());
    let args = call_args("child_closed");
    assert_eq!(args["returned"], (5 * USD).to_string());
    assert_eq!(args["paid"], true);
    assert!(!calls().iter().any(|x| x.1 == "record"), "record sent once");
    assert!(close_attempt(&j) > first);
    // This time the parent answers: the child deletes to it.
    assert!(matches!(ack(&mut j, true), PromiseOrValue::Promise(_)));
    assert_eq!(delete_to(), Some(acc(ROOT)));
}

#[test]
#[should_panic(expected = "parent notice in flight")]
fn resume_close_waits_while_the_notice_is_in_flight() {
    let mut j = paid_sub();
    balance_cb(&mut j, T1, ok(&u128_json(0)));
    Ctx::new(T1, EVE).at(AFTER + NS_PER_SEC).set();
    j.resume_close();
}

#[test]
fn resume_close_resends_a_notice_whose_answer_was_lost() {
    // The callback itself never ran (out of gas): after RESUME_AFTER_SECS
    // the notice may be sent again; the parent ignores a repeat.
    let mut j = paid_sub();
    balance_cb(&mut j, T1, ok(&u128_json(0)));
    let since = j.get_job().close_pending.unwrap().since_ns;
    Ctx::new(T1, EVE)
        .at(since + RESUME_AFTER_SECS * NS_PER_SEC)
        .set();
    j.resume_close();
    assert!(calls().contains(&(ROOT.into(), "child_closed".into())));
    assert!(j.get_job().close_pending.unwrap().in_flight);
}

#[test]
#[should_panic(expected = "no parent notice pending")]
fn resume_close_only_for_a_settled_child_with_a_pending_notice() {
    let mut j = funded_root(10 * USD);
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    Ctx::new(ROOT, EVE).set();
    j.settle();
    rest_cb(&mut j, ROOT, true);
    balance_cb(&mut j, ROOT, ok(&u128_json(0)));
    assert!(j.get_job().close_pending.is_none(), "a root tells nobody");
    Ctx::new(ROOT, EVE).at(STUCK).set();
    j.resume_close();
}

#[test]
#[should_panic(expected = "not settled")]
fn resume_close_refused_before_settle() {
    let mut j = delivered_sub(USD, USD);
    Ctx::new(T1, EVE).at(STUCK).set();
    j.resume_close();
}

#[test]
#[should_panic(expected = "not enough gas for resume_close")]
fn resume_close_needs_gas() {
    let mut j = paid_sub();
    balance_cb(&mut j, T1, ok(&u128_json(0)));
    ack(&mut j, false);
    Ctx::new(T1, EVE).at(AFTER).gas(Gas::from_tgas(10)).set();
    j.resume_close();
}

#[test]
fn a_late_failed_answer_cannot_stall_a_newer_notice() {
    let mut j = paid_sub();
    balance_cb(&mut j, T1, ok(&u128_json(0)));
    let first = close_attempt(&j);
    let since = j.get_job().close_pending.unwrap().since_ns;
    Ctx::new(T1, EVE)
        .at(since + RESUME_AFTER_SECS * NS_PER_SEC)
        .set();
    j.resume_close();
    // The first notice's failure arrives late: it changes nothing.
    assert!(matches!(
        ack_at(&mut j, first, false),
        PromiseOrValue::Value(false)
    ));
    assert!(j.get_job().close_pending.unwrap().in_flight);
    // Any success, even a late one, means the parent has us closed.
    assert!(matches!(
        ack_at(&mut j, first, true),
        PromiseOrValue::Promise(_)
    ));
    assert_eq!(delete_to(), Some(acc(ROOT)));
}

#[test]
#[should_panic(expected = "callback: caller must be this contract")]
fn parent_ack_rejects_outside_callers() {
    let mut j = paid_sub();
    balance_cb(&mut j, T1, ok(&u128_json(0)));
    let a = close_attempt(&j);
    Ctx::new(T1, EVE).at(AFTER).results(vec![ok(b"")]).set();
    j.on_parent_ack(a);
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

fn sweep_cb(j: &mut Job, ok_: bool) -> PromiseOrValue<bool> {
    let a = attempt(j);
    Ctx::new(T1, T1)
        .at(AFTER)
        .results(vec![if ok_ { ok(b"") } else { PromiseResult::Failed }])
        .set();
    j.on_swept(a)
}

#[test]
fn stray_tokens_are_swept_up_then_the_job_deletes() {
    let mut j = paid_sub();
    // 7 stray tokens: never booked, sent to the parent instead of deleting.
    assert!(matches!(
        balance_cb(&mut j, T1, ok(&u128_json(7))),
        PromiseOrValue::Promise(_)
    ));
    assert_eq!(transfers(), vec![(ROOT.into(), "7".into())]);
    assert!(!deleted());
    assert_eq!(j.get_job().ft_balance.0, 0, "the surplus is never booked");
    assert_eq!(j.get_job().payout.unwrap().leg, Leg::Sweep);
    sweep_cb(&mut j, true);
    assert_eq!(j.get_job().returned_to_parent.0, 5 * USD + 7);
    assert!(matches!(
        balance_cb(&mut j, T1, ok(&u128_json(0))),
        PromiseOrValue::Promise(_)
    ));
    assert!(deletes_after_parent_ack(&mut j));
}

#[test]
fn phantom_surplus_from_a_refunded_transfer_call_cannot_lock_settle() {
    // Review F1: a refused ft_transfer_call in flight shows up in the
    // balance read, then is refunded. The sweep fails; nothing was booked,
    // so the next settle goes straight through.
    let mut j = paid_sub();
    balance_cb(&mut j, T1, ok(&u128_json(1_000)));
    sweep_cb(&mut j, false);
    let v = j.get_job();
    assert_eq!(v.status, "Delivered");
    assert_eq!(v.ft_balance.0, 0);
    assert!(v.payee_paid);
    Ctx::new(T1, EVE).at(AFTER).set();
    j.settle();
    assert!(transfers().is_empty(), "nothing left to pay");
    assert!(matches!(
        balance_cb(&mut j, T1, ok(&u128_json(0))),
        PromiseOrValue::Promise(_)
    ));
    assert!(deletes_after_parent_ack(&mut j));
}

#[test]
#[should_panic(expected = "only the manager may resume in the first day")]
fn stranger_cannot_resume_in_the_first_day() {
    let mut j = settling_sub();
    Ctx::new(T1, EVE).at(STUCK).set();
    j.resume_settle();
}

#[test]
fn anyone_can_resume_after_a_day() {
    let mut j = settling_sub();
    let since = j.payout.as_ref().unwrap().since_ns;
    Ctx::new(T1, EVE)
        .at(since + RESUME_ANYONE_SECS * NS_PER_SEC)
        .set();
    j.resume_settle();
    assert!(calls().contains(&(TOKEN.into(), "ft_balance_of".into())));
}

#[test]
fn root_registers_the_client_before_refunding_it() {
    let mut j = funded_root(10 * USD);
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    Ctx::new(ROOT, EVE).set();
    j.settle();
    assert_eq!(call_args("storage_deposit")["account_id"], CLIENT);
    assert_eq!(transfers(), vec![(CLIENT.into(), (10 * USD).to_string())]);
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
    Ctx::new(cur, LEAD).at(now).set();
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
fn resume_never_books_extra_tokens() {
    let mut j = settling_sub();
    // 35 + 7 extra: the payee leg did not happen; the 7 are not booked
    // (a later balance check sweeps them, if they are real).
    resume(&mut j, T1, ok(&u128_json(42 * USD)));
    rest_cb(&mut j, T1, true);
    let v = j.get_job();
    assert!(!v.payee_paid);
    assert_eq!(v.ft_balance.0, 30 * USD);
}

#[test]
fn resume_of_a_lost_sweep_restarts_settle() {
    let mut j = paid_sub();
    balance_cb(&mut j, T1, ok(&u128_json(7)));
    resume(&mut j, T1, ok(&u128_json(7)));
    let v = j.get_job();
    assert_eq!(v.status, "Delivered");
    assert_eq!(v.ft_balance.0, 0);
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
    assert!(deletes_after_parent_ack(&mut j));
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
    j.child_closed(U128(5 * USD), None, None);
    let v = j.get_job();
    assert_eq!(v.children_open, 0);
    assert_eq!(v.children[0].state, ChildState::Closed);
    assert_eq!(v.ft_balance.0, 75 * USD);
}

#[test]
fn a_fired_childs_returned_pay_can_hire_the_runner_up() {
    // The whole budget went to one child; it was fired and sent all 100
    // back, so the manager can hire the runner-up with it.
    let mut j = funded_root(100 * USD);
    hired(&mut j, WORKER, 100 * USD);
    assert_eq!(j.get_job().unallocated.0, 0);
    Ctx::new(ROOT, T1).set();
    j.child_closed(U128(100 * USD), Some(false), None);
    assert_eq!(j.get_job().unallocated.0, 100 * USD);
    hired(&mut j, WORKER2, 90 * USD);
    // A child that paid its worker keeps that much allocated.
    let v = j.get_job();
    assert_eq!((v.allocated.0, v.unallocated.0), (90 * USD, 10 * USD));
}

#[test]
#[should_panic(expected = "not an open child")]
fn unknown_caller_cannot_close_a_child() {
    let mut j = funded_root(100 * USD);
    hired(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, EVE).set();
    j.child_closed(U128(1_000 * USD), None, None);
}

#[test]
fn a_repeated_child_closed_credits_nothing() {
    // The child resends its notice when it never saw the parent's answer;
    // the repeat succeeds (so the child can delete) and credits nothing.
    let mut j = funded_root(100 * USD);
    hired(&mut j, WORKER, 30 * USD);
    Ctx::new(ROOT, T1).set();
    j.child_closed(U128(5 * USD), Some(true), None);
    let once = j.get_job();
    assert_eq!(once.ft_balance.0, 75 * USD);
    Ctx::new(ROOT, T1).set();
    j.child_closed(U128(5 * USD), Some(true), None);
    let twice = j.get_job();
    assert_eq!(twice.ft_balance.0, 75 * USD, "not credited twice");
    assert_eq!(twice.children_open, 0);
    assert_eq!(twice.allocated, once.allocated);
    assert_eq!(twice.unallocated, once.unallocated);
    assert_eq!(events("child_closed_again").len(), 1);
    assert!(get_created_receipts().is_empty(), "no bond paid twice");
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
    j.child_closed(U128(30 * USD), None, None);
}

// ================================================================ two-level fold

/// Root hires t1 (pay 30, sub-budget 10); the lead hires t1's own child
/// (pay 8). Everything delivers and settles bottom-up; the books close.
#[test]
fn two_level_tree_folds_bottom_up() {
    const GC: &str = "t1.t1.job-1.hiretree.test";
    let mut root = funded_root(100 * USD);
    Ctx::new(ROOT, LEAD).deposit(HIRE_MIN_NEAR).set();
    root.hire(acc(WORKER), U128(30 * USD), spec(), Some(U128(10 * USD)));
    Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
    root.on_child_created(0);
    Ctx::new(ROOT, ROOT)
        .results(vec![ok(&u128_json(40 * USD))])
        .set();
    assert!(root.on_child_funded(0));

    let mut t1 = funded_sub(30 * USD, 40 * USD);
    assert_eq!(t1.get_job().unallocated.0, 10 * USD);
    Ctx::new(T1, LEAD).deposit(HIRE_MIN_NEAR).set();
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
        None,
        None,
        None,
    );
    Ctx::new(GC, TOKEN).set();
    gc.ft_on_transfer(acc(T1), U128(8 * USD), "fund".into());
    Ctx::new(GC, WORKER2).set();
    gc.deliver("01".repeat(32), None, None, None, None);
    Ctx::new(GC, EVE).at(AFTER).set();
    gc.settle();
    payee_cb(&mut gc, GC, true);
    balance_cb(&mut gc, GC, ok(&u128_json(0)));
    assert_eq!(gc.get_job().status, "Settled");
    Ctx::new(T1, GC).at(AFTER).set();
    t1.child_closed(U128(0), None, None);

    // t1's worker delivers inside the window; settle pays 30 and returns 2.
    Ctx::new(T1, WORKER).set();
    t1.deliver("02".repeat(32), None, None, None, None);
    Ctx::new(T1, EVE).at(AFTER).set();
    t1.settle();
    assert_eq!(t1.get_job().ft_balance.0, 2 * USD);
    payee_cb(&mut t1, T1, true);
    rest_cb(&mut t1, T1, true);
    balance_cb(&mut t1, T1, ok(&u128_json(0)));
    assert_eq!(t1.get_job().status, "Settled");
    Ctx::new(ROOT, T1).at(AFTER).set();
    root.child_closed(U128(2 * USD), None, None);

    // Root (no fee): 60 unspent + 2 returned = 62 back to the client.
    assert_eq!(root.get_job().ft_balance.0, 62 * USD);
    Ctx::new(ROOT, EVE).at(AFTER).set();
    root.settle();
    let args = call_args("ft_transfer");
    assert_eq!(args["receiver_id"], CLIENT);
    assert_eq!(args["amount"], (62 * USD).to_string());
    // Money rule: 8 (worker2) + 30 (worker) + 62 (client) == 100 funded.
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
            None,
            None,
            None,
        );
        Ctx::new(T1, TOKEN).set();
        j.ft_on_transfer(acc(ROOT), U128(PAY), "fund".into());
        j
    }

    /// A delivery without a receipt; the response bytes go to the checker.
    pub fn plain(j: &mut Job, resp: &[u8]) {
        Ctx::new(T1, WORKER).set();
        j.deliver(
            hex::encode(ht_core::receipt::sha256(resp)),
            None,
            None,
            None,
            Some(Base64VecU8(resp.to_vec())),
        );
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
        plain(&mut j, b"plain work");
        let c = calls();
        assert_eq!(c.iter().filter(|x| x.1 == "check").count(), 1);
        assert!(c.contains(&(CHECKER.into(), "check".into())));
        assert!(c.contains(&(T1.into(), "on_check".into())));
        let a = call_args("check");
        assert_eq!(a["job"], T1);
        assert_eq!(a["seq"], 1);
        assert_eq!(a["panel"], 0);
        assert_eq!(
            a["deliverable_hash"],
            hex::encode(ht_core::receipt::sha256(b"plain work"))
        );
        // The checker gets the content itself, not just its hash.
        assert_eq!(a["deliverable"], "plain work");
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
        use super::receipt_tests::{deliver_with, receipt_for, RESP2};
        deliver_with(&mut j, receipt_for(&key(1), &request(T1, 2), RESP2), RESP2);
        assert_eq!(j.get_job().seq, 2);
    }

    #[test]
    #[should_panic(expected = "delivery limit reached")]
    fn deliveries_are_capped() {
        let mut j = sub_full(true);
        for seq in 1..=MAX_DELIVERIES {
            plain(&mut j, format!("try {seq}").as_bytes());
            score(&mut j, seq, num(1));
        }
        plain(&mut j, b"plain work");
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
        plain(&mut j, b"plain work");
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
        plain(&mut j, b"plain work");
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
    fn a_note_bought_part_counts_as_paid_for_the_lead_fee() {
        // The worker was paid early by the lead's treasury: the notice
        // tells the parent the part was paid, so the root counts it for
        // the lead's fee (`the_lead_fee_is_earned_pro_rata_to_the_paid_parts`).
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        note_paid(&mut j, ok(b""));
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        payee_cb(&mut j, T1, true);
        balance_cb(&mut j, T1, ok(&u128_json(0)));
        assert_eq!(call_args("child_closed")["paid"], true);
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
        plain(&mut j, b"plain work");
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
    fn a_routed_worker_is_paid_now_into_the_intents_verifier() {
        let mut j = checked();
        Ctx::new(T1, WORKER).set();
        j.set_payout_via(Some(acc("intents.test")));
        buy(&mut j, TREASURY, QUOTE);
        assert!(transfers().is_empty(), "no plain transfer");
        assert_eq!(call_args("storage_deposit")["account_id"], "intents.test");
        let c = call_args("ft_transfer_call");
        assert_eq!(c["receiver_id"], "intents.test");
        assert_eq!(c["amount"], QUOTE.to_string());
        assert_eq!(c["msg"], WORKER);
        // The Verifier kept it all: sold; the treasury, now the payee, has no route.
        assert_eq!(
            note_paid(&mut j, ok(format!("\"{QUOTE}\"").as_bytes())),
            U128(0)
        );
        let v = j.get_job();
        assert!(v.note_sold && !v.note_pending);
        assert_eq!(v.payout_via, None);
    }

    #[test]
    fn a_refused_routed_note_payment_undoes_the_sale_and_drops_the_route() {
        let mut j = checked();
        Ctx::new(T1, WORKER).set();
        j.set_payout_via(Some(acc("intents.test")));
        buy(&mut j, TREASURY, QUOTE);
        // Used 0: the Verifier refunded; the treasury gets its price back.
        assert_eq!(note_paid(&mut j, ok(b"\"0\"")), U128(QUOTE));
        let v = j.get_job();
        assert!(!v.note_sold);
        assert_eq!(v.payee, Some(acc(WORKER)));
        assert_eq!(v.payout_via, None);
    }

    #[test]
    #[should_panic(expected = "note sale in flight")]
    fn no_route_change_while_a_note_sale_is_in_flight() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        Ctx::new(T1, TREASURY).set();
        j.set_payout_via(Some(acc("intents.test")));
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
        j.dispute(Base64VecU8(receipt_tests::RESPONSE.to_vec()));
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
        j.dispute(Base64VecU8(receipt_tests::RESPONSE.to_vec()));
    }

    #[test]
    #[should_panic(expected = "window is closed")]
    fn dispute_after_window_refused() {
        let mut j = checked();
        Ctx::new(T1, LEAD).at(AFTER).set();
        j.dispute(Base64VecU8(receipt_tests::RESPONSE.to_vec()));
    }

    #[test]
    #[should_panic(expected = "already disputed")]
    fn second_dispute_refused() {
        let mut j = disputed();
        verdict(&mut j, num(9), num(9));
        Ctx::new(T1, LEAD).set();
        j.dispute(Base64VecU8(receipt_tests::RESPONSE.to_vec()));
    }

    #[test]
    #[should_panic(expected = "root disputes are not built")]
    fn root_dispute_not_built() {
        let mut j = funded_root(USD);
        Ctx::new(ROOT, CLIENT).set();
        j.dispute(Base64VecU8(receipt_tests::RESPONSE.to_vec()));
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
    fn dispute_after_own_note_purchase_only_hurts_treasury() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        note_paid(&mut j, ok(b""));
        Ctx::new(T1, LEAD).set();
        j.dispute(Base64VecU8(receipt_tests::RESPONSE.to_vec()));
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
        j.dispute(Base64VecU8(receipt_tests::RESPONSE.to_vec()));
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
            None,
            None,
            None,
        );
        Ctx::new(ROOT, TOKEN).set();
        r.ft_on_transfer(acc(CLIENT), U128(100 * USD), "fund".into());
        Ctx::new(ROOT, LEAD).deposit(HIRE_MIN_NEAR).set();
        r.hire(acc(WORKER), U128(USD), spec(), None);
        let a = call_args("new");
        assert_eq!(a["checker"], CHECKER);
        assert_eq!(a["treasury"], TREASURY);
    }

    // ------------------------------------------------ review fixes (2026-11-05)

    #[test]
    #[should_panic(expected = "check pending")]
    fn settle_waits_for_a_pending_check() {
        // Junk in the last block, then settle before the score lands.
        let mut j = sub_full(true);
        Ctx::new(T1, WORKER).at(AFTER - 2 * NS_PER_SEC).set();
        j.deliver(
            hex::encode(ht_core::receipt::sha256(b"junk")),
            None,
            None,
            None,
            Some(Base64VecU8(b"junk".to_vec())),
        );
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
    }

    #[test]
    #[should_panic(expected = "check pending")]
    fn approve_waits_for_a_pending_check() {
        let mut j = sub_full(true);
        plain(&mut j, b"x");
        Ctx::new(T1, LEAD).set();
        j.approve();
    }

    #[test]
    #[should_panic(expected = "send the response bytes")]
    fn checker_needs_the_content() {
        let mut j = sub_full(true);
        Ctx::new(T1, WORKER).set();
        j.deliver("cd".repeat(32), None, None, None, None);
    }

    #[test]
    #[should_panic(expected = "resp_hash is not sha256(resp)")]
    fn content_must_match_hash() {
        let mut j = sub_full(true);
        Ctx::new(T1, WORKER).set();
        j.deliver(
            "cd".repeat(32),
            None,
            None,
            None,
            Some(Base64VecU8(b"x".to_vec())),
        );
    }

    #[test]
    #[should_panic(expected = "same deliverable as before")]
    fn same_bytes_cannot_be_rerolled() {
        let mut j = sub_full(true);
        plain(&mut j, b"junk");
        score(&mut j, 1, num(2));
        plain(&mut j, b"junk");
    }

    #[test]
    fn a_failed_check_makes_the_note_unsellable() {
        let mut j = sub_full(true);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        score(&mut j, 1, num(2));
        use super::receipt_tests::{deliver_with, receipt_for, RESP2};
        deliver_with(&mut j, receipt_for(&key(1), &request(T1, 2), RESP2), RESP2);
        score(&mut j, 2, num(9));
        assert_eq!(j.get_job().status, "Checked");
        assert_eq!(j.quote_note(), None, "a failed check on record");
    }

    #[test]
    fn no_checker_no_note() {
        let mut j = sub_full(false);
        deliver_r(&mut j, receipt(&key(1), &request(T1, 1)));
        assert_eq!(j.get_job().status, "Checked");
        assert_eq!(j.quote_note(), None);
    }

    #[test]
    fn unscored_delivery_is_not_paid_without_approval() {
        let mut j = sub_full(true);
        plain(&mut j, b"withheld");
        score(&mut j, 1, PromiseResult::Failed);
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        assert_eq!(transfers(), vec![(ROOT.into(), PAY.to_string())]);
    }

    #[test]
    fn unscored_delivery_is_paid_when_the_manager_approves() {
        let mut j = sub_full(true);
        plain(&mut j, b"fine work, checker was down");
        score(&mut j, 1, PromiseResult::Failed);
        Ctx::new(T1, LEAD).set();
        j.approve();
        Ctx::new(T1, EVE).set();
        j.settle();
        assert_eq!(transfers(), vec![(WORKER.into(), PAY.to_string())]);
    }

    #[test]
    fn lost_check_answer_can_be_cleared_after_timeout() {
        let mut j = sub_full(true);
        plain(&mut j, b"x");
        Ctx::new(T1, EVE)
            .at(T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC)
            .set();
        j.resume_check();
        let v = j.get_job();
        assert!(!v.check_pending);
        assert_eq!(v.status, "Delivered");
        assert_eq!(v.quality, None);
    }

    #[test]
    #[should_panic(expected = "check is not stuck yet")]
    fn resume_check_too_early() {
        let mut j = sub_full(true);
        plain(&mut j, b"x");
        Ctx::new(T1, EVE).set();
        j.resume_check();
    }

    #[test]
    fn missing_dispute_votes_count_as_junk() {
        let mut j = disputed();
        verdict(&mut j, PromiseResult::Failed, num(9));
        // original 8 (pass), 9 (pass), missing (junk): 1 junk -> rejected
        assert_eq!(j.get_job().status, "Checked");
        let mut j = disputed();
        verdict(&mut j, PromiseResult::Failed, num(1));
        // missing (junk) + 1 (junk) -> upheld
        assert_eq!(j.get_job().status, "Refunded");
    }

    #[test]
    fn stuck_dispute_resume_upholds_never_rejects() {
        // A worker who waits out the dispute cannot win by resuming it.
        let mut j = disputed();
        Ctx::new(T1, WORKER)
            .at(T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC)
            .set();
        j.resume_dispute();
        assert_eq!(j.get_job().status, "Refunded");
    }

    #[test]
    #[should_panic(expected = "check pending")]
    fn dispute_waits_for_a_pending_check() {
        let mut j = sub_full(true);
        plain(&mut j, b"x");
        Ctx::new(T1, LEAD).set();
        j.dispute(Base64VecU8(receipt_tests::RESPONSE.to_vec()));
    }

    // ------------------------------------------------ stuck note sale

    fn stuck_sale() -> Job {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        j
    }

    fn resume_note_with(j: &mut Job, real: u128) -> bool {
        Ctx::new(T1, EVE)
            .at(T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC)
            .set();
        j.resume_note();
        assert!(calls().contains(&(TOKEN.into(), "ft_balance_of".into())));
        Ctx::new(T1, T1)
            .at(T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC)
            .results(vec![ok(&u128_json(real))])
            .set();
        j.on_resume_note()
    }

    #[test]
    fn stuck_sale_worker_unpaid_is_undone() {
        let mut j = stuck_sale();
        resume_note_with(&mut j, PAY);
        let v = j.get_job();
        assert!(!v.note_pending && !v.note_sold);
        assert_eq!(v.payee, Some(acc(WORKER)));
        assert_eq!(v.amount.0, PAY);
    }

    #[test]
    fn stuck_sale_worker_paid_from_job_counts_as_prepaid() {
        // The callback died, the token refunded the treasury, but the
        // worker was paid out of the job's own tokens: the worker is owed
        // only the rest, and the books still close.
        let mut j = stuck_sale();
        resume_note_with(&mut j, PAY - QUOTE);
        let v = j.get_job();
        assert!(!v.note_pending && !v.note_sold);
        assert_eq!(v.payee, Some(acc(WORKER)));
        assert_eq!(v.amount.0, PAY - QUOTE);
        assert_eq!(v.ft_balance.0, PAY - QUOTE);
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        assert_eq!(
            transfers(),
            vec![(WORKER.into(), (PAY - QUOTE).to_string())]
        );
    }

    #[test]
    #[should_panic(expected = "note sale is not stuck yet")]
    fn resume_note_too_early() {
        let mut j = stuck_sale();
        Ctx::new(T1, EVE).set();
        j.resume_note();
    }

    #[test]
    fn oversized_success_result_still_counts_as_success() {
        let mut j = checked();
        buy(&mut j, TREASURY, QUOTE);
        Ctx::new(T1, T1).results(vec![ok(&[b'x'; 500])]).set();
        assert_eq!(j.on_note_paid(acc(WORKER), U128(QUOTE)), U128(0));
        assert!(j.get_job().note_sold);
    }
}

mod stuck_hire {
    use super::*;

    /// Resume a stuck hire; the token answers `real` and the child's own
    /// funding view is unavailable (as for a child that was never created).
    pub(super) fn resume_hire_with(j: &mut Job, idx: u32, real: PromiseResult) {
        resume_hire_seen(j, idx, real, PromiseResult::Failed);
    }

    /// Resume a stuck hire; the token answers `real`, the child's
    /// `get_funded` answers `funded`.
    pub(super) fn resume_hire_seen(
        j: &mut Job,
        idx: u32,
        real: PromiseResult,
        funded: PromiseResult,
    ) {
        let at = T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC;
        Ctx::new(ROOT, EVE).at(at).set();
        j.resume_hire(idx);
        assert!(calls().contains(&(TOKEN.into(), "ft_balance_of".into())));
        assert!(calls().contains(&(T1.into(), "get_funded".into())));
        Ctx::new(ROOT, ROOT)
            .at(at)
            .results(vec![real, funded])
            .set();
        j.on_resume_hire(idx);
    }

    #[test]
    fn lost_hire_callback_with_funded_child_opens_it() {
        let mut j = funded_root(100 * USD);
        let idx = hire_one(&mut j, WORKER, 30 * USD);
        resume_hire_with(&mut j, idx, ok(&u128_json(30 * USD)));
        let v = j.get_job();
        assert_eq!(v.children[0].state, ChildState::Open);
        assert_eq!(v.children_pending, 0);
        assert_eq!(v.children_open, 1);
    }

    #[test]
    fn lost_hire_callback_with_empty_child_rolls_back() {
        let mut j = funded_root(100 * USD);
        let idx = hire_one(&mut j, WORKER, 30 * USD);
        resume_hire_with(&mut j, idx, ok(&u128_json(0)));
        let v = j.get_job();
        assert_eq!(v.children[0].state, ChildState::Failed);
        assert_eq!(v.children_pending, 0);
        assert_eq!(v.unallocated.0, 100 * USD);
    }

    #[test]
    fn unknown_child_balance_keeps_waiting() {
        let mut j = funded_root(100 * USD);
        let idx = hire_one(&mut j, WORKER, 30 * USD);
        resume_hire_with(&mut j, idx, PromiseResult::Failed);
        assert_eq!(j.get_job().children_pending, 1);
    }

    #[test]
    #[should_panic(expected = "hire is not stuck yet")]
    fn resume_hire_too_early() {
        let mut j = funded_root(100 * USD);
        let idx = hire_one(&mut j, WORKER, 30 * USD);
        Ctx::new(ROOT, EVE).set();
        j.resume_hire(idx);
    }

    #[test]
    #[should_panic(expected = "child is not being created")]
    fn late_create_callback_after_resume_refused() {
        let mut j = funded_root(100 * USD);
        let idx = hire_one(&mut j, WORKER, 30 * USD);
        resume_hire_with(&mut j, idx, ok(&u128_json(0)));
        Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
        j.on_child_created(idx);
    }

    /// The static floors of the non-settle paths. The settle chain itself
    /// is checked end to end at its minimum in `settle_gas`.
    #[test]
    fn gas_minimums_cover_the_static_gas_they_schedule() {
        assert!(GAS_ON_RESUME.as_gas() >= GAS_ON_PAYEE_PAID.as_gas());
        assert!(
            GAS_RESUME_CLOSE_MIN.as_gas()
                >= GAS_CHILD_CLOSED.as_gas()
                    + GAS_ON_PARENT_ACK.as_gas()
                    + Gas::from_tgas(5).as_gas()
        );
        assert!(
            GAS_RESUME_HIRE_MIN.as_gas()
                >= GAS_FT_BALANCE_OF.as_gas()
                    + GAS_GET_FUNDED.as_gas()
                    + GAS_ON_RESUME_HIRE.as_gas()
                    + Gas::from_tgas(20).as_gas()
        );
        assert!(
            GAS_DISPUTE_MIN.as_gas()
                >= 2 * GAS_CHECK.as_gas() + GAS_ON_DISPUTE.as_gas() + Gas::from_tgas(20).as_gas()
        );
        assert!(
            GAS_DELIVER_MIN.as_gas()
                >= GAS_CHECK.as_gas() + GAS_ON_CHECK.as_gas() + Gas::from_tgas(25).as_gas()
        );
        assert!(
            GAS_BUY_MIN.as_gas()
                >= GAS_STORAGE_DEPOSIT.as_gas()
                    + GAS_FT_TRANSFER.as_gas()
                    + GAS_ON_NOTE_PAID.as_gas()
                    + Gas::from_tgas(10).as_gas()
        );
    }
}

// ================================================================ signed check

mod signed_check {
    use super::check_note_dispute::{CHECKER, TREASURY};
    use super::receipt_tests::{addr_hex, key, sign, MODEL};
    use super::*;
    use ht_core::check::request_bytes;
    use ht_core::receipt::{sha256, sha256_hex};

    pub const TASK: &str = "Translate to Spanish: global contracts are deployed once";
    const DELIVERED: &[u8] = b"los contratos globales se despliegan una vez";
    const PAY: u128 = 30 * USD;
    const CHECK_DONE: u64 = T0 + (CHECK_WINDOW_SECS + 1) * NS_PER_SEC;

    fn task_spec() -> String {
        hex::encode(sha256(TASK.as_bytes()))
    }

    /// A funded sub-job in signed-check mode; signer = key(1).
    pub fn judged_sub() -> Job {
        Ctx::new(T1, ROOT).set();
        let mut j = Job::new(
            None,
            acc(LEAD),
            Some(acc(WORKER)),
            acc(TOKEN),
            U128(PAY),
            task_spec(),
            WINDOW,
            addr_hex(&key(1)),
            code_hash(),
            None,
            Some(acc(TREASURY)),
            Some(MODEL.into()),
            None,
            None,
        );
        Ctx::new(T1, TOKEN).set();
        j.ft_on_transfer(acc(ROOT), U128(PAY), "fund".into());
        j
    }

    fn deliver(j: &mut Job, bytes: &[u8]) {
        Ctx::new(T1, WORKER).set();
        j.deliver(
            hex::encode(sha256(bytes)),
            None,
            None,
            None,
            Some(Base64VecU8(bytes.to_vec())),
        );
    }

    /// A NEAR AI answer `{"score": n}` (plus `salt` to make it distinct),
    /// signed by `k` over the request the contract builds.
    fn answer(
        k: u8,
        j: &Job,
        panel: u8,
        bytes: &[u8],
        score: &str,
        salt: &str,
    ) -> (Base64VecU8, String) {
        let req = request_bytes(
            MODEL,
            T1,
            j.seq,
            panel,
            TASK,
            std::str::from_utf8(bytes).unwrap(),
        );
        let ans = format!(
            "{{\"id\":\"{salt}\",\"choices\":[{{\"message\":{{\"content\":\"{score}\"}}}}]}}"
        )
        .into_bytes();
        let text = format!("{MODEL}:{}:{}", sha256_hex(&req), sha256_hex(&ans));
        (Base64VecU8(ans), sign(&key(k), &text))
    }

    fn submit(j: &mut Job, who: &str, panel: u8, bytes: &[u8], a: (Base64VecU8, String)) -> u8 {
        Ctx::new(T1, who).set();
        j.submit_check(panel, TASK.into(), Base64VecU8(bytes.to_vec()), a.0, a.1)
    }

    fn finalize(j: &mut Job) -> bool {
        Ctx::new(T1, EVE).at(CHECK_DONE).set();
        j.finalize_check()
    }

    #[test]
    fn delivery_opens_a_round_and_a_signed_answer_scores_it() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        assert!(j.get_job().check_pending);
        let a = answer(1, &j, 0, DELIVERED, "{\\\"score\\\": 8}", "a");
        assert_eq!(submit(&mut j, EVE, 0, DELIVERED, a), 8);
        assert!(finalize(&mut j));
        let v = j.get_job();
        assert_eq!(v.status, "Checked");
        assert_eq!(v.quality, Some(8));
        assert!(!v.check_pending);
    }

    #[test]
    fn view_returns_the_exact_request_to_send() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let r = j.check_request(0, TASK.into(), Base64VecU8(DELIVERED.to_vec()));
        assert_eq!(
            r.0,
            request_bytes(
                MODEL,
                T1,
                1,
                0,
                TASK,
                std::str::from_utf8(DELIVERED).unwrap()
            )
        );
    }

    #[test]
    fn lowest_answer_counts() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 0, DELIVERED, "8", "a");
        submit(&mut j, WORKER, 0, DELIVERED, a);
        let b = answer(1, &j, 0, DELIVERED, "7", "b");
        submit(&mut j, LEAD, 0, DELIVERED, b);
        finalize(&mut j);
        assert_eq!(j.get_job().quality, Some(7));
    }

    #[test]
    fn split_answers_are_unclear_not_junk() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 0, DELIVERED, "8", "a");
        submit(&mut j, WORKER, 0, DELIVERED, a);
        let b = answer(1, &j, 0, DELIVERED, "2", "b");
        submit(&mut j, LEAD, 0, DELIVERED, b);
        assert!(!finalize(&mut j));
        let v = j.get_job();
        assert_eq!(v.status, "Delivered", "unscored");
        assert_eq!(v.quality, None);
        assert_eq!(v.failed_checks, 0, "an unclear check is not a failed one");
    }

    #[test]
    fn a_low_answer_fails_the_check() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 0, DELIVERED, "3", "a");
        submit(&mut j, EVE, 0, DELIVERED, a);
        finalize(&mut j);
        let v = j.get_job();
        assert_eq!(v.status, "Open");
        assert_eq!(v.failed_checks, 1);
    }

    #[test]
    #[should_panic(expected = "send the response bytes")]
    fn hash_only_delivery_refused_when_a_judge_is_pinned() {
        let mut j = judged_sub();
        Ctx::new(T1, WORKER).set();
        j.deliver(hex::encode(sha256(DELIVERED)), None, None, None, None);
    }

    fn failed(j: &mut Job) {
        deliver(j, DELIVERED);
        let a = answer(1, j, 0, DELIVERED, "2", "a");
        submit(j, EVE, 0, DELIVERED, a);
        finalize(j);
    }

    #[test]
    fn the_buyer_fires_a_failed_worker_and_gets_everything_back() {
        let mut j = judged_sub();
        failed(&mut j);
        Ctx::new(T1, LEAD).at(CHECK_DONE).set();
        j.fire();
        // Settle runs now, long before the window, and pays the worker nothing.
        Ctx::new(T1, EVE).at(CHECK_DONE).set();
        j.settle();
        assert_eq!(transfers(), vec![(ROOT.into(), PAY.to_string())]);
    }

    #[test]
    #[should_panic(expected = "fire only after a failed check")]
    fn a_worker_whose_work_passed_cannot_be_fired() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 0, DELIVERED, "8", "a");
        submit(&mut j, EVE, 0, DELIVERED, a);
        finalize(&mut j);
        Ctx::new(T1, LEAD).at(CHECK_DONE).set();
        j.fire();
    }

    #[test]
    #[should_panic(expected = "fire only after a failed check")]
    fn a_worker_cannot_be_fired_before_any_check() {
        let mut j = judged_sub();
        Ctx::new(T1, LEAD).set();
        j.fire();
    }

    #[test]
    #[should_panic(expected = "only the sub-job's manager can fire")]
    fn only_the_buyer_can_fire() {
        let mut j = judged_sub();
        failed(&mut j);
        Ctx::new(T1, EVE).at(CHECK_DONE).set();
        j.fire();
    }

    #[test]
    #[should_panic(expected = "window is closed")]
    fn a_fired_worker_cannot_redeliver() {
        let mut j = judged_sub();
        failed(&mut j);
        Ctx::new(T1, LEAD).at(CHECK_DONE).set();
        j.fire();
        Ctx::new(T1, WORKER).at(CHECK_DONE + NS_PER_SEC).set();
        j.deliver(
            hex::encode(sha256(DELIVERED)),
            None,
            None,
            None,
            Some(Base64VecU8(DELIVERED.to_vec())),
        );
    }

    #[test]
    fn unscored_judged_delivery_is_not_paid_without_approval() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        assert!(!finalize(&mut j));
        Ctx::new(T1, WORKER)
            .at(T0 + (WINDOW + 10) * NS_PER_SEC)
            .set();
        j.settle();
        assert_eq!(transfers(), vec![(ROOT.into(), PAY.to_string())]);
    }

    #[test]
    fn no_answer_leaves_the_delivery_unscored() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        assert!(!finalize(&mut j));
        assert_eq!(j.get_job().status, "Delivered");
    }

    #[test]
    #[should_panic(expected = "receipt: signer is not the pinned NEAR AI Cloud key")]
    fn answer_signed_by_another_key_refused() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(2, &j, 0, DELIVERED, "9", "a");
        submit(&mut j, WORKER, 0, DELIVERED, a);
    }

    #[test]
    #[should_panic(expected = "receipt: signer is not the pinned NEAR AI Cloud key")]
    fn answer_to_a_different_question_refused() {
        // Signed for panel 1's request, submitted as panel 0.
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 1, DELIVERED, "9", "a");
        submit(&mut j, WORKER, 0, DELIVERED, a);
    }

    #[test]
    #[should_panic(expected = "deliverable does not match the delivered hash")]
    fn answer_about_other_bytes_refused() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 0, b"something better", "9", "a");
        submit(&mut j, WORKER, 0, b"something better", a);
    }

    #[test]
    #[should_panic(expected = "task text does not match the spec hash")]
    fn answer_for_another_task_refused() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        Ctx::new(T1, WORKER).set();
        j.submit_check(
            0,
            "an easier task".into(),
            Base64VecU8(DELIVERED.to_vec()),
            Base64VecU8(b"{}".to_vec()),
            "00".repeat(65),
        );
    }

    #[test]
    #[should_panic(expected = "this answer was already counted")]
    fn same_answer_counts_once() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 0, DELIVERED, "3", "a");
        submit(&mut j, LEAD, 0, DELIVERED, a.clone());
        submit(&mut j, LEAD, 0, DELIVERED, a);
    }

    #[test]
    #[should_panic(expected = "answer is not")]
    fn unparseable_answer_refused() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 0, DELIVERED, "great work, 10/10", "a");
        submit(&mut j, WORKER, 0, DELIVERED, a);
    }

    #[test]
    #[should_panic(expected = "check round is closed")]
    fn late_answer_refused() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 0, DELIVERED, "9", "a");
        Ctx::new(T1, WORKER).at(CHECK_DONE).set();
        j.submit_check(0, TASK.into(), Base64VecU8(DELIVERED.to_vec()), a.0, a.1);
    }

    #[test]
    #[should_panic(expected = "check round is still open")]
    fn finalize_too_early_refused() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        Ctx::new(T1, EVE).set();
        j.finalize_check();
    }

    #[test]
    #[should_panic(expected = "check pending")]
    fn settle_waits_for_the_round() {
        let mut j = judged_sub();
        Ctx::new(T1, WORKER).at(AFTER - 10 * NS_PER_SEC).set();
        j.deliver(
            hex::encode(sha256(DELIVERED)),
            None,
            None,
            None,
            Some(Base64VecU8(DELIVERED.to_vec())),
        );
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
    }

    #[test]
    #[should_panic(expected = "signed checks finish with finalize_check")]
    fn resume_check_is_not_for_signed_mode() {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        Ctx::new(T1, EVE)
            .at(T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC)
            .set();
        j.resume_check();
    }

    #[test]
    #[should_panic(expected = "pin a checker contract or a judge model, not both")]
    fn checker_and_judge_together_refused() {
        Ctx::new(T1, ROOT).set();
        Job::new(
            None,
            acc(LEAD),
            Some(acc(WORKER)),
            acc(TOKEN),
            U128(PAY),
            task_spec(),
            WINDOW,
            addr_hex(&key(1)),
            code_hash(),
            Some(acc(CHECKER)),
            None,
            Some(MODEL.into()),
            None,
            None,
        );
    }

    fn checked_judged() -> Job {
        let mut j = judged_sub();
        deliver(&mut j, DELIVERED);
        let a = answer(1, &j, 0, DELIVERED, "7", "a");
        submit(&mut j, EVE, 0, DELIVERED, a);
        finalize(&mut j);
        assert_eq!(j.get_job().status, "Checked");
        j
    }

    fn dispute_done(j: &mut Job) -> bool {
        Ctx::new(T1, EVE)
            .at(T0 + (2 * CHECK_WINDOW_SECS + 2) * NS_PER_SEC)
            .set();
        j.finalize_dispute()
    }

    #[test]
    fn dispute_opens_two_panels_and_two_junk_votes_uphold() {
        let mut j = checked_judged();
        Ctx::new(T1, LEAD).at(CHECK_DONE).set();
        j.dispute(Base64VecU8(DELIVERED.to_vec()));
        assert_eq!(j.get_job().rounds.len(), 2);
        for (p, s) in [(1u8, "2"), (2, "3")] {
            let a = answer(1, &j, p, DELIVERED, s, "x");
            Ctx::new(T1, LEAD).at(CHECK_DONE).set();
            j.submit_check(p, TASK.into(), Base64VecU8(DELIVERED.to_vec()), a.0, a.1);
        }
        assert!(dispute_done(&mut j));
        assert_eq!(j.get_job().status, "Refunded");
    }

    #[test]
    fn dispute_with_one_good_panel_is_rejected() {
        let mut j = checked_judged();
        Ctx::new(T1, LEAD).at(CHECK_DONE).set();
        j.dispute(Base64VecU8(DELIVERED.to_vec()));
        let a = answer(1, &j, 1, DELIVERED, "8", "x");
        Ctx::new(T1, WORKER).at(CHECK_DONE).set();
        j.submit_check(1, TASK.into(), Base64VecU8(DELIVERED.to_vec()), a.0, a.1);
        // Panel 2 gets no answer: it abstains. Votes 7, 8: rejected.
        assert!(!dispute_done(&mut j));
        assert_eq!(j.get_job().status, "Checked");
    }

    #[test]
    fn a_dispute_with_empty_panels_is_rejected() {
        // The buyer disputes and leaves both panels unanswered: they abstain,
        // so the passing first check stands.
        let mut j = checked_judged();
        Ctx::new(T1, LEAD).at(CHECK_DONE).set();
        j.dispute(Base64VecU8(DELIVERED.to_vec()));
        assert!(!dispute_done(&mut j));
        assert_eq!(j.get_job().status, "Checked");
    }

    #[test]
    fn a_buyer_cannot_win_a_dispute_with_one_low_answer_against_a_high_one() {
        // Panel 1: the buyer sends 2, the worker sends 9: the answers
        // disagree, so the panel abstains. Panel 2 says 2. Votes 7, -, 2:
        // one junk vote, rejected.
        let mut j = checked_judged();
        Ctx::new(T1, LEAD).at(CHECK_DONE).set();
        j.dispute(Base64VecU8(DELIVERED.to_vec()));
        for (who, p, sc) in [(LEAD, 1u8, "2"), (WORKER, 1, "9"), (LEAD, 2, "2")] {
            let a = answer(1, &j, p, DELIVERED, sc, "x");
            Ctx::new(T1, who).at(CHECK_DONE).set();
            j.submit_check(p, TASK.into(), Base64VecU8(DELIVERED.to_vec()), a.0, a.1);
        }
        assert!(!dispute_done(&mut j));
        assert_eq!(j.get_job().status, "Checked");
    }

    #[test]
    #[should_panic(expected = "dispute rounds are still open")]
    fn finalize_dispute_too_early_refused() {
        let mut j = checked_judged();
        Ctx::new(T1, LEAD).at(CHECK_DONE).set();
        j.dispute(Base64VecU8(DELIVERED.to_vec()));
        Ctx::new(T1, EVE).at(CHECK_DONE + NS_PER_SEC).set();
        j.finalize_dispute();
    }

    #[test]
    fn a_signed_check_makes_a_verified_note_sellable() {
        // Receipt from the worker's own NEAR AI call (bound to t1, delivery 1),
        // then a passing signed check: the note has a quote.
        let mut j = judged_sub();
        let resp = DELIVERED;
        let req = format!(
            "{{\"model\":\"{MODEL}\",\"messages\":[{{\"role\":\"user\",\"content\":\"HT1:{T1}:{}:1 {TASK}\"}}]}}",
            task_spec()
        )
        .into_bytes();
        let text = format!("{MODEL}:{}:{}", sha256_hex(&req), sha256_hex(resp));
        Ctx::new(T1, WORKER).set();
        j.deliver(
            hex::encode(sha256(resp)),
            Some(Base64VecU8(req)),
            Some(sign(&key(1), &text)),
            Some(MODEL.into()),
            Some(Base64VecU8(resp.to_vec())),
        );
        let a = answer(1, &j, 0, DELIVERED, "8", "a");
        submit(&mut j, EVE, 0, DELIVERED, a);
        finalize(&mut j);
        assert_eq!(j.quote_note(), Some(U128(PAY - PAY / 100)));
    }
}

// ================================================================ track record

#[test]
fn deleted_sub_reports_the_workers_record_to_the_registry() {
    let mut j = paid_sub();
    balance_cb(&mut j, T1, ok(&u128_json(0)));
    let r = call_args("record");
    assert_eq!(r["worker"], WORKER);
    assert_eq!(r["delivered"], true);
    assert_eq!(r["paid"], true);
    assert_eq!(r["junk"], false);
    // Weighted by money: the pay at stake and who paid it.
    assert_eq!(r["amount"], json!(U128(j.get_job().amount.0)));
    assert_eq!(r["payer"], LEAD);
    let to = get_created_receipts()
        .into_iter()
        .find(|x| {
            x.actions.iter().any(|a| matches!(a, MockAction::FunctionCallWeight { method_name, .. } if method_name == b"record"))
        })
        .unwrap()
        .receiver_id;
    assert_eq!(
        to,
        acc(ROOT),
        "registry defaults to the creator; hire passes the factory down"
    );
}

#[test]
fn root_reports_no_record() {
    let mut j = funded_root(10 * USD);
    Ctx::new(ROOT, CLIENT).set();
    j.approve();
    Ctx::new(ROOT, EVE).set();
    j.settle();
    rest_cb(&mut j, ROOT, true);
    balance_cb(&mut j, ROOT, ok(&u128_json(0)));
    assert!(!calls().iter().any(|c| c.1 == "record"));
}

#[test]
fn hire_passes_the_registry_down() {
    let mut j = funded_root(100 * USD);
    hire_one(&mut j, WORKER, 30 * USD);
    assert_eq!(call_args("new")["registry"], FACTORY);
}

mod bidding {
    use super::*;

    const BID_SECS: u64 = 60;
    const CLOSED: u64 = T0 + (BID_SECS + 1) * NS_PER_SEC;

    fn open(j: &mut Job) -> u32 {
        Ctx::new(ROOT, LEAD).set();
        j.open_bids(spec(), U128(30 * USD), BID_SECS, None, None)
    }

    fn bid(j: &mut Job, who: &str, slot: u32, price: u128) {
        Ctx::new(ROOT, who).deposit(BID_NEAR).set();
        j.bid(slot, U128(price));
    }

    fn award(j: &mut Job, slot: u32, who: &str) {
        Ctx::new(ROOT, LEAD)
            .at(CLOSED)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(slot, acc(who), None);
    }

    #[test]
    #[should_panic(expected = "attach 0.001 NEAR for the bid's storage")]
    fn a_new_bid_pays_for_its_storage() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        Ctx::new(ROOT, WORKER).set();
        j.bid(s, U128(20 * USD));
    }

    #[test]
    fn replacing_a_bid_needs_no_new_deposit() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        bid(&mut j, WORKER, s, 25 * USD);
        Ctx::new(ROOT, WORKER).set();
        j.bid(s, U128(20 * USD));
        assert_eq!(j.get_bids(s).unwrap().bids[0].price, U128(20 * USD));
    }

    #[test]
    fn manager_opens_workers_bid_manager_awards_at_the_bid_price() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        bid(&mut j, WORKER, s, 25 * USD);
        bid(&mut j, WORKER2, s, 18 * USD);
        let slot = j.get_bids(s).unwrap();
        assert_eq!(slot.bids.len(), 2);
        award(&mut j, s, WORKER);
        let v = j.get_job();
        assert_eq!(v.children.len(), 1);
        assert_eq!(v.children[0].worker, acc(WORKER));
        assert_eq!(v.children[0].total, U128(25 * USD), "hired at its own bid");
        assert_eq!(j.get_bids(s).unwrap().awarded, Some(acc(WORKER)));
    }

    #[test]
    fn a_new_bid_replaces_the_bidders_old_one() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        bid(&mut j, WORKER, s, 25 * USD);
        bid(&mut j, WORKER, s, 20 * USD);
        let b = j.get_bids(s).unwrap().bids;
        assert_eq!(
            b,
            vec![Bid {
                worker: acc(WORKER),
                price: U128(20 * USD),
                // The second call's deposit is not kept: it joins the bond.
                bond: U128(BID_NEAR.as_yoctonear()),
                commit: None,
                offer: U128(0),
                backer: None,
                backed: U128(0),
            }]
        );
    }

    #[test]
    #[should_panic(expected = "only the manager or this job's worker can open bids")]
    fn stranger_cannot_open_bids() {
        let mut j = funded_root(100 * USD);
        Ctx::new(ROOT, EVE).set();
        j.open_bids(spec(), U128(30 * USD), BID_SECS, None, None);
    }

    #[test]
    #[should_panic(expected = "bid is above the slot's max")]
    fn bid_above_max_refused() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        bid(&mut j, WORKER, s, 31 * USD);
    }

    #[test]
    #[should_panic(expected = "the manager cannot bid")]
    fn manager_cannot_bid() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        bid(&mut j, LEAD, s, 10 * USD);
    }

    #[test]
    #[should_panic(expected = "bidding is closed")]
    fn late_bid_refused() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        Ctx::new(ROOT, WORKER).at(CLOSED).set();
        j.bid(s, U128(10 * USD));
    }

    #[test]
    #[should_panic(expected = "bidding is still open")]
    fn award_before_close_refused() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        bid(&mut j, WORKER, s, 10 * USD);
        Ctx::new(ROOT, LEAD)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(s, acc(WORKER), None);
    }

    #[test]
    #[should_panic(expected = "did not bid")]
    fn award_to_a_non_bidder_refused() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        bid(&mut j, WORKER, s, 10 * USD);
        award(&mut j, s, WORKER2);
    }

    #[test]
    #[should_panic(expected = "already awarded")]
    fn second_award_refused() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        bid(&mut j, WORKER, s, 10 * USD);
        bid(&mut j, WORKER2, s, 12 * USD);
        award(&mut j, s, WORKER);
        award(&mut j, s, WORKER2);
    }

    #[test]
    #[should_panic(expected = "bid on the awarded")]
    fn bid_after_award_refused() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        bid(&mut j, WORKER, s, 10 * USD);
        award(&mut j, s, WORKER);
        Ctx::new(ROOT, WORKER2).set();
        j.bid(s, U128(5 * USD));
    }

    #[test]
    #[should_panic(expected = "max is more than the job can still hire")]
    fn slot_max_cannot_exceed_the_free_budget() {
        let mut j = funded_root(100 * USD);
        Ctx::new(ROOT, LEAD).set();
        j.open_bids(spec(), U128(101 * USD), BID_SECS, None, None);
    }

    #[test]
    #[should_panic(expected = "too many bids")]
    fn bids_per_slot_are_capped() {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j);
        for i in 0..=MAX_BIDS {
            bid(&mut j, &format!("w{i}.test"), s, 10 * USD);
        }
    }
}

// ============================================================ contractor

/// A sub-job's worker hires its own sub-agents out of its pay (a recursive
/// labor market: any agent that wins work can become a buyer).
mod contractor {
    use super::*;

    const GC: &str = "t1.t1.job-1.hiretree.test";
    const CLOSED: u64 = T0 + 61 * NS_PER_SEC;

    /// t1 (worker WORKER, pay 20) puts a part out for bids and awards
    /// WORKER2 at 9. Returns t1.
    fn contracted() -> Job {
        let mut t1 = funded_sub(20 * USD, 20 * USD);
        Ctx::new(T1, WORKER).set();
        let s = t1.open_bids(spec(), U128(12 * USD), 60, None, None);
        Ctx::new(T1, WORKER2).deposit(BID_NEAR).set();
        t1.bid(s, U128(9 * USD));
        Ctx::new(T1, WORKER)
            .at(CLOSED)
            .deposit(NearToken::from_millinear(50))
            .set();
        t1.award(s, acc(WORKER2), None);
        assert_eq!(
            call_args("new")["manager"],
            WORKER,
            "the contractor manages its sub-job"
        );
        Ctx::new(T1, T1).at(CLOSED).results(vec![ok(b"")]).set();
        t1.on_child_created(0);
        Ctx::new(T1, T1)
            .at(CLOSED)
            .results(vec![ok(&u128_json(9 * USD))])
            .set();
        assert!(t1.on_child_funded(0));
        t1
    }

    #[test]
    fn a_worker_hires_sub_agents_out_of_its_own_pay() {
        let mut t1 = contracted();
        let v = t1.get_job();
        assert!(v.children[0].by_worker);
        assert_eq!(v.children[0].total, U128(9 * USD));
        assert_eq!(t1.ledger.amount, 11 * USD, "pay 20, spent 9: margin 11");
        // The sub-job paid its worker and sent nothing back; the contractor
        // delivers and keeps its margin.
        Ctx::new(T1, GC).at(CLOSED).set();
        t1.child_closed(U128(0), None, None);
        Ctx::new(T1, WORKER).at(CLOSED).set();
        t1.deliver("02".repeat(32), None, None, None, None);
        Ctx::new(T1, EVE).at(AFTER).set();
        t1.settle();
        let args = call_args("ft_transfer");
        assert_eq!(args["receiver_id"], WORKER);
        assert_eq!(args["amount"], (11 * USD).to_string());
    }

    #[test]
    fn unspent_or_refunded_sub_pay_goes_back_to_the_contractor() {
        let mut t1 = contracted();
        // The sub-agent failed: its 9 come back as the contractor's pay.
        Ctx::new(T1, GC).at(CLOSED).set();
        t1.child_closed(U128(9 * USD), None, None);
        assert_eq!(t1.ledger.amount, 20 * USD);
        assert_eq!(t1.get_job().ft_balance.0, 20 * USD);
    }

    #[test]
    fn a_contractors_note_waits_for_its_sub_jobs() {
        let mut t1 = contracted();
        assert!(t1.has_open_worker_hires());
        Ctx::new(T1, GC).at(CLOSED).set();
        t1.child_closed(U128(0), None, None);
        assert!(!t1.has_open_worker_hires());
    }

    #[test]
    #[should_panic(expected = "a contractor keeps part of it")]
    fn a_contractor_cannot_bid_out_its_whole_pay() {
        let mut t1 = funded_sub(20 * USD, 20 * USD);
        Ctx::new(T1, WORKER).set();
        t1.open_bids(spec(), U128(20 * USD), 60, None, None);
    }

    #[test]
    #[should_panic(expected = "the job's worker cannot bid on its own job")]
    fn a_contractor_cannot_bid_on_its_own_slot() {
        let mut t1 = funded_sub(20 * USD, 25 * USD);
        Ctx::new(T1, LEAD).set();
        let s = t1.open_bids(spec(), U128(1), 60, None, None);
        Ctx::new(T1, WORKER).deposit(BID_NEAR).set();
        t1.bid(s, U128(1));
    }

    #[test]
    fn a_failed_contractor_hire_restores_its_pay() {
        let mut t1 = funded_sub(20 * USD, 20 * USD);
        Ctx::new(T1, WORKER).deposit(HIRE_MIN_NEAR).set();
        t1.hire(acc(WORKER2), U128(5 * USD), spec(), None);
        assert_eq!(t1.ledger.amount, 15 * USD);
        Ctx::new(T1, T1).results(vec![PromiseResult::Failed]).set();
        t1.on_child_created(0);
        assert_eq!(t1.ledger.amount, 20 * USD);
        assert_eq!(t1.get_job().ft_balance.0, 20 * USD);
    }

    #[test]
    #[should_panic(expected = "only the manager or this job's worker can hire")]
    fn a_stranger_cannot_hire_in_a_sub_job() {
        let mut t1 = funded_sub(20 * USD, 20 * USD);
        Ctx::new(T1, EVE).deposit(HIRE_MIN_NEAR).set();
        t1.hire(acc(WORKER2), U128(5 * USD), spec(), None);
    }

    #[test]
    fn a_contractor_hires_again_after_a_refunded_sub_job() {
        // A contractor (pay 20) hired a sub-agent for 9; the sub-job was
        // fired and sent the 9 back. Its pay is whole and it can hire the
        // runner-up for the same 9.
        let mut t1 = contracted();
        Ctx::new(T1, GC).at(CLOSED).set();
        t1.child_closed(U128(9 * USD), None, None);
        assert_eq!(t1.ledger.amount, 20 * USD);
        assert_eq!(t1.ledger.allocated, 0);
        Ctx::new(T1, WORKER).at(CLOSED).deposit(HIRE_MIN_NEAR).set();
        t1.hire(acc(EVE), U128(9 * USD), spec(), None);
        assert_eq!(t1.ledger.amount, 11 * USD);
        assert_eq!(t1.ledger.allocated, 9 * USD);
    }
}

// ================================================================= bonds

/// Bidders stake NEAR on a slot that asks for it; losers get it back at the
/// award, the winner when its work is paid, and the buyer keeps it if not.
mod bonds {
    use super::*;

    const BOND: NearToken = NearToken::from_near(1);
    const CLOSED: u64 = T0 + 61 * NS_PER_SEC;

    /// (receiver, yocto) of every NEAR transfer created in this context.
    fn transfers() -> Vec<(String, u128)> {
        get_created_receipts()
            .into_iter()
            .flat_map(|r| {
                let to = r.receiver_id.to_string();
                r.actions.into_iter().filter_map(move |a| match a {
                    MockAction::Transfer { deposit, .. } => {
                        Some((to.clone(), deposit.as_yoctonear()))
                    }
                    _ => None,
                })
            })
            .collect()
    }

    fn with_bond(j: &mut Job) -> u32 {
        Ctx::new(ROOT, LEAD).set();
        j.open_bids(
            spec(),
            U128(30 * USD),
            60,
            Some(U128(BOND.as_yoctonear())),
            None,
        )
    }

    fn bid_bonded(j: &mut Job, who: &str, s: u32, price: u128) {
        Ctx::new(ROOT, who)
            .deposit(BID_NEAR.saturating_add(BOND))
            .set();
        j.bid(s, U128(price));
    }

    /// Two bonded bids; WORKER wins at 20. Returns the job (child 0 open).
    fn awarded() -> Job {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        bid_bonded(&mut j, WORKER2, s, 25 * USD);
        Ctx::new(ROOT, LEAD)
            .at(CLOSED)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(s, acc(WORKER), None);
        assert!(
            transfers().contains(&(WORKER2.into(), BOND.as_yoctonear())),
            "the loser's bond goes back at the award"
        );
        assert_eq!(j.children[0].bond, U128(BOND.as_yoctonear()));
        Ctx::new(ROOT, ROOT).at(CLOSED).results(vec![ok(b"")]).set();
        j.on_child_created(0);
        Ctx::new(ROOT, ROOT)
            .at(CLOSED)
            .results(vec![ok(&u128_json(20 * USD))])
            .set();
        assert!(j.on_child_funded(0));
        j
    }

    #[test]
    #[should_panic(expected = "plus the slot's bond")]
    fn a_bid_without_the_bond_is_refused() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        Ctx::new(ROOT, WORKER).deposit(BID_NEAR).set();
        j.bid(s, U128(20 * USD));
    }

    #[test]
    #[should_panic(expected = "bond is at most 5 NEAR")]
    fn a_slot_cannot_ask_for_a_huge_bond() {
        let mut j = funded_root(100 * USD);
        Ctx::new(ROOT, LEAD).set();
        j.open_bids(
            spec(),
            U128(30 * USD),
            60,
            Some(U128(NearToken::from_near(6).as_yoctonear())),
            None,
        );
    }

    #[test]
    fn a_deposit_on_a_rebid_is_added_to_the_bond() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 25 * USD);
        Ctx::new(ROOT, WORKER)
            .deposit(NearToken::from_millinear(7))
            .set();
        j.bid(s, U128(24 * USD));
        let b = j.get_bids(s).unwrap();
        assert_eq!(
            b.bids[0].bond,
            U128(BOND.as_yoctonear() + NearToken::from_millinear(7).as_yoctonear())
        );
        assert_eq!(b.bids[0].price, U128(24 * USD));
    }

    #[test]
    fn a_paid_worker_gets_its_bond_back() {
        let mut j = awarded();
        Ctx::new(ROOT, T1).at(CLOSED).set();
        j.child_closed(U128(0), Some(true), None);
        assert!(transfers().contains(&(WORKER.into(), BOND.as_yoctonear())));
        assert_eq!(events("bond_returned").len(), 1);
    }

    #[test]
    fn junk_loses_its_bond_to_the_buyer() {
        let mut j = awarded();
        Ctx::new(ROOT, T1).at(CLOSED).set();
        j.child_closed(U128(20 * USD), Some(false), None);
        assert!(transfers().contains(&(LEAD.into(), BOND.as_yoctonear())));
        assert_eq!(events("bond_kept").len(), 1);
        assert_eq!(j.children[0].bond, U128(0));
    }

    // ------------------------------------------------------ underwriters

    const BACKING: NearToken = NearToken::from_near(4);
    const PREMIUM: u128 = BOND.as_yoctonear() / 5;

    /// WORKER bids, seeks a backer for PREMIUM, EVE backs it with 4 NEAR;
    /// WORKER2 bids too. WORKER wins. Returns the job (child 0 open).
    fn backed_award() -> Job {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        bid_bonded(&mut j, WORKER2, s, 25 * USD);
        Ctx::new(ROOT, WORKER).set();
        j.seek_backer(s, U128(PREMIUM));
        Ctx::new(ROOT, EVE).deposit(BACKING).set();
        j.back(s, acc(WORKER));
        let b = j.get_bids(s).unwrap();
        assert_eq!(b.bids[0].backer.as_ref().map(|a| a.as_str()), Some(EVE));
        assert_eq!(b.bids[0].backed.0, BACKING.as_yoctonear());
        Ctx::new(ROOT, LEAD)
            .at(CLOSED)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(s, acc(WORKER), None);
        Ctx::new(ROOT, ROOT).at(CLOSED).results(vec![ok(b"")]).set();
        j.on_child_created(0);
        Ctx::new(ROOT, ROOT)
            .at(CLOSED)
            .results(vec![ok(&u128_json(20 * USD))])
            .set();
        assert!(j.on_child_funded(0));
        j
    }

    #[test]
    fn a_backer_of_paid_work_gets_its_near_back_plus_the_premium() {
        let mut j = backed_award();
        Ctx::new(ROOT, T1).at(CLOSED).set();
        j.child_closed(U128(0), Some(true), None);
        let t = transfers();
        assert!(t.contains(&(EVE.into(), BACKING.as_yoctonear() + PREMIUM)));
        assert!(t.contains(&(WORKER.into(), BOND.as_yoctonear() - PREMIUM)));
        assert_eq!(events("backer_paid").len(), 1);
    }

    #[test]
    fn a_backer_of_junk_loses_its_near_to_the_buyer() {
        let mut j = backed_award();
        Ctx::new(ROOT, T1).at(CLOSED).set();
        j.child_closed(U128(20 * USD), Some(false), None);
        assert!(transfers().contains(&(LEAD.into(), BOND.as_yoctonear() + BACKING.as_yoctonear())));
        assert!(!transfers().iter().any(|(to, _)| to == EVE));
    }

    #[test]
    fn a_backer_of_a_losing_bid_is_refunded_at_the_award() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        bid_bonded(&mut j, WORKER2, s, 25 * USD);
        Ctx::new(ROOT, WORKER2).set();
        j.seek_backer(s, U128(PREMIUM));
        Ctx::new(ROOT, EVE).deposit(BACKING).set();
        j.back(s, acc(WORKER2));
        Ctx::new(ROOT, LEAD)
            .at(CLOSED)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(s, acc(WORKER), None);
        let t = transfers();
        assert!(t.contains(&(EVE.into(), BACKING.as_yoctonear())));
        assert!(t.contains(&(WORKER2.into(), BOND.as_yoctonear())));
    }

    #[test]
    #[should_panic(expected = "that bid does not seek a backer")]
    fn nobody_backs_a_bid_that_did_not_ask() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        Ctx::new(ROOT, EVE).deposit(BACKING).set();
        j.back(s, acc(WORKER));
    }

    #[test]
    #[should_panic(expected = "the premium is at most half your bond")]
    fn a_premium_above_half_the_bond_is_refused() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        Ctx::new(ROOT, WORKER).set();
        j.seek_backer(s, U128(BOND.as_yoctonear() / 2 + 1));
    }

    #[test]
    #[should_panic(expected = "you cannot back your own bid")]
    fn a_bidder_cannot_back_itself() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        Ctx::new(ROOT, WORKER).set();
        j.seek_backer(s, U128(PREMIUM));
        Ctx::new(ROOT, WORKER).deposit(BACKING).set();
        j.back(s, acc(WORKER));
    }

    #[test]
    fn a_failed_hire_returns_the_bond() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        Ctx::new(ROOT, LEAD)
            .at(CLOSED)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(s, acc(WORKER), None);
        Ctx::new(ROOT, ROOT)
            .at(CLOSED)
            .results(vec![PromiseResult::Failed])
            .set();
        j.on_child_created(0);
        assert!(transfers().contains(&(WORKER.into(), BOND.as_yoctonear())));
    }

    #[test]
    fn bonds_on_a_slot_never_awarded_come_back() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        Ctx::new(ROOT, WORKER).at(AFTER).set();
        j.reclaim_bond(s);
        assert!(transfers().contains(&(WORKER.into(), BOND.as_yoctonear())));
        assert_eq!(j.get_bids(s).unwrap().bids[0].bond, U128(0));
    }

    #[test]
    #[should_panic(expected = "the window is still open")]
    fn a_bond_cannot_be_reclaimed_while_the_job_runs() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        Ctx::new(ROOT, WORKER).at(CLOSED).set();
        j.reclaim_bond(s);
    }

    #[test]
    fn settle_returns_bonds_nobody_reclaimed() {
        let mut j = funded_root(100 * USD);
        let s = with_bond(&mut j);
        bid_bonded(&mut j, WORKER, s, 20 * USD);
        Ctx::new(ROOT, EVE).at(AFTER).set();
        j.settle();
        assert!(transfers().contains(&(WORKER.into(), BOND.as_yoctonear())));
    }
}

// ================================================================ sealed

/// Sealed bids: commit a hash while bidding is open, reveal after it
/// closes; only revealed bids can win.
mod sealed {
    use super::*;

    const OPEN_END: u64 = T0 + 61 * NS_PER_SEC;
    const REVEAL_END: u64 = T0 + 121 * NS_PER_SEC;

    fn commit(price: u128, salt: &str, who: &str) -> String {
        hex_of(&env::sha256_array(
            format!("{price}:{salt}:{who}").as_bytes(),
        ))
    }

    fn sealed_slot(j: &mut Job) -> u32 {
        Ctx::new(ROOT, LEAD).set();
        j.open_bids(spec(), U128(30 * USD), 60, None, Some(true))
    }

    fn seal(j: &mut Job, who: &str, s: u32, price: u128, salt: &str) {
        Ctx::new(ROOT, who).deposit(SEALED_BID_NEAR).set();
        j.bid_sealed(s, commit(price, salt, who));
    }

    #[test]
    fn sealed_bids_hide_the_price_until_the_reveal() {
        let mut j = funded_root(100 * USD);
        let s = sealed_slot(&mut j);
        seal(&mut j, WORKER, s, 20 * USD, "a1");
        seal(&mut j, WORKER2, s, 18 * USD, "b2");
        let b = j.get_bids(s).unwrap();
        assert!(b
            .bids
            .iter()
            .all(|b| b.price == U128(0) && b.commit.is_some()));
        Ctx::new(ROOT, WORKER).at(OPEN_END).set();
        j.reveal(s, U128(20 * USD), "a1".into());
        Ctx::new(ROOT, WORKER2).at(OPEN_END).set();
        j.reveal(s, U128(18 * USD), "b2".into());
        Ctx::new(ROOT, LEAD)
            .at(REVEAL_END)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(s, acc(WORKER2), None);
        assert_eq!(j.get_job().children[0].total, U128(18 * USD));
    }

    #[test]
    fn a_sealed_bid_pays_for_its_own_storage() {
        let mut j = funded_root(100 * USD);
        let s = sealed_slot(&mut j);
        let before = env::storage_usage();
        seal(&mut j, WORKER, s, 20 * USD, "a1");
        let used = (env::storage_usage() - before) as u128;
        assert!(
            used * env::storage_byte_cost().as_yoctonear() <= SEALED_BID_NEAR.as_yoctonear(),
            "a sealed bid uses {used} bytes"
        );
    }

    #[test]
    #[should_panic(expected = "attach 0.002 NEAR for the sealed bid's storage")]
    fn a_sealed_bid_with_the_open_deposit_is_refused() {
        let mut j = funded_root(100 * USD);
        let s = sealed_slot(&mut j);
        Ctx::new(ROOT, WORKER).deposit(BID_NEAR).set();
        j.bid_sealed(s, commit(20 * USD, "a1", WORKER));
    }

    #[test]
    #[should_panic(expected = "price and salt do not match the commit")]
    fn a_reveal_must_match_the_commit() {
        let mut j = funded_root(100 * USD);
        let s = sealed_slot(&mut j);
        seal(&mut j, WORKER, s, 20 * USD, "a1");
        Ctx::new(ROOT, WORKER).at(OPEN_END).set();
        j.reveal(s, U128(19 * USD), "a1".into());
    }

    #[test]
    #[should_panic(expected = "bidding is still open: reveal after it closes")]
    fn no_reveal_while_bidding_is_open() {
        let mut j = funded_root(100 * USD);
        let s = sealed_slot(&mut j);
        seal(&mut j, WORKER, s, 20 * USD, "a1");
        Ctx::new(ROOT, WORKER).set();
        j.reveal(s, U128(20 * USD), "a1".into());
    }

    #[test]
    #[should_panic(expected = "the reveal is still open")]
    fn no_award_before_the_reveal_closes() {
        let mut j = funded_root(100 * USD);
        let s = sealed_slot(&mut j);
        seal(&mut j, WORKER, s, 20 * USD, "a1");
        Ctx::new(ROOT, WORKER).at(OPEN_END).set();
        j.reveal(s, U128(20 * USD), "a1".into());
        Ctx::new(ROOT, LEAD)
            .at(OPEN_END)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(s, acc(WORKER), None);
    }

    #[test]
    #[should_panic(expected = "that bid was never revealed")]
    fn an_unrevealed_bid_cannot_win() {
        let mut j = funded_root(100 * USD);
        let s = sealed_slot(&mut j);
        seal(&mut j, WORKER, s, 20 * USD, "a1");
        Ctx::new(ROOT, LEAD)
            .at(REVEAL_END)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(s, acc(WORKER), None);
    }

    #[test]
    #[should_panic(expected = "this slot is sealed: use bid_sealed")]
    fn an_open_bid_on_a_sealed_slot_is_refused() {
        let mut j = funded_root(100 * USD);
        let s = sealed_slot(&mut j);
        Ctx::new(ROOT, WORKER).deposit(BID_NEAR).set();
        j.bid(s, U128(20 * USD));
    }

    #[test]
    fn an_unrevealed_bond_goes_to_the_buyer() {
        let mut j = funded_root(100 * USD);
        Ctx::new(ROOT, LEAD).set();
        let bond = NearToken::from_near(1);
        let s = j.open_bids(
            spec(),
            U128(30 * USD),
            60,
            Some(U128(bond.as_yoctonear())),
            Some(true),
        );
        Ctx::new(ROOT, WORKER)
            .deposit(SEALED_BID_NEAR.saturating_add(bond))
            .set();
        j.bid_sealed(s, commit(20 * USD, "a1", WORKER));
        Ctx::new(ROOT, WORKER2)
            .deposit(SEALED_BID_NEAR.saturating_add(bond))
            .set();
        j.bid_sealed(s, commit(18 * USD, "b2", WORKER2));
        Ctx::new(ROOT, WORKER).at(OPEN_END).set();
        j.reveal(s, U128(20 * USD), "a1".into());
        Ctx::new(ROOT, LEAD)
            .at(REVEAL_END)
            .deposit(NearToken::from_millinear(50))
            .set();
        j.award(s, acc(WORKER), None);
        let lead_got: u128 = get_created_receipts()
            .into_iter()
            .filter(|r| r.receiver_id.as_str() == LEAD)
            .flat_map(|r| r.actions)
            .filter_map(|a| match a {
                MockAction::Transfer { deposit, .. } => Some(deposit.as_yoctonear()),
                _ => None,
            })
            .sum();
        assert_eq!(lead_got, bond.as_yoctonear());
    }
}

// ====================================================== review fixes (2026-11-09)

mod late_bids_and_unfunded_roots {
    use super::*;

    #[test]
    #[should_panic(expected = "the job no longer takes bids")]
    fn a_bid_after_approve_is_refused() {
        let mut j = funded_root(100 * USD);
        Ctx::new(ROOT, LEAD).set();
        let s = j.open_bids(
            spec(),
            U128(30 * USD),
            600,
            Some(U128(NearToken::from_near(1).as_yoctonear())),
            None,
        );
        Ctx::new(ROOT, CLIENT).set();
        j.approve();
        Ctx::new(ROOT, WORKER)
            .deposit(NearToken::from_millinear(1001))
            .set();
        j.bid(s, U128(5 * USD));
    }

    #[test]
    #[should_panic(expected = "the job no longer takes bids")]
    fn a_bid_while_settling_is_refused() {
        let mut j = funded_root(100 * USD);
        Ctx::new(ROOT, LEAD).set();
        let s = j.open_bids(spec(), U128(30 * USD), 600, None, None);
        Ctx::new(ROOT, CLIENT).set();
        j.approve();
        Ctx::new(ROOT, EVE).set();
        j.settle();
        Ctx::new(ROOT, WORKER)
            .deposit(NearToken::from_millinear(1))
            .set();
        j.bid(s, U128(5 * USD));
    }

    #[test]
    fn a_root_that_was_never_funded_still_settles() {
        Ctx::new(ROOT, FACTORY).set();
        let mut j = Job::new(
            Some(acc(CLIENT)),
            acc(LEAD),
            None,
            acc(TOKEN),
            U128(10 * USD),
            spec(),
            1,
            signer(),
            code_hash(),
            None,
            None,
            None,
            None,
            None,
        );
        // The fund arrives after the 1 s window and is refused.
        Ctx::new(ROOT, TOKEN).at(T0 + 2 * NS_PER_SEC).set();
        assert_eq!(
            value(j.ft_on_transfer(acc(FACTORY), U128(100 * USD), "fund".into())),
            100 * USD
        );
        Ctx::new(ROOT, EVE).at(AFTER).set();
        j.settle();
        assert_ne!(j.get_job().status, "Open");
    }
}

// ================================================================ award_best

/// The award rule on-chain: the job reads each revealed bidder's record from
/// the registry, then a callback hires the bid `ht_core::pick` chooses.
mod award_best {
    use super::*;

    const BID_SECS: u64 = 60;
    const CLOSED: u64 = T0 + (BID_SECS + 1) * NS_PER_SEC;
    const REVEAL_END: u64 = T0 + (2 * BID_SECS + 1) * NS_PER_SEC;
    const DEPOSIT: NearToken = NearToken::from_millinear(50);

    fn open(j: &mut Job, bond: Option<NearToken>) -> u32 {
        Ctx::new(ROOT, LEAD).set();
        j.open_bids(
            spec(),
            U128(30 * USD),
            BID_SECS,
            bond.map(|b| U128(b.as_yoctonear())),
            None,
        )
    }

    fn bid(j: &mut Job, who: &str, slot: u32, price: u128, extra: NearToken) {
        Ctx::new(ROOT, who)
            .deposit(BID_NEAR.saturating_add(extra))
            .set();
        j.bid(slot, U128(price));
    }

    /// A factory `Record` reply: (paid, unpaid) in whole fUSD.
    fn rec(paid: u128, unpaid: u128) -> PromiseResult {
        ok(format!(
            r#"{{"jobs":{},"delivered":0,"checked":0,"junk":0,"paid":0,"score_sum":0,"scored":0,"paid_value":"{}","unpaid_value":"{}","payers":1}}"#,
            paid + unpaid,
            paid * USD,
            unpaid * USD
        )
        .as_bytes())
    }

    fn start(j: &mut Job, slot: u32, weight: u32, pct: u32) {
        Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
        j.award_best(slot, weight, pct, None, None, None, None);
    }

    fn finish(
        j: &mut Job,
        slot: u32,
        workers: &[&str],
        weight: u32,
        pct: u32,
        results: Vec<PromiseResult>,
    ) -> PromiseOrValue<bool> {
        Ctx::new(ROOT, ROOT).at(CLOSED).results(results).set();
        j.on_award_best(
            slot,
            workers.iter().map(|w| acc(w)).collect(),
            weight,
            pct,
            None,
            false,
            acc(LEAD),
            U128(DEPOSIT.as_yoctonear()),
            None,
            None,
        )
    }

    fn refunded() -> bool {
        get_created_receipts().iter().any(|r| {
            r.receiver_id == acc(LEAD)
                && r.actions.iter().any(|a| {
                    matches!(a, MockAction::Transfer { deposit, .. }
                        if deposit.as_yoctonear() == DEPOSIT.as_yoctonear())
                })
        })
    }

    /// WORKER bids 25, WORKER2 bids 18.
    fn two_bids() -> (Job, u32) {
        let mut j = funded_root(100 * USD);
        let s = open(&mut j, None);
        bid(&mut j, WORKER, s, 25 * USD, NearToken::from_yoctonear(0));
        bid(&mut j, WORKER2, s, 18 * USD, NearToken::from_yoctonear(0));
        (j, s)
    }

    #[test]
    fn the_job_reads_every_bidders_record_from_the_registry() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 150, 0);
        let reads: Vec<_> = calls()
            .into_iter()
            .filter(|c| c.1 == "get_record")
            .collect();
        assert_eq!(reads.len(), 2);
        assert!(reads.iter().all(|c| c.0 == FACTORY));
        assert_eq!(j.get_bids(s).unwrap().awarded, None, "nothing yet");
        assert!(calls().iter().any(|c| c.1 == "on_award_best"));
    }

    #[test]
    fn a_bad_record_costs_more_than_a_low_bid_saves() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 150, 0);
        // WORKER2's 18 weighs 63 with a record that is all unpaid.
        let r = finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            150,
            0,
            vec![rec(30, 0), rec(0, 20)],
        );
        assert!(matches!(r, PromiseOrValue::Promise(_)));
        drop(r); // a promise is scheduled when it is dropped
        let v = j.get_job();
        assert_eq!(v.children.len(), 1);
        assert_eq!(v.children[0].worker, acc(WORKER));
        assert_eq!(v.children[0].total, U128(25 * USD), "hired at its own bid");
        assert_eq!(j.get_bids(s).unwrap().awarded, Some(acc(WORKER)));
        let e = &events("award_best")[0];
        assert_eq!(e["winner"], WORKER);
        assert_eq!(e["record_weight"], 150);
        assert_eq!(e["inputs"].as_array().unwrap().len(), 2);
        assert_eq!(events("award").len(), 1, "the same award event as by hand");
        assert_eq!(call_args("new")["worker"], WORKER, "the child is created");
    }

    #[test]
    fn with_no_record_rule_the_lowest_bid_wins() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 0, 0);
        finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            0,
            0,
            vec![rec(30, 0), rec(0, 20)],
        );
        assert_eq!(j.get_job().children[0].worker, acc(WORKER2));
    }

    #[test]
    fn a_bond_and_backing_cut_a_newcomers_premium() {
        let bond = NearToken::from_millinear(100);
        let mut j = funded_root(100 * USD);
        let s = open(&mut j, Some(bond));
        // WORKER stakes the bond only; WORKER2 stakes ten times it.
        bid(&mut j, WORKER, s, 13 * USD, bond);
        bid(
            &mut j,
            WORKER2,
            s,
            18 * USD,
            NearToken::from_yoctonear(10 * bond.as_yoctonear()),
        );
        start(&mut j, s, 150, 100);
        finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            150,
            100,
            vec![rec(0, 0), rec(0, 0)],
        );
        let e = &events("award_best")[0];
        assert_eq!(e["inputs"][1]["vouch"], "10");
        assert_eq!(j.get_job().children[0].worker, acc(WORKER2));
    }

    #[test]
    fn a_failed_record_read_awards_nobody_and_refunds() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 150, 0);
        let r = finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            150,
            0,
            vec![rec(30, 0), PromiseResult::Failed],
        );
        assert!(matches!(r, PromiseOrValue::Value(false)));
        assert_eq!(j.get_bids(s).unwrap().awarded, None);
        assert!(j.get_job().children.is_empty());
        assert_eq!(j.get_job().allocated.0, 0);
        assert!(refunded(), "the deposit goes back to the buyer");
        assert_eq!(events("award_best_failed").len(), 1);
        // The slot is still open for a retry.
        let r = finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            150,
            0,
            vec![rec(30, 0), rec(30, 0)],
        );
        assert!(matches!(r, PromiseOrValue::Promise(_)));
        assert_eq!(j.get_bids(s).unwrap().awarded, Some(acc(WORKER2)));
    }

    #[test]
    fn a_garbled_record_fails_closed() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 150, 0);
        let r = finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            150,
            0,
            vec![rec(30, 0), ok(b"not json")],
        );
        assert!(matches!(r, PromiseOrValue::Value(false)));
        assert_eq!(j.get_bids(s).unwrap().awarded, None);
        assert!(refunded());
    }

    #[test]
    fn a_hand_award_between_the_read_and_the_callback_wins_the_race() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 0, 0);
        Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
        j.award(s, acc(WORKER), None);
        let r = finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            0,
            0,
            vec![rec(0, 0), rec(0, 0)],
        );
        assert!(matches!(r, PromiseOrValue::Value(false)));
        let v = j.get_job();
        assert_eq!(v.children.len(), 1, "awarded once");
        assert_eq!(v.children[0].worker, acc(WORKER));
        assert!(refunded());
        assert_eq!(
            events("award_best_failed")[0]["reason"],
            "slot already awarded"
        );
    }

    #[test]
    fn two_award_bests_award_the_slot_once() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 0, 0);
        start(&mut j, s, 150, 0);
        let first = finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            0,
            0,
            vec![rec(0, 0), rec(0, 0)],
        );
        assert!(matches!(first, PromiseOrValue::Promise(_)));
        let second = finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            150,
            0,
            vec![rec(0, 0), rec(0, 0)],
        );
        assert!(matches!(second, PromiseOrValue::Value(false)));
        assert_eq!(j.get_job().children.len(), 1);
    }

    #[test]
    fn a_hand_award_after_award_best_is_refused() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 0, 0);
        finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            0,
            0,
            vec![rec(0, 0), rec(0, 0)],
        );
        Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            j.award(s, acc(WORKER), None)
        }));
        assert!(r.is_err());
        assert_eq!(j.get_job().children.len(), 1);
    }

    #[test]
    fn skipped_bidders_are_not_read_or_awarded() {
        let (mut j, s) = two_bids();
        Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
        j.award_best(s, 0, 0, None, Some(vec![acc(WORKER2)]), None, None);
        let args: Vec<_> = get_created_receipts()
            .into_iter()
            .flat_map(|r| r.actions)
            .filter_map(|a| match a {
                MockAction::FunctionCallWeight {
                    method_name, args, ..
                } if method_name == b"get_record" => Some(String::from_utf8(args).unwrap()),
                _ => None,
            })
            .collect();
        assert_eq!(args.len(), 1);
        assert!(args[0].contains(WORKER) && !args[0].contains(WORKER2));
        assert_eq!(call_args("on_award_best")["workers"], json!([WORKER]));
    }

    #[test]
    fn unrevealed_sealed_bids_are_not_candidates() {
        let mut j = funded_root(100 * USD);
        Ctx::new(ROOT, LEAD).set();
        let s = j.open_bids(spec(), U128(30 * USD), BID_SECS, None, Some(true));
        let commit = |price: u128, salt: &str, who: &str| {
            hex_of(&env::sha256_array(
                format!("{price}:{salt}:{who}").as_bytes(),
            ))
        };
        Ctx::new(ROOT, WORKER).deposit(SEALED_BID_NEAR).set();
        j.bid_sealed(s, commit(20 * USD, "a", WORKER));
        Ctx::new(ROOT, WORKER2).deposit(SEALED_BID_NEAR).set();
        j.bid_sealed(s, commit(10 * USD, "b", WORKER2));
        Ctx::new(ROOT, WORKER).at(CLOSED).set();
        j.reveal(s, U128(20 * USD), "a".into());
        Ctx::new(ROOT, LEAD).at(REVEAL_END).deposit(DEPOSIT).set();
        j.award_best(s, 0, 0, None, None, None, None);
        assert_eq!(call_args("on_award_best")["workers"], json!([WORKER]));
    }

    #[test]
    #[should_panic(expected = "no revealed bid can be awarded")]
    fn award_best_with_nothing_revealed_is_refused() {
        let mut j = funded_root(100 * USD);
        Ctx::new(ROOT, LEAD).set();
        let s = j.open_bids(spec(), U128(30 * USD), BID_SECS, None, Some(true));
        Ctx::new(ROOT, WORKER).deposit(SEALED_BID_NEAR).set();
        j.bid_sealed(s, "ab".repeat(32));
        Ctx::new(ROOT, LEAD).at(REVEAL_END).deposit(DEPOSIT).set();
        j.award_best(s, 0, 0, None, None, None, None);
    }

    #[test]
    #[should_panic(expected = "bidding is still open")]
    fn award_best_before_the_close_is_refused() {
        let (mut j, s) = two_bids();
        Ctx::new(ROOT, LEAD).deposit(DEPOSIT).set();
        j.award_best(s, 0, 0, None, None, None, None);
    }

    #[test]
    #[should_panic(expected = "only the manager or this job's worker can award")]
    fn only_the_buyer_can_award_best() {
        let (mut j, s) = two_bids();
        Ctx::new(ROOT, EVE).at(CLOSED).deposit(DEPOSIT).set();
        j.award_best(s, 0, 0, None, None, None, None);
    }

    #[test]
    #[should_panic(expected = "attach at least 0.035 NEAR")]
    fn award_best_needs_the_childs_storage_deposit() {
        let (mut j, s) = two_bids();
        Ctx::new(ROOT, LEAD).at(CLOSED).set();
        j.award_best(s, 0, 0, None, None, None, None);
    }

    #[test]
    #[should_panic(expected = "not enough gas for award_best")]
    fn award_best_needs_gas_for_every_read() {
        let (mut j, s) = two_bids();
        Ctx::new(ROOT, LEAD)
            .at(CLOSED)
            .deposit(DEPOSIT)
            .gas(Gas::from_tgas(170))
            .set();
        j.award_best(s, 0, 0, None, None, None, None);
    }

    #[test]
    fn the_most_bids_a_slot_takes_fit_in_the_gas_cap() {
        let need = GAS_HIRE_MIN.as_gas()
            + GAS_AWARD_OWN.as_gas()
            + GAS_GET_RECORD.as_gas() * MAX_BIDS as u64;
        assert!(need <= Gas::from_tgas(300).as_gas());
    }

    #[test]
    #[should_panic(expected = "callback: caller must be this contract")]
    fn on_award_best_is_private() {
        let (mut j, s) = two_bids();
        Ctx::new(ROOT, EVE).at(CLOSED).set();
        j.on_award_best(
            s,
            vec![acc(WORKER)],
            0,
            0,
            None,
            false,
            acc(LEAD),
            U128(DEPOSIT.as_yoctonear()),
            None,
            None,
        );
    }

    #[test]
    fn a_job_that_cannot_fund_the_hire_refunds_instead_of_stranding() {
        // 30 fUSD funded; a 25 fUSD winner plus a 10 fUSD sub-budget is too much.
        let mut j = funded_root(30 * USD);
        let s = open(&mut j, None);
        bid(&mut j, WORKER, s, 25 * USD, NearToken::from_yoctonear(0));
        Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
        j.award_best(s, 0, 0, Some(U128(10 * USD)), None, None, None);
        Ctx::new(ROOT, ROOT)
            .at(CLOSED)
            .results(vec![rec(0, 0)])
            .set();
        let r = j.on_award_best(
            s,
            vec![acc(WORKER)],
            0,
            0,
            Some(U128(10 * USD)),
            false,
            acc(LEAD),
            U128(DEPOSIT.as_yoctonear()),
            None,
            None,
        );
        assert!(matches!(r, PromiseOrValue::Value(false)));
        assert_eq!(j.get_bids(s).unwrap().awarded, None);
        assert!(refunded());
    }

    // ------------------------------------------------ security review 6

    /// 30 s before the root's window ends.
    const LATE: u64 = T0 + (WINDOW - 30) * NS_PER_SEC;

    #[test]
    #[should_panic(expected = "too late to award")]
    fn a_late_award_cannot_take_the_winners_bond() {
        // The child's window is what is left of this job's. Awarded in the
        // last seconds, the winner could not deliver and the buyer would
        // keep its bond (and its backer's NEAR).
        let bond = NearToken::from_near(5);
        let mut j = funded_root(100 * USD);
        let s = open(&mut j, Some(bond));
        bid(&mut j, WORKER, s, 20 * USD, bond);
        Ctx::new(ROOT, LEAD).at(LATE).deposit(DEPOSIT).set();
        j.award(s, acc(WORKER), None);
    }

    #[test]
    #[should_panic(expected = "too late to award")]
    fn a_late_award_best_is_refused() {
        let (mut j, s) = two_bids();
        Ctx::new(ROOT, LEAD).at(LATE).deposit(DEPOSIT).set();
        j.award_best(s, 0, 0, None, None, None, None);
    }

    #[test]
    fn an_award_best_callback_that_lands_too_late_refunds() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 0, 0);
        Ctx::new(ROOT, ROOT)
            .at(LATE)
            .results(vec![rec(0, 0), rec(0, 0)])
            .set();
        let r = j.on_award_best(
            s,
            vec![acc(WORKER), acc(WORKER2)],
            0,
            0,
            None,
            false,
            acc(LEAD),
            U128(DEPOSIT.as_yoctonear()),
            None,
            None,
        );
        assert!(matches!(r, PromiseOrValue::Value(false)));
        assert_eq!(j.get_bids(s).unwrap().awarded, None);
        assert!(refunded());
    }

    #[test]
    fn an_award_leaves_the_winner_at_least_a_minute() {
        let (mut j, s) = two_bids();
        let ok_at = T0 + (WINDOW - MIN_WORK_SECS) * NS_PER_SEC;
        Ctx::new(ROOT, LEAD).at(ok_at).deposit(DEPOSIT).set();
        j.award(s, acc(WORKER), None);
        assert_eq!(call_args("new")["window_secs"], MIN_WORK_SECS);
    }

    #[test]
    fn an_award_tells_the_child_it_was_won_by_a_bid() {
        let (mut j, s) = two_bids();
        Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
        j.award(s, acc(WORKER), None);
        assert_eq!(call_args("new")["bid"], true);
        let mut j = funded_root(100 * USD);
        hire_one(&mut j, WORKER, 10 * USD);
        assert_eq!(call_args("new")["bid"], false);
    }

    /// A sub-job nobody delivered to, settled and deleted: its record call.
    fn lapsed_record(bid: Option<bool>) -> serde_json::Value {
        Ctx::new(T1, ROOT).set();
        let mut j = Job::new(
            None,
            acc(LEAD),
            Some(acc(WORKER)),
            acc(TOKEN),
            U128(30 * USD),
            spec(),
            WINDOW,
            signer(),
            code_hash(),
            None,
            None,
            None,
            None,
            bid,
        );
        Ctx::new(T1, TOKEN).set();
        j.ft_on_transfer(acc(ROOT), U128(30 * USD), "fund".into());
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        rest_cb(&mut j, T1, true);
        balance_cb(&mut j, T1, ok(&u128_json(0)));
        call_args("record")
    }

    #[test]
    fn a_hire_nobody_bid_for_leaves_no_unpaid_mark() {
        // Anyone can `hire` any account without asking it. Left to lapse,
        // that must not write unpaid money on the victim's record.
        let r = lapsed_record(None);
        assert_eq!(r["worker"], WORKER);
        assert_eq!(r["paid"], false);
        assert_eq!(r["delivered"], false);
        assert_eq!(r["amount"], "0");
    }

    #[test]
    fn a_bid_winner_that_never_delivers_is_marked_unpaid() {
        let r = lapsed_record(Some(true));
        assert_eq!(r["paid"], false);
        assert_eq!(r["amount"], (30 * USD).to_string());
    }

    #[test]
    fn paid_money_below_min_paid_does_not_make_a_record() {
        // WORKER2 washed 1 fUSD through a job it paid itself. With no
        // floor that is a record and its 18 beats 25; with a 10 fUSD floor
        // it is a newcomer again and pays the premium (18 x 2 = 36).
        let run = |min_paid: Option<U128>| {
            let (mut j, s) = two_bids();
            Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
            j.award_best(s, 150, 100, None, None, min_paid, None);
            assert_eq!(call_args("on_award_best")["min_paid"], json!(min_paid));
            Ctx::new(ROOT, ROOT)
                .at(CLOSED)
                .results(vec![rec(30, 0), rec(1, 0)])
                .set();
            j.on_award_best(
                s,
                vec![acc(WORKER), acc(WORKER2)],
                150,
                100,
                None,
                false,
                acc(LEAD),
                U128(DEPOSIT.as_yoctonear()),
                min_paid,
                None,
            );
            j.get_job().children[0].worker.clone()
        };
        assert_eq!(run(None), acc(WORKER2));
        assert_eq!(run(Some(U128(10 * USD))), acc(WORKER));
    }

    #[test]
    fn a_thin_record_with_unpaid_money_keeps_its_penalty() {
        // Ignoring thin paid money never helps a bad record: 1 paid and 5
        // unpaid under a 10 fUSD floor weighs all unpaid (18 x 2.5 = 45).
        let (mut j, s) = two_bids();
        start(&mut j, s, 150, 0);
        Ctx::new(ROOT, ROOT)
            .at(CLOSED)
            .results(vec![rec(30, 0), rec(1, 5)])
            .set();
        j.on_award_best(
            s,
            vec![acc(WORKER), acc(WORKER2)],
            150,
            0,
            None,
            false,
            acc(LEAD),
            U128(DEPOSIT.as_yoctonear()),
            Some(U128(10 * USD)),
            None,
        );
        let e = &events("award_best")[0];
        assert_eq!(e["inputs"][1]["score"], (45 * USD).to_string());
        assert_eq!(e["min_paid"], (10 * USD).to_string());
        assert_eq!(j.get_job().children[0].worker, acc(WORKER));
    }

    #[test]
    fn yocto_on_a_slot_without_a_bond_do_not_cut_the_premium() {
        // No bond asked: 1000 yocto attached is not "1000x the bond".
        let mut j = funded_root(100 * USD);
        let s = open(&mut j, None);
        bid(&mut j, WORKER, s, 18 * USD, NearToken::from_yoctonear(0));
        bid(
            &mut j,
            WORKER2,
            s,
            13 * USD,
            NearToken::from_yoctonear(1_000),
        );
        start(&mut j, s, 150, 75);
        finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            150,
            75,
            vec![rec(30, 0), rec(0, 0)],
        );
        assert_eq!(events("award_best")[0]["inputs"][1]["vouch"], "1");
        assert_eq!(j.get_job().children[0].worker, acc(WORKER));
    }

    #[test]
    fn the_skip_list_is_in_the_event() {
        let (mut j, s) = two_bids();
        Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
        j.award_best(s, 150, 0, None, Some(vec![acc(WORKER2)]), None, None);
        let e = &events("award_best_asked")[0];
        assert_eq!(e["slot"], s);
        assert_eq!(e["skip"], json!([WORKER2]));
        assert_eq!(e["reading"], json!([WORKER]));
    }

    #[test]
    fn a_child_id_too_long_refunds_instead_of_stranding() {
        // A job deep enough that `t1.<job>` passes 64 bytes: the callback
        // must refund, not panic with the deposit left in the job.
        let long = format!("{}.{FACTORY}", "a".repeat(63 - FACTORY.len()));
        assert!(long.len() == 64 && long.parse::<AccountId>().is_ok());
        Ctx::new(&long, FACTORY).set();
        let mut j = Job::new(
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
            None,
            None,
            None,
        );
        Ctx::new(&long, TOKEN).set();
        j.ft_on_transfer(acc(CLIENT), U128(100 * USD), "fund".into());
        Ctx::new(&long, LEAD).set();
        let s = j.open_bids(spec(), U128(30 * USD), BID_SECS, None, None);
        Ctx::new(&long, WORKER).deposit(BID_NEAR).set();
        j.bid(s, U128(20 * USD));
        Ctx::new(&long, &long)
            .at(CLOSED)
            .results(vec![rec(0, 0)])
            .set();
        let r = j.on_award_best(
            s,
            vec![acc(WORKER)],
            0,
            0,
            None,
            false,
            acc(LEAD),
            U128(DEPOSIT.as_yoctonear()),
            None,
            None,
        );
        assert!(matches!(r, PromiseOrValue::Value(false)));
        assert_eq!(j.get_bids(s).unwrap().awarded, None);
        assert!(refunded());
    }

    // ---- the buyer's payer list

    /// `payers` as the buyer passes them (the lead's own managers).
    fn list(ids: &[&str]) -> Option<Vec<AccountId>> {
        Some(ids.iter().map(|x| acc(x)).collect())
    }

    /// A factory `get_record_from` reply: listed pay (whole fUSD) from `n` payers.
    fn rec_from(paid: u128, unpaid: u128, n: u32) -> PromiseResult {
        ok(format!(
            r#"{{"jobs":{},"delivered":0,"checked":0,"junk":0,"paid":0,"score_sum":0,"scored":0,"paid_value":"{}","unpaid_value":"{}","payers":{n}}}"#,
            paid + unpaid,
            paid * USD,
            unpaid * USD
        )
        .as_bytes())
    }

    fn start_listed(j: &mut Job, s: u32, payers: Option<Vec<AccountId>>, min_paid: Option<U128>) {
        Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
        j.award_best(s, 150, 100, None, None, min_paid, payers);
    }

    fn finish_listed(
        j: &mut Job,
        s: u32,
        payers: Option<Vec<AccountId>>,
        min_paid: Option<U128>,
        results: Vec<PromiseResult>,
    ) {
        Ctx::new(ROOT, ROOT).at(CLOSED).results(results).set();
        j.on_award_best(
            s,
            vec![acc(WORKER), acc(WORKER2)],
            150,
            100,
            None,
            false,
            acc(LEAD),
            U128(DEPOSIT.as_yoctonear()),
            min_paid,
            payers,
        );
    }

    #[test]
    fn a_payer_list_makes_the_job_read_the_record_from_those_payers() {
        let (mut j, s) = two_bids();
        start_listed(&mut j, s, list(&[LEAD, EVE]), None);
        let c = calls();
        let reads: Vec<_> = c.iter().filter(|c| c.1 == "get_record_from").collect();
        assert_eq!(reads.len(), 2);
        assert!(reads.iter().all(|c| c.0 == FACTORY));
        assert!(!c.iter().any(|c| c.1 == "get_record"));
        let a = call_args("get_record_from");
        assert_eq!(a["payers"], json!([LEAD, EVE]));
        assert_eq!(a["account_id"], WORKER);
        assert_eq!(call_args("on_award_best")["payers"], json!([LEAD, EVE]));
    }

    #[test]
    fn without_a_payer_list_the_reads_and_events_are_as_before() {
        let (mut j, s) = two_bids();
        start(&mut j, s, 150, 100);
        assert!(calls().iter().all(|c| c.1 != "get_record_from"));
        assert!(call_args("get_record").get("payers").is_none());
        assert_eq!(call_args("on_award_best")["payers"], json!(null));
        assert!(events("award_best_asked")[0].get("payers").is_none());
        finish(
            &mut j,
            s,
            &[WORKER, WORKER2],
            150,
            100,
            vec![rec(30, 0), rec(30, 0)],
        );
        let e = &events("award_best")[0];
        assert!(e.get("payers").is_none());
        assert!(e["inputs"][0].get("listed_paid_value").is_none());
    }

    #[test]
    fn the_payer_list_and_what_was_read_are_in_the_events() {
        let (mut j, s) = two_bids();
        start_listed(&mut j, s, list(&[LEAD]), None);
        assert_eq!(events("award_best_asked")[0]["payers"], json!([LEAD]));
        finish_listed(
            &mut j,
            s,
            list(&[LEAD]),
            None,
            vec![rec_from(30, 0, 2), rec_from(0, 0, 0)],
        );
        let e = &events("award_best")[0];
        assert_eq!(e["payers"], json!([LEAD]));
        assert_eq!(e["inputs"][0]["listed_paid_value"], (30 * USD).to_string());
        assert_eq!(e["inputs"][0]["listed_payers"], 2);
        assert_eq!(e["inputs"][1]["listed_paid_value"], "0");
        assert_eq!(e["inputs"][1]["listed_payers"], 0);
    }

    #[test]
    fn pay_from_payers_the_buyer_does_not_list_is_no_record() {
        // WORKER2 (18) washed 30 fUSD through two accounts of its own. Read
        // whole, that is a record and it beats WORKER's 25. Read through
        // the buyer's list the factory shows none of it: a newcomer pays
        // the premium (18 x 2 = 36) and WORKER wins.
        let run = |payers: Option<Vec<AccountId>>, results: Vec<PromiseResult>| {
            let (mut j, s) = two_bids();
            start_listed(&mut j, s, payers.clone(), None);
            finish_listed(&mut j, s, payers, None, results);
            j.get_job().children[0].worker.clone()
        };
        assert_eq!(run(None, vec![rec(30, 0), rec(30, 0)]), acc(WORKER2));
        assert_eq!(
            run(list(&[LEAD]), vec![rec_from(30, 0, 2), rec_from(0, 0, 0)]),
            acc(WORKER)
        );
    }

    #[test]
    fn a_listed_manager_pay_counts_like_any_record() {
        // The same bidder, paid by a listed manager, keeps its record: the
        // option closes the washing door, not the front one.
        let (mut j, s) = two_bids();
        start_listed(&mut j, s, list(&[LEAD]), None);
        finish_listed(
            &mut j,
            s,
            list(&[LEAD]),
            None,
            vec![rec_from(30, 0, 2), rec_from(30, 0, 1)],
        );
        assert_eq!(j.get_job().children[0].worker, acc(WORKER2));
    }

    #[test]
    fn unpaid_money_counts_whoever_is_listed() {
        let (mut j, s) = two_bids();
        start_listed(&mut j, s, list(&[LEAD]), None);
        // WORKER2's whole record is unpaid: 18 x 2.5 = 45 loses to 25.
        finish_listed(
            &mut j,
            s,
            list(&[LEAD]),
            None,
            vec![rec_from(30, 0, 2), rec_from(0, 20, 0)],
        );
        assert_eq!(j.get_job().children[0].worker, acc(WORKER));
        assert_eq!(
            events("award_best")[0]["inputs"][1]["score"],
            (45 * USD).to_string()
        );
    }

    #[test]
    fn min_paid_applies_to_the_listed_money() {
        // 5 fUSD from a listed manager is under a 10 fUSD floor: a newcomer.
        let (mut j, s) = two_bids();
        let floor = Some(U128(10 * USD));
        start_listed(&mut j, s, list(&[LEAD]), floor);
        finish_listed(
            &mut j,
            s,
            list(&[LEAD]),
            floor,
            vec![rec_from(30, 0, 2), rec_from(5, 0, 1)],
        );
        assert_eq!(j.get_job().children[0].worker, acc(WORKER));
    }

    #[test]
    fn an_empty_payer_list_counts_nobodys_pay() {
        let (mut j, s) = two_bids();
        start_listed(&mut j, s, list(&[]), None);
        assert_eq!(call_args("get_record_from")["payers"], json!([]));
        // Both are newcomers, so the lowest bid wins.
        finish_listed(
            &mut j,
            s,
            list(&[]),
            None,
            vec![rec_from(0, 0, 0), rec_from(0, 0, 0)],
        );
        assert_eq!(j.get_job().children[0].worker, acc(WORKER2));
    }

    #[test]
    #[should_panic(expected = "list at most 16 payers")]
    fn a_payer_list_over_sixteen_is_refused() {
        let (mut j, s) = two_bids();
        let many: Vec<AccountId> = (0..17).map(|i| acc(&format!("p{i}.test"))).collect();
        Ctx::new(ROOT, LEAD).at(CLOSED).deposit(DEPOSIT).set();
        j.award_best(s, 0, 0, None, None, None, Some(many));
    }

    #[test]
    fn a_failed_listed_read_awards_nobody_and_refunds() {
        let (mut j, s) = two_bids();
        start_listed(&mut j, s, list(&[LEAD]), None);
        // An old registry without `get_record_from` fails the read.
        finish_listed(
            &mut j,
            s,
            list(&[LEAD]),
            None,
            vec![rec_from(30, 0, 2), PromiseResult::Failed],
        );
        assert_eq!(j.get_bids(s).unwrap().awarded, None);
        assert!(refunded());
        assert_eq!(events("award_best_failed").len(), 1);
    }
}

/// NEAR Intents payout: the worker can have its pay land as its balance in
/// the NEAR Intents Verifier (`ft_transfer_call` with the worker as
/// receiver), where it can swap it or withdraw any token.
mod intents_payout {
    use super::*;

    const VERIFIER: &str = "intents.test";

    fn with_route() -> Job {
        let mut j = delivered_sub(30 * USD, 35 * USD);
        Ctx::new(T1, WORKER).set();
        j.set_payout_via(Some(acc(VERIFIER)));
        assert_eq!(events("payout_via").len(), 1);
        j
    }

    #[test]
    fn the_payee_can_route_its_pay_to_the_intents_verifier() {
        let mut j = with_route();
        assert_eq!(j.get_job().payout_via, Some(acc(VERIFIER)));
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        // No plain transfer: the pay goes into the Verifier for the worker.
        assert!(transfers().is_empty());
        let reg = call_args("storage_deposit");
        assert_eq!(reg["account_id"], VERIFIER);
        let c = call_args("ft_transfer_call");
        assert_eq!(c["receiver_id"], VERIFIER);
        assert_eq!(c["amount"], (30 * USD).to_string());
        assert_eq!(c["msg"], WORKER);
    }

    #[test]
    #[should_panic(expected = "only the payee picks where its pay goes")]
    fn only_the_payee_picks_the_route() {
        let mut j = delivered_sub(30 * USD, 35 * USD);
        Ctx::new(T1, EVE).set();
        j.set_payout_via(Some(acc(VERIFIER)));
    }

    #[test]
    #[should_panic(expected = "too late: settle has started")]
    fn the_route_is_fixed_once_settle_starts() {
        let mut j = with_route();
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        Ctx::new(T1, WORKER).at(AFTER).set();
        j.set_payout_via(None);
    }

    #[test]
    fn paid_only_when_the_verifier_kept_the_whole_amount() {
        let mut j = with_route();
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        let a = attempt(&j);
        Ctx::new(T1, T1)
            .at(AFTER)
            .results(vec![ok(&u128_json(30 * USD))])
            .set();
        j.on_payee_paid(a);
        assert!(j.get_job().payee_paid);
    }

    #[test]
    fn a_refused_deposit_falls_back_to_a_plain_transfer_on_resettle() {
        let mut j = with_route();
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        // The Verifier refunded everything: used = 0.
        let a = attempt(&j);
        Ctx::new(T1, T1)
            .at(AFTER)
            .results(vec![ok(&u128_json(0))])
            .set();
        j.on_payee_paid(a);
        let v = j.get_job();
        assert!(!v.payee_paid);
        assert_eq!(v.payout_via, None, "a failed route is dropped");
        assert_eq!(events("payout_via_dropped").len(), 1);
    }

    #[test]
    fn a_failed_call_is_not_paid() {
        let mut j = with_route();
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        let a = attempt(&j);
        Ctx::new(T1, T1)
            .at(AFTER)
            .results(vec![PromiseResult::Failed])
            .set();
        j.on_payee_paid(a);
        assert!(!j.get_job().payee_paid);
        assert_eq!(j.get_job().payout_via, None);
    }

    #[test]
    fn the_payee_can_clear_the_route() {
        let mut j = with_route();
        Ctx::new(T1, WORKER).set();
        j.set_payout_via(None);
        Ctx::new(T1, EVE).at(AFTER).set();
        j.settle();
        assert_eq!(transfers(), vec![(WORKER.into(), (30 * USD).to_string())]);
    }
}

// ================================================================ settle gas funnel

mod settle_gas {
    use super::*;

    /// What a receipt's own wasm may burn on top of what the mock charges
    /// (host calls and receipt fees, no wasm). The whole settle and delete
    /// of a sub-job burns 24.4 Tgas on the local node, fees included.
    const WASM: Gas = Gas::from_tgas(5);

    /// Gas `name` left after running with `prepaid`, less its wasm. The
    /// runtime hands this to the weighted calls it scheduled (`Self::ext`
    /// callbacks; plain `function_call`s such as the transfers and the
    /// record have weight 0 and get none).
    fn spare(name: &str, prepaid: Gas) -> Gas {
        let used = env::used_gas().as_gas();
        let left = prepaid.as_gas().saturating_sub(used);
        assert!(
            left >= WASM.as_gas(),
            "{name}: {} of {prepaid} left, less than {WASM} for its own wasm",
            Gas::from_gas(left)
        );
        Gas::from_gas(left - WASM.as_gas())
    }

    fn plus(a: Gas, b: Gas) -> Gas {
        Gas::from_gas(a.as_gas() + b.as_gas())
    }

    /// Weight of every function call scheduled in this receipt, by method.
    fn weights() -> Vec<(String, u64)> {
        get_created_receipts()
            .into_iter()
            .flat_map(|r| r.actions)
            .filter_map(|a| match a {
                MockAction::FunctionCallWeight {
                    method_name,
                    gas_weight,
                    ..
                } => Some((String::from_utf8(method_name).unwrap(), gas_weight.0)),
                _ => None,
            })
            .collect()
    }

    /// The share of `spare` each weighted call gets.
    fn share(spare: Gas) -> Gas {
        let total: u64 = weights().iter().map(|w| w.1).sum();
        assert!(total > 0, "no weighted call to hand the spare gas to");
        Gas::from_gas(spare.as_gas() / total)
    }

    /// Run a settle callback with its static gas plus `extra`.
    fn leg(j: &mut Job, cur: &str, now: u64, name: &str, extra: Gas, r: PromiseResult) -> Gas {
        let stat = match name {
            "on_payee_paid" => GAS_ON_PAYEE_PAID,
            "on_rest_paid" | "on_swept" => GAS_ON_REST_PAID,
            "on_balance_checked" => GAS_ON_BALANCE_CHECKED,
            "on_resume_balance" => GAS_ON_RESUME,
            _ => unreachable!(),
        };
        let prepaid = plus(stat, extra);
        let a = attempt(j);
        Ctx::new(cur, cur)
            .at(now)
            .gas(prepaid)
            .results(vec![r])
            .set();
        match name {
            "on_payee_paid" => drop(j.on_payee_paid(a)),
            "on_rest_paid" => drop(j.on_rest_paid(a)),
            "on_swept" => drop(j.on_swept(a)),
            "on_balance_checked" => drop(j.on_balance_checked(a)),
            _ => drop(j.on_resume_balance(a)),
        }
        share(spare(name, prepaid))
    }

    /// A sub-job's last step: the parent's answer reaches `on_parent_ack`
    /// with its static gas plus `extra`, and it deletes the account.
    fn acked(j: &mut Job, extra: Gas) {
        let prepaid = plus(GAS_ON_PARENT_ACK, extra);
        let a = j.get_job().close_pending.unwrap().attempt;
        Ctx::new(T1, T1)
            .at(STUCK)
            .gas(prepaid)
            .results(vec![ok(b"")])
            .set();
        drop(j.on_parent_ack(a));
        spare("on_parent_ack", prepaid);
        assert_eq!(delete_to(), Some(acc(ROOT)));
    }

    /// A paid sub-job with a worker and a rest, from settle at exactly
    /// `GAS_SETTLE_MIN` to the parent notice and the delete. `via` routes
    /// the pay with `ft_transfer_call`; `stray` tokens make the balance leg
    /// sweep once.
    fn sub_funnel(via: bool, stray: u128) {
        let mut j = delivered_sub(30 * USD, 35 * USD);
        if via {
            Ctx::new(T1, WORKER).set();
            j.set_payout_via(Some(acc(EVE)));
        }
        Ctx::new(T1, EVE).at(AFTER).gas(GAS_SETTLE_MIN).set();
        drop(j.settle());
        let mut x = share(spare("settle", GAS_SETTLE_MIN));
        let paid = if via {
            ok(&u128_json(30 * USD))
        } else {
            ok(b"")
        };
        x = leg(&mut j, T1, AFTER, "on_payee_paid", x, paid);
        x = leg(&mut j, T1, AFTER, "on_rest_paid", x, ok(b""));
        if stray > 0 {
            x = leg(
                &mut j,
                T1,
                AFTER,
                "on_balance_checked",
                x,
                ok(&u128_json(stray)),
            );
            x = leg(&mut j, T1, AFTER, "on_swept", x, ok(b""));
        }
        x = leg(
            &mut j,
            T1,
            AFTER,
            "on_balance_checked",
            x,
            ok(&u128_json(0)),
        );
        assert_eq!(j.get_job().status, "Settled");
        assert!(calls().contains(&(ROOT.into(), "child_closed".into())));
        acked(&mut j, x);
    }

    #[test]
    fn settle_at_minimum_gas_budget_reserves_enough_for_every_leg() {
        sub_funnel(false, 0);
    }

    #[test]
    fn settle_at_minimum_gas_routed_pay_reaches_the_delete() {
        sub_funnel(true, 0);
    }

    #[test]
    fn settle_at_minimum_gas_sweeps_stray_tokens_and_reaches_the_delete() {
        sub_funnel(false, 7);
    }

    #[test]
    fn settle_at_minimum_gas_root_reaches_the_delete() {
        let mut j = funded_root(10 * USD);
        Ctx::new(ROOT, CLIENT).set();
        j.approve();
        Ctx::new(ROOT, EVE).at(AFTER).gas(GAS_SETTLE_MIN).set();
        drop(j.settle());
        let mut x = share(spare("settle", GAS_SETTLE_MIN));
        x = leg(&mut j, ROOT, AFTER, "on_rest_paid", x, ok(b""));
        Ctx::new(ROOT, ROOT)
            .at(AFTER)
            .gas(plus(GAS_ON_BALANCE_CHECKED, x))
            .results(vec![ok(&u128_json(0))])
            .set();
        drop(j.on_balance_checked(attempt(&j)));
        spare("on_balance_checked", plus(GAS_ON_BALANCE_CHECKED, x));
        assert_eq!(j.get_job().status, "Settled");
    }

    /// `resume_settle` at exactly `GAS_SETTLE_MIN` from a stuck payee leg:
    /// the rest leg, the balance leg, the notice and the delete still fit.
    #[test]
    fn resume_settle_at_minimum_gas_reaches_the_delete() {
        let mut j = settling_sub();
        Ctx::new(T1, LEAD).at(STUCK).gas(GAS_SETTLE_MIN).set();
        drop(j.resume_settle());
        let mut x = share(spare("resume_settle", GAS_SETTLE_MIN));
        // The payee was paid: 5 left of 35.
        x = leg(
            &mut j,
            T1,
            STUCK,
            "on_resume_balance",
            x,
            ok(&u128_json(5 * USD)),
        );
        x = leg(&mut j, T1, STUCK, "on_rest_paid", x, ok(b""));
        x = leg(
            &mut j,
            T1,
            STUCK,
            "on_balance_checked",
            x,
            ok(&u128_json(0)),
        );
        acked(&mut j, x);
    }

    /// `resume_close` at exactly its minimum: the notice and the delete fit.
    #[test]
    fn resume_close_at_minimum_gas_reaches_the_delete() {
        let mut j = paid_sub();
        balance_cb(&mut j, T1, ok(&u128_json(0)));
        ack(&mut j, false);
        Ctx::new(T1, EVE).at(AFTER).gas(GAS_RESUME_CLOSE_MIN).set();
        drop(j.resume_close());
        let x = share(spare("resume_close", GAS_RESUME_CLOSE_MIN));
        acked(&mut j, x);
    }
}

// ================================================================ parent notice

mod close_notice {
    use super::stuck_hire::resume_hire_seen;
    use super::*;

    const WASM: Gas = Gas::from_tgas(5);

    /// (gas, weight) of each scheduled call to `method`.
    fn gas_of(method: &str) -> Vec<(Gas, u64)> {
        get_created_receipts()
            .into_iter()
            .flat_map(|r| r.actions)
            .filter_map(|a| match a {
                MockAction::FunctionCallWeight {
                    method_name,
                    prepaid_gas,
                    gas_weight,
                    ..
                } if method_name == method.as_bytes() => Some((prepaid_gas, gas_weight.0)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_notice_reports_what_the_child_was_funded_with() {
        let mut j = paid_sub();
        balance_cb(&mut j, T1, ok(&u128_json(0)));
        assert_eq!(call_args("child_closed")["funded"], (35 * USD).to_string());
    }

    /// A gas failure of `child_closed` must not repeat forever: the resent
    /// notice gets the spare gas of whoever calls `resume_close`, not a
    /// fixed 10 Tgas.
    #[test]
    fn a_resent_notice_gives_child_closed_the_callers_spare_gas() {
        let mut j = paid_sub();
        balance_cb(&mut j, T1, ok(&u128_json(0)));
        ack(&mut j, false);
        let prepaid = Gas::from_tgas(300);
        Ctx::new(T1, EVE).at(AFTER).gas(prepaid).set();
        drop(j.resume_close());
        let cc = gas_of("child_closed");
        assert_eq!(cc.len(), 1);
        let (stat, w) = cc[0];
        assert!(w > 0, "child_closed must take a share of the spare gas");
        let total_w: u64 = gas_of("on_parent_ack").iter().map(|x| x.1).sum::<u64>() + w;
        let spare = prepaid.as_gas() - env::used_gas().as_gas();
        let got = stat.as_gas() + spare / total_w * w;
        assert!(
            got >= Gas::from_tgas(100).as_gas(),
            "child_closed gets only {}",
            Gas::from_gas(got)
        );
    }

    /// `child_closed` on the fullest parent there can be (16 children, 16
    /// bid slots of 16 backed, sealed bids), state read and written as on
    /// chain, with a bond and a backer to pay: fits its static gas.
    #[test]
    fn child_closed_fits_its_gas_on_a_full_parent() {
        let mut j = funded_root(1_000 * USD);
        let idx = hired(&mut j, WORKER, 30 * USD) as usize;
        j.children[idx].bond = U128(NearToken::from_near(1).as_yoctonear());
        j.children[idx].backer = Some(acc("backer-with-a-long-name-0000000.test"));
        j.children[idx].backed = U128(NearToken::from_near(1).as_yoctonear());
        j.children[idx].premium = U128(NearToken::from_millinear(100).as_yoctonear());
        while j.children.len() < MAX_CHILDREN {
            let mut c = j.children[idx].clone();
            c.account = acc(&format!(
                "c{:02}-with-a-long-name.job-1.hiretree.test",
                j.children.len()
            ));
            c.state = ChildState::Closed;
            j.children.push(c);
        }
        let long = |p: &str, n: usize| acc(&format!("{p}-{n:02}-with-a-long-account-name.test"));
        for s in 0..MAX_BID_SLOTS {
            j.bid_slots.push(BidSlot {
                spec_hash: spec(),
                max: U128(30 * USD),
                closes_ns: U64(AFTER),
                bids: (0..MAX_BIDS)
                    .map(|b| Bid {
                        worker: long("bidder", b),
                        price: U128(20 * USD),
                        bond: U128(NearToken::from_near(1).as_yoctonear()),
                        commit: Some("ef".repeat(32)),
                        offer: U128(1),
                        backer: Some(long("backer", b)),
                        backed: U128(NearToken::from_near(1).as_yoctonear()),
                    })
                    .collect(),
                awarded: Some(long("bidder", s)),
                bond: U128(NearToken::from_near(1).as_yoctonear()),
                reveal_ns: U64(AFTER),
            });
        }
        Ctx::new(ROOT, ROOT).set();
        env::state_write(&j);
        Ctx::new(ROOT, T1).gas(GAS_CHILD_CLOSED).set();
        let mut p: Job = env::state_read().expect("state");
        p.child_closed(U128(0), Some(true), Some(U128(30 * USD)));
        env::state_write(&p);
        let used = env::used_gas().as_gas();
        assert!(
            used + WASM.as_gas() <= GAS_CHILD_CLOSED.as_gas(),
            "child_closed used {} of {GAS_CHILD_CLOSED}",
            Gas::from_gas(used)
        );
        assert_eq!(p.children[idx].state, ChildState::Closed);
    }

    /// Hire t1 (30): created, funded on the token, but the parent's fund
    /// callback failed (`hire_unclear`), so it still lists t1 as Funding.
    fn hire_unclear() -> (Job, u32) {
        let mut j = funded_root(100 * USD);
        let idx = hire_one(&mut j, WORKER, 30 * USD);
        Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
        j.on_child_created(idx);
        Ctx::new(ROOT, ROOT)
            .results(vec![PromiseResult::Failed])
            .set();
        assert!(!j.on_child_funded(idx));
        assert_eq!(
            j.get_job().children[idx as usize].state,
            ChildState::Funding
        );
        (j, idx)
    }

    fn books_after_t1_paid_its_worker(j: &Job) {
        let v = j.get_job();
        assert_eq!(v.children[0].state, ChildState::Closed);
        assert_eq!(v.children_pending, 0);
        assert_eq!(v.children_open, 0);
        // 30 went to t1, all of it to its worker: the parent holds 70.
        assert_eq!(v.ft_balance.0, 70 * USD);
        assert_eq!(v.allocated.0, 30 * USD);
    }

    /// Review: the fund callback was lost, t1 worked and settled before
    /// anyone resumed the hire. Its notice says it was funded: the parent
    /// closes it, and a later `resume_hire` has nothing left to do.
    #[test]
    fn a_funded_child_whose_fund_callback_was_lost_closes_on_its_notice() {
        let (mut j, idx) = hire_unclear();
        Ctx::new(ROOT, T1).set();
        j.child_closed(U128(0), Some(true), Some(U128(30 * USD)));
        books_after_t1_paid_its_worker(&j);
        assert_eq!(events("child_closed").len(), 1);
        // The parent can settle now.
        let at = T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC;
        Ctx::new(ROOT, EVE).at(at).set();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| j.resume_hire(idx)));
        assert!(r.is_err(), "resume_hire must refuse a closed child");
    }

    /// The notice arrives while `resume_hire` is reading: the child closes,
    /// and the late resume answer cannot act.
    #[test]
    #[should_panic(expected = "hire is not being resumed")]
    fn a_notice_during_resume_hire_closes_the_child_and_the_late_answer_is_refused() {
        let (mut j, idx) = hire_unclear();
        let at = T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC;
        Ctx::new(ROOT, EVE).at(at).set();
        j.resume_hire(idx);
        Ctx::new(ROOT, T1).at(at).set();
        j.child_closed(U128(0), Some(true), Some(U128(30 * USD)));
        books_after_t1_paid_its_worker(&j);
        Ctx::new(ROOT, ROOT)
            .at(at)
            .results(vec![ok(&u128_json(0)), ok(&u128_json(30 * USD))])
            .set();
        j.on_resume_hire(idx);
    }

    /// Review: t1 already paid out (balance 0) when `resume_hire` reads it.
    /// Its own books say it was funded: it is opened, not rolled back, and
    /// its notice is then credited.
    #[test]
    fn resume_hire_opens_a_funded_child_that_already_paid_out() {
        let (mut j, idx) = hire_unclear();
        resume_hire_seen(&mut j, idx, ok(&u128_json(0)), ok(&u128_json(30 * USD)));
        let v = j.get_job();
        assert_eq!(v.children[0].state, ChildState::Open);
        assert_eq!(v.children_pending, 0);
        assert_eq!(v.children_open, 1);
        assert!(!calls().iter().any(|c| c.1 == "close_unfunded"));
        Ctx::new(ROOT, T1).set();
        j.child_closed(U128(0), Some(true), None);
        books_after_t1_paid_its_worker(&j);
    }

    #[test]
    fn resume_hire_still_rolls_back_a_child_that_was_never_funded() {
        let (mut j, idx) = hire_unclear();
        resume_hire_seen(&mut j, idx, ok(&u128_json(0)), ok(&u128_json(0)));
        let v = j.get_job();
        assert_eq!(v.children[0].state, ChildState::Failed);
        assert_eq!(v.ft_balance.0, 100 * USD);
        assert!(calls().contains(&(T1.into(), "close_unfunded".into())));
    }

    /// An unfunded child (its window lapsed, anyone settled it) closing
    /// while the parent still lists the hire in flight: the hire is undone
    /// and the child may delete itself.
    #[test]
    fn an_unfunded_child_closing_while_its_hire_is_in_flight_rolls_it_back() {
        let (mut j, _) = hire_unclear();
        Ctx::new(ROOT, T1).set();
        j.child_closed(U128(0), Some(false), Some(U128(0)));
        let v = j.get_job();
        assert_eq!(v.children[0].state, ChildState::Failed);
        assert_eq!(v.children_pending, 0);
        assert_eq!(v.ft_balance.0, 100 * USD);
        assert_eq!(v.allocated.0, 0);
    }

    /// A child that does not say what it was funded with (older code) is
    /// still refused while its hire is in flight.
    #[test]
    #[should_panic(expected = "caller is not an open child")]
    fn a_notice_without_funding_from_a_hire_in_flight_is_refused() {
        let (mut j, _) = hire_unclear();
        Ctx::new(ROOT, T1).set();
        j.child_closed(U128(0), Some(true), None);
    }

    #[test]
    #[should_panic(expected = "not enough gas for resume_hire")]
    fn resume_hire_needs_gas() {
        let (mut j, idx) = hire_unclear();
        let at = T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC;
        Ctx::new(ROOT, EVE).at(at).gas(Gas::from_tgas(20)).set();
        j.resume_hire(idx);
    }

    /// A resume whose answer never ran (out of gas) can run again later.
    #[test]
    fn a_stuck_resume_hire_can_run_again() {
        let (mut j, idx) = hire_unclear();
        let at = T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC;
        Ctx::new(ROOT, EVE).at(at).set();
        j.resume_hire(idx);
        let again = at + (RESUME_AFTER_SECS + 1) * NS_PER_SEC;
        Ctx::new(ROOT, EVE).at(again).set();
        j.resume_hire(idx);
        Ctx::new(ROOT, ROOT)
            .at(again)
            .results(vec![ok(&u128_json(30 * USD)), ok(&u128_json(30 * USD))])
            .set();
        assert!(j.on_resume_hire(idx));
        assert_eq!(j.get_job().children_open, 1);
    }

    /// `on_resume_hire` rolls back at its static gas alone (a resume at
    /// the minimum): the close request and both bond transfers fit.
    #[test]
    fn resume_hire_rollback_fits_its_static_gas() {
        let (mut j, idx) = hire_unclear();
        j.children[idx as usize].bond = U128(NearToken::from_near(1).as_yoctonear());
        j.children[idx as usize].backer = Some(acc(EVE));
        j.children[idx as usize].backed = U128(NearToken::from_near(1).as_yoctonear());
        let at = T0 + (RESUME_AFTER_SECS + 1) * NS_PER_SEC;
        Ctx::new(ROOT, EVE).at(at).gas(GAS_RESUME_HIRE_MIN).set();
        drop(j.resume_hire(idx));
        let used = env::used_gas();
        assert!(
            used.as_gas() + WASM.as_gas() <= GAS_RESUME_HIRE_MIN.as_gas(),
            "resume_hire used {used}"
        );
        Ctx::new(ROOT, ROOT)
            .at(at)
            .gas(GAS_ON_RESUME_HIRE)
            .results(vec![ok(&u128_json(0)), ok(&u128_json(0))])
            .set();
        j.on_resume_hire(idx);
        let used = env::used_gas();
        assert!(
            used.as_gas() + WASM.as_gas() <= GAS_ON_RESUME_HIRE.as_gas(),
            "on_resume_hire used {used}"
        );
        assert_eq!(j.get_job().children[0].state, ChildState::Failed);
    }
}

mod close_notice_repeat {
    use super::*;

    /// The unfunded child never saw the parent's answer and sends the same
    /// notice again: it succeeds (so the child can delete) and moves nothing.
    #[test]
    fn a_repeated_notice_from_an_unfunded_child_is_a_no_op() {
        let mut j = funded_root(100 * USD);
        let idx = hire_one(&mut j, WORKER, 30 * USD);
        Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
        j.on_child_created(idx);
        Ctx::new(ROOT, T1).set();
        j.child_closed(U128(0), Some(false), Some(U128(0)));
        let before = j.get_job();
        assert_eq!(before.children[0].state, ChildState::Failed);
        Ctx::new(ROOT, T1).set();
        j.child_closed(U128(0), Some(false), Some(U128(0)));
        let after = j.get_job();
        assert_eq!(after.ft_balance, before.ft_balance);
        assert_eq!(after.allocated, before.allocated);
        assert_eq!(after.children_pending, 0);
        assert_eq!(events("child_closed_again").len(), 1);
        assert!(get_created_receipts().is_empty(), "no bond paid twice");
    }

    /// A child the parent marked Failed that says it was funded is still
    /// refused: the parent cannot tell what it would be crediting.
    #[test]
    #[should_panic(expected = "caller is not an open child")]
    fn a_funded_notice_from_a_failed_child_is_refused() {
        let mut j = funded_root(100 * USD);
        let idx = hire_one(&mut j, WORKER, 30 * USD);
        Ctx::new(ROOT, ROOT).results(vec![ok(b"")]).set();
        j.on_child_created(idx);
        Ctx::new(ROOT, T1).set();
        j.child_closed(U128(0), Some(false), Some(U128(0)));
        Ctx::new(ROOT, T1).set();
        j.child_closed(U128(0), Some(true), Some(U128(30 * USD)));
    }
}
