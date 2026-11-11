#![allow(unused_must_use)]
use super::*;
use near_sdk::mock::MockAction;
use near_sdk::test_utils::{get_created_receipts, VMContextBuilder};
use near_sdk::{testing_env, PromiseResult};

const POOL: &str = "pool-1.op.test";
const TOKEN: &str = "usdc.test";
const FACTORY: &str = "hub.op.test";
const USD: u128 = 1_000_000;
const SEC: u64 = 1_000_000_000;

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}

fn ctx_full(pred: &str, now_s: u64, deposit: NearToken, gas: u64, results: Vec<PromiseResult>) {
    testing_env!(
        VMContextBuilder::new()
            .current_account_id(acc(POOL))
            .predecessor_account_id(acc(pred))
            .block_timestamp(now_s * SEC)
            .attached_deposit(deposit)
            .prepaid_gas(Gas::from_tgas(gas))
            .account_balance(NearToken::from_near(5))
            .build(),
        near_sdk::test_vm_config(),
        near_sdk::RuntimeFeesConfig::test(),
        Default::default(),
        results
    );
}

fn ctx(pred: &str, now_s: u64) {
    ctx_full(pred, now_s, NearToken::from_yoctonear(0), 300, vec![]);
}

fn cb(results: Vec<PromiseResult>) {
    ctx_full(POOL, 200, NearToken::from_yoctonear(0), 300, results);
}

fn ok(v: serde_json::Value) -> PromiseResult {
    PromiseResult::Successful(v.to_string().into_bytes())
}

fn pool() -> Pool {
    ctx("organiser.test", 100);
    Pool::new(
        acc(TOKEN),
        acc(FACTORY),
        acc("lead.test"),
        "ab".repeat(32),
        3600,
        U128(10 * USD),
        Some(acc("treasury.test")),
        U128(90 * USD),
        600,
    )
}

fn join(p: &mut Pool, who: &str, usd: u128, now_s: u64) -> u128 {
    ctx(TOKEN, now_s);
    match p.ft_on_transfer(acc(who), U128(usd * USD), "join".into()) {
        PromiseOrValue::Value(v) => v.0,
        PromiseOrValue::Promise(_) => panic!("expected a value"),
    }
}

fn full() -> Pool {
    let mut p = pool();
    join(&mut p, "a.test", 30, 110);
    join(&mut p, "b.test", 30, 110);
    join(&mut p, "c.test", 30, 110);
    p
}

/// The attempt of the step in flight (launch, finish or claim).
fn att(p: &Pool) -> u32 {
    p.pending.as_ref().expect("a step in flight").attempt
}

/// `launch` without its promises: the pool is Launching at `now_s`.
fn launching(now_s: u64) -> Pool {
    let mut p = full();
    ctx_full("anyone.test", now_s, POST_NEAR, 300, vec![]);
    p.launch();
    p
}

fn launched(p: &mut Pool, r: PromiseResult) -> State {
    let a = att(p);
    cb(vec![r]);
    p.on_launched(a)
}

fn job_checked(p: &mut Pool, r: PromiseResult) -> bool {
    let a = att(p);
    cb(vec![r]);
    p.on_job_checked(a).is_some()
}

fn balance(p: &mut Pool, r: PromiseResult) -> U128 {
    let a = att(p);
    cb(vec![r]);
    p.on_balance(a)
}

fn claimed(p: &mut Pool, r: PromiseResult) -> bool {
    let a = att(p);
    cb(vec![r]);
    p.on_claimed(a)
}

fn posted() -> Pool {
    let mut p = launching(120);
    assert_eq!(launched(&mut p, ok(json!(U128(90 * USD)))), State::Posted);
    p
}

fn bound() -> Pool {
    let mut p = posted();
    cb(vec![ok(
        json!({ "job": "job-3.hub.op.test", "client": POOL,
        "manager": "lead.test", "budget": U128(90 * USD), "state": "Posted" }),
    )]);
    p.on_bound(3);
    p
}

fn done(payout: u128) -> Pool {
    let mut p = bound();
    ctx("anyone.test", 200);
    p.finish();
    job_checked(&mut p, PromiseResult::Failed);
    balance(&mut p, ok(json!(U128(payout))));
    p
}

fn calls() -> Vec<(String, String, Vec<u8>, NearToken)> {
    get_created_receipts()
        .into_iter()
        .flat_map(|r| {
            let to = r.receiver_id.to_string();
            r.actions.into_iter().filter_map(move |a| match a {
                MockAction::FunctionCallWeight {
                    method_name,
                    args,
                    attached_deposit,
                    ..
                } => Some((
                    to.clone(),
                    String::from_utf8(method_name).unwrap(),
                    args,
                    attached_deposit,
                )),
                _ => None,
            })
        })
        .collect()
}

#[test]
#[should_panic(expected = "spec_hash must be 64 hex chars")]
fn new_checks_the_spec_hash() {
    ctx("organiser.test", 0);
    Pool::new(
        acc(TOKEN),
        acc(FACTORY),
        acc("lead.test"),
        "zz".into(),
        3600,
        U128(0),
        None,
        U128(90 * USD),
        600,
    );
}

#[test]
#[should_panic(expected = "lead_fee is larger than the target")]
fn new_checks_the_fee() {
    ctx("organiser.test", 0);
    Pool::new(
        acc(TOKEN),
        acc(FACTORY),
        acc("lead.test"),
        "ab".repeat(32),
        3600,
        U128(91 * USD),
        None,
        U128(90 * USD),
        600,
    );
}

#[test]
fn joins_add_up_and_the_excess_is_refunded() {
    let mut p = pool();
    assert_eq!(join(&mut p, "a.test", 40, 110), 0);
    assert_eq!(join(&mut p, "a.test", 10, 110), 0);
    // b sends 50 but only 40 is still needed.
    assert_eq!(join(&mut p, "b.test", 50, 110), 10 * USD);
    let v = p.get_pool();
    assert_eq!(v["raised"], json!(U128(90 * USD)));
    assert_eq!(v["members"][0]["stake"], json!(U128(50 * USD)));
    assert_eq!(v["members"][1]["stake"], json!(U128(40 * USD)));
    // Full: everything is refunded.
    assert_eq!(join(&mut p, "c.test", 5, 110), 5 * USD);
}

#[test]
fn joins_are_refused_from_the_wrong_token_with_a_bad_msg_or_late() {
    let mut p = pool();
    ctx("fake.test", 110);
    assert!(
        matches!(p.ft_on_transfer(acc("a.test"), U128(USD), "join".into()),
        PromiseOrValue::Value(U128(v)) if v == USD)
    );
    ctx(TOKEN, 110);
    assert!(
        matches!(p.ft_on_transfer(acc("a.test"), U128(USD), "hi".into()),
        PromiseOrValue::Value(U128(v)) if v == USD)
    );
    // Joining closes at 100 + 600 s.
    assert_eq!(join(&mut p, "a.test", 1, 700), USD);
    assert_eq!(p.get_pool()["raised"], json!(U128(0)));
}

#[test]
fn the_pool_takes_at_most_sixteen_members() {
    let mut p = pool();
    for i in 0..MAX_MEMBERS {
        assert_eq!(join(&mut p, &format!("m{i}.test"), 1, 110), 0);
    }
    assert_eq!(join(&mut p, "late.test", 1, 110), USD);
    // An existing member can still add.
    assert_eq!(join(&mut p, "m0.test", 1, 110), 0);
}

#[test]
#[should_panic(expected = "the target is not reached yet")]
fn launch_needs_the_full_target() {
    let mut p = pool();
    join(&mut p, "a.test", 30, 110);
    ctx_full("a.test", 120, POST_NEAR, 300, vec![]);
    p.launch();
}

#[test]
#[should_panic(expected = "attach 0.06 NEAR")]
fn launch_needs_the_post_deposit() {
    let mut p = full();
    ctx_full("a.test", 120, NearToken::from_millinear(10), 300, vec![]);
    p.launch();
}

#[test]
#[should_panic(expected = "attach at least 240 Tgas")]
fn launch_needs_gas() {
    let mut p = full();
    ctx_full("a.test", 120, POST_NEAR, 200, vec![]);
    p.launch();
}

#[test]
fn launch_prepays_and_posts_the_whole_budget_as_client() {
    let mut p = full();
    ctx_full("anyone.test", 120, POST_NEAR, 300, vec![]);
    p.launch();
    assert_eq!(p.get_pool()["state"], json!("Launching"));
    let c = calls();
    assert_eq!((c[0].0.as_str(), c[0].1.as_str()), (FACTORY, "prepay_post"));
    assert_eq!(c[0].3, POST_NEAR);
    assert_eq!(
        (c[1].0.as_str(), c[1].1.as_str()),
        (TOKEN, "ft_transfer_call")
    );
    let a: serde_json::Value = serde_json::from_slice(&c[1].2).unwrap();
    assert_eq!(a["receiver_id"], json!(FACTORY));
    assert_eq!(a["amount"], json!(U128(90 * USD)));
    let m: serde_json::Value = serde_json::from_str(a["msg"].as_str().unwrap()).unwrap();
    assert_eq!(m["manager"], json!("lead.test"));
    assert_eq!(m["lead_fee"], json!(U128(10 * USD)));
    assert_eq!(c[2].1, "on_launched");
    // No second launch.
    ctx_full("anyone.test", 120, POST_NEAR, 300, vec![]);
    let again = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.launch()));
    assert!(again.is_err());
}

#[test]
fn a_refused_post_is_a_failed_launch_and_members_get_it_back() {
    let mut p = launching(120);
    assert_eq!(launched(&mut p, ok(json!(U128(0)))), State::LaunchFailed);
    ctx("anyone.test", 130);
    p.finish();
    assert_eq!(calls()[0].1, "ft_balance_of");
    balance(&mut p, ok(json!(U128(90 * USD))));
    ctx("b.test", 140);
    p.claim();
    let a: serde_json::Value = serde_json::from_slice(&calls()[0].2).unwrap();
    assert_eq!(a["amount"], json!(U128(30 * USD)));
    assert_eq!(a["receiver_id"], json!("b.test"));
}

#[test]
fn no_answer_from_the_token_is_a_failed_launch() {
    let mut p = launching(120);
    assert_eq!(launched(&mut p, PromiseResult::Failed), State::LaunchFailed);
}

#[test]
fn bind_takes_only_a_job_whose_client_is_this_pool() {
    let mut p = posted();
    cb(vec![ok(
        json!({ "job": "job-3.hub.op.test", "client": "someone.test",
        "manager": "lead.test", "budget": U128(90 * USD), "state": "Posted" }),
    )]);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.on_bound(3))).is_err());
    cb(vec![ok(json!(null))]);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.on_bound(4))).is_err());
    cb(vec![ok(
        json!({ "job": "job-3.hub.op.test", "client": POOL,
        "manager": "lead.test", "budget": U128(90 * USD), "state": "Posted" }),
    )]);
    assert_eq!(p.on_bound(3), acc("job-3.hub.op.test"));
    assert_eq!(p.get_pool()["job_n"], json!(3));
}

#[test]
#[should_panic(expected = "the pool has not posted a job")]
fn bind_needs_a_posted_pool() {
    let mut p = full();
    ctx("anyone.test", 120);
    p.bind(1);
}

#[test]
fn the_pool_approves_once_more_than_half_the_stake_votes() {
    let mut p = bound();
    ctx("a.test", 150);
    assert!(p.vote_approve().is_none());
    // a again: still 30 of 90.
    ctx("a.test", 150);
    assert!(p.vote_approve().is_none());
    ctx("b.test", 150);
    assert!(p.vote_approve().is_some());
    let c = calls();
    assert_eq!(
        (c[0].0.as_str(), c[0].1.as_str()),
        ("job-3.hub.op.test", "approve")
    );
    // Sent once only.
    ctx("c.test", 150);
    assert!(p.vote_approve().is_none());
}

#[test]
fn a_refused_approve_can_be_resent_by_a_later_vote() {
    let mut p = bound();
    ctx("a.test", 150);
    p.vote_approve();
    ctx("b.test", 150);
    assert!(p.vote_approve().is_some());
    cb(vec![PromiseResult::Failed]);
    assert!(!p.on_approved());
    ctx("a.test", 160);
    assert!(p.vote_approve().is_some());
}

#[test]
#[should_panic(expected = "only members can vote")]
fn outsiders_cannot_vote() {
    let mut p = bound();
    ctx("lead.test", 150);
    p.vote_approve();
}

#[test]
fn finish_waits_until_the_job_account_is_gone() {
    let mut p = bound();
    ctx("anyone.test", 200);
    p.finish();
    let c = calls();
    assert_eq!(
        (c[0].0.as_str(), c[0].1.as_str()),
        ("job-3.hub.op.test", "get_job")
    );
    // The job answered: not settled yet.
    assert!(!job_checked(&mut p, ok(json!({ "status": "Open" }))));
    assert_eq!(p.get_pool()["state"], json!("Posted"));
    // Later: the account is gone, so read the balance.
    ctx("anyone.test", 300);
    p.finish();
    assert!(job_checked(&mut p, PromiseResult::Failed));
    assert_eq!(calls()[0].1, "ft_balance_of");
    assert_eq!(balance(&mut p, ok(json!(U128(42 * USD)))), U128(42 * USD));
    assert_eq!(p.get_pool()["state"], json!("Done"));
}

#[test]
#[should_panic(expected = "the pool is still open")]
fn an_open_pool_cannot_finish_before_joining_closes() {
    let mut p = pool();
    join(&mut p, "a.test", 30, 110);
    ctx("anyone.test", 300);
    p.finish();
}

#[test]
fn a_pool_that_missed_its_target_refunds_after_closing() {
    let mut p = pool();
    join(&mut p, "a.test", 30, 110);
    join(&mut p, "b.test", 10, 110);
    ctx("anyone.test", 800);
    p.finish();
    balance(&mut p, ok(json!(U128(40 * USD))));
    ctx("b.test", 810);
    p.claim();
    let a: serde_json::Value = serde_json::from_slice(&calls()[0].2).unwrap();
    assert_eq!(a["amount"], json!(U128(10 * USD)));
}

#[test]
fn an_unreadable_balance_goes_back_to_the_previous_state() {
    let mut p = bound();
    ctx("anyone.test", 200);
    p.finish();
    job_checked(&mut p, PromiseResult::Failed);
    balance(&mut p, PromiseResult::Failed);
    assert_eq!(p.get_pool()["state"], json!("Posted"));
}

#[test]
fn members_claim_pro_rata_once() {
    // 60 of 90 came back: 20 each.
    let mut p = done(60 * USD);
    ctx("c.test", 400);
    p.claim();
    let c = calls();
    assert_eq!((c[0].0.as_str(), c[0].1.as_str()), (TOKEN, "ft_transfer"));
    assert_eq!(c[0].3, NearToken::from_yoctonear(1));
    let a: serde_json::Value = serde_json::from_slice(&c[0].2).unwrap();
    assert_eq!(a["amount"], json!(U128(20 * USD)));
    ctx("c.test", 400);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.claim())).is_err());
}

#[test]
fn a_failed_claim_can_be_retried() {
    let mut p = done(60 * USD);
    ctx("a.test", 400);
    p.claim();
    assert!(!claimed(&mut p, PromiseResult::Failed));
    ctx("a.test", 410);
    p.claim();
    assert_eq!(calls()[0].1, "ft_transfer");
}

#[test]
fn tokens_arriving_after_finish_are_shared_by_a_recount() {
    let mut p = done(60 * USD);
    ctx("a.test", 400);
    p.claim();
    assert!(claimed(&mut p, ok(json!(null))));
    // 30 more arrive late; the pool now holds 40 + 30.
    ctx("anyone.test", 500);
    p.finish();
    assert_eq!(calls()[0].1, "ft_balance_of");
    assert_eq!(balance(&mut p, ok(json!(U128(70 * USD)))), U128(90 * USD));
    // a already had 20 of its 30; b gets the full 30.
    ctx("a.test", 510);
    p.claim();
    let a: serde_json::Value = serde_json::from_slice(&calls()[0].2).unwrap();
    assert_eq!(a["amount"], json!(U128(10 * USD)));
    assert!(claimed(&mut p, ok(json!(null))));
    ctx("b.test", 510);
    p.claim();
    let b: serde_json::Value = serde_json::from_slice(&calls()[0].2).unwrap();
    assert_eq!(b["amount"], json!(U128(30 * USD)));
}

#[test]
#[should_panic(expected = "claims in flight")]
fn a_recount_waits_for_claims_in_flight() {
    let mut p = done(60 * USD);
    ctx("a.test", 400);
    p.claim();
    ctx("anyone.test", 410);
    p.finish();
}

#[test]
#[should_panic(expected = "only members can claim")]
fn outsiders_cannot_claim() {
    let mut p = done(60 * USD);
    ctx("lead.test", 400);
    p.claim();
}

// ================================================================ resume

const STUCK_AFTER: u64 = RESUME_AFTER_SECS;

/// `resume` at `now_s`, then the token's answer to the balance read.
fn resume_with(p: &mut Pool, now_s: u64, real: PromiseResult) -> bool {
    ctx("anyone.test", now_s);
    p.resume();
    assert_eq!(
        (calls()[0].0.as_str(), calls()[0].1.as_str()),
        (TOKEN, "ft_balance_of")
    );
    let a = att(p);
    ctx_full(POOL, now_s, NearToken::from_yoctonear(0), 300, vec![real]);
    p.on_resume(a)
}

#[test]
#[should_panic(expected = "nothing is stuck")]
fn resume_needs_a_step_in_flight() {
    let mut p = bound();
    ctx("anyone.test", 5_000);
    p.resume();
}

#[test]
#[should_panic(expected = "not stuck yet")]
fn resume_waits_until_the_step_is_stuck() {
    let mut p = launching(120);
    ctx("anyone.test", 120 + STUCK_AFTER - 1);
    p.resume();
}

#[test]
#[should_panic(expected = "not enough gas for resume")]
fn resume_needs_gas() {
    let mut p = launching(120);
    ctx_full(
        "anyone.test",
        120 + STUCK_AFTER,
        NearToken::from_yoctonear(0),
        10,
        vec![],
    );
    p.resume();
}

#[test]
fn a_stuck_launch_whose_budget_left_resumes_to_posted() {
    // on_launched never ran; the factory kept the budget (balance 0).
    let mut p = launching(120);
    assert!(resume_with(&mut p, 120 + STUCK_AFTER, ok(json!(U128(0)))));
    assert_eq!(p.get_pool()["state"], json!("Posted"));
    assert!(p.pending.is_none());
}

#[test]
fn a_stuck_launch_whose_budget_came_back_resumes_to_launch_failed() {
    let mut p = launching(120);
    assert!(resume_with(
        &mut p,
        120 + STUCK_AFTER,
        ok(json!(U128(90 * USD)))
    ));
    assert_eq!(p.get_pool()["state"], json!("LaunchFailed"));
    // Members get it back.
    ctx("anyone.test", 2_000);
    p.finish();
    balance(&mut p, ok(json!(U128(90 * USD))));
    ctx("a.test", 2_010);
    p.claim();
    let a: serde_json::Value = serde_json::from_slice(&calls()[0].2).unwrap();
    assert_eq!(a["amount"], json!(U128(30 * USD)));
}

#[test]
fn a_late_launch_answer_after_resume_changes_nothing() {
    let mut p = launching(120);
    let first = att(&p);
    resume_with(&mut p, 120 + STUCK_AFTER, ok(json!(U128(90 * USD))));
    cb(vec![ok(json!(U128(90 * USD)))]);
    let late = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.on_launched(first)));
    assert!(late.is_err(), "stale callback refused");
    assert_eq!(p.get_pool()["state"], json!("LaunchFailed"));
}

#[test]
fn an_unanswered_resume_stays_stuck_and_can_retry() {
    let mut p = launching(120);
    assert!(!resume_with(
        &mut p,
        120 + STUCK_AFTER,
        PromiseResult::Failed
    ));
    assert_eq!(p.get_pool()["state"], json!("Launching"));
    assert!(p.pending.is_some());
    assert!(resume_with(
        &mut p,
        120 + 2 * STUCK_AFTER,
        ok(json!(U128(0)))
    ));
    assert_eq!(p.get_pool()["state"], json!("Posted"));
}

#[test]
fn a_stuck_finish_goes_back_and_can_run_again() {
    // finish's callbacks never ran: no money moved, so resume just puts the
    // pool back where it was.
    let mut p = bound();
    ctx("anyone.test", 200);
    p.finish();
    let first = att(&p);
    ctx("anyone.test", 200 + STUCK_AFTER);
    p.resume();
    assert!(calls().is_empty(), "no read needed: nothing was in flight");
    assert_eq!(p.get_pool()["state"], json!("Posted"));
    assert!(p.pending.is_none());
    // A late answer of the lost round is refused.
    cb(vec![PromiseResult::Failed]);
    let late = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.on_job_checked(first)));
    assert!(late.is_err());
    ctx("anyone.test", 2_000);
    p.finish();
    assert!(job_checked(&mut p, PromiseResult::Failed));
    assert_eq!(balance(&mut p, ok(json!(U128(60 * USD)))), U128(60 * USD));
}

#[test]
fn a_stale_balance_from_a_lost_finish_cannot_inflate_the_payout() {
    // A recount's balance read (60) is lost; after resume a member claims
    // 20; the next recount starts. The old read must not land, or the 20
    // already paid would be counted twice.
    let mut p = done(60 * USD);
    ctx("anyone.test", 300);
    p.finish();
    let old = att(&p);
    ctx("anyone.test", 300 + STUCK_AFTER);
    p.resume();
    assert_eq!(p.get_pool()["state"], json!("Done"));
    ctx("a.test", 1_000);
    p.claim();
    assert!(claimed(&mut p, ok(json!(null))));
    ctx("anyone.test", 1_100);
    p.finish();
    cb(vec![ok(json!(U128(60 * USD)))]);
    let late = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.on_balance(old)));
    assert!(late.is_err(), "stale balance refused");
    assert_eq!(balance(&mut p, ok(json!(U128(40 * USD)))), U128(60 * USD));
}

#[test]
#[should_panic(expected = "a claim is in flight")]
fn one_claim_in_flight_at_a_time() {
    let mut p = done(60 * USD);
    ctx("a.test", 400);
    p.claim();
    ctx("b.test", 400);
    p.claim();
}

#[test]
fn a_stuck_claim_that_was_paid_is_not_paid_again() {
    // a's 20 left (the pool holds 40 = 60 - 20) but on_claimed never ran.
    let mut p = done(60 * USD);
    ctx("a.test", 400);
    p.claim();
    assert!(resume_with(
        &mut p,
        400 + STUCK_AFTER,
        ok(json!(U128(40 * USD)))
    ));
    assert!(p.pending.is_none());
    assert_eq!(p.get_pool()["paid_out"], json!(U128(20 * USD)));
    ctx("a.test", 2_000);
    let again = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.claim()));
    assert!(again.is_err(), "nothing left to claim");
    // Others claim as usual.
    ctx("b.test", 2_000);
    p.claim();
    let b: serde_json::Value = serde_json::from_slice(&calls()[0].2).unwrap();
    assert_eq!(b["amount"], json!(U128(20 * USD)));
}

#[test]
fn a_stuck_claim_that_failed_can_be_claimed_again() {
    // The pool still holds all 60: a's transfer never happened.
    let mut p = done(60 * USD);
    ctx("a.test", 400);
    p.claim();
    assert!(resume_with(
        &mut p,
        400 + STUCK_AFTER,
        ok(json!(U128(60 * USD)))
    ));
    assert_eq!(p.get_pool()["paid_out"], json!(U128(0)));
    ctx("a.test", 2_000);
    p.claim();
    let a: serde_json::Value = serde_json::from_slice(&calls()[0].2).unwrap();
    assert_eq!(a["amount"], json!(U128(20 * USD)));
}

#[test]
fn a_late_claim_answer_after_resume_changes_nothing() {
    let mut p = done(60 * USD);
    ctx("a.test", 400);
    p.claim();
    let first = att(&p);
    resume_with(&mut p, 400 + STUCK_AFTER, ok(json!(U128(40 * USD))));
    cb(vec![PromiseResult::Failed]);
    let late = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.on_claimed(first)));
    assert!(late.is_err(), "stale callback refused");
    assert_eq!(
        p.get_pool()["paid_out"],
        json!(U128(20 * USD)),
        "not reverted"
    );
}

/// A phantom (a refused `ft_transfer_call` the token takes back a block
/// later) cannot inflate the payout: once the job reports its refund, the
/// pool shares exactly that, with no balance read at all.
#[test]
fn a_reported_refund_is_shared_exactly_with_no_read() {
    let mut p = bound();
    ctx("job-3.hub.op.test", 190);
    p.job_refund(U128(30 * USD));
    ctx("anyone.test", 200);
    p.finish();
    assert!(calls().is_empty(), "no get_job, no balance read");
    assert_eq!(p.get_pool()["state"], json!("Done"));
    assert_eq!(p.get_pool()["payout"], json!(U128(30 * USD)));
    ctx("a.test", 400);
    p.claim();
    let a: serde_json::Value = serde_json::from_slice(&calls()[0].2).unwrap();
    assert_eq!(a["amount"], json!(U128(10 * USD)), "a gets its fair 10");
    // A recount with a phantom in the read cannot raise the payout.
    claimed(&mut p, ok(json!(null)));
    ctx("anyone.test", 500);
    p.finish();
    assert_eq!(balance(&mut p, ok(json!(U128(80 * USD)))), U128(30 * USD));
}

#[test]
#[should_panic(expected = "only the bound job reports its refund")]
fn only_the_bound_job_reports_a_refund() {
    let mut p = bound();
    ctx("a.test", 190);
    p.job_refund(U128(90 * USD));
}

#[test]
fn a_read_never_counts_more_than_members_put_in() {
    // No report (the notice was lost): the read is capped at what was raised.
    let mut p = done(500 * USD);
    assert_eq!(p.get_pool()["payout"], json!(U128(90 * USD)));
    ctx("a.test", 400);
    p.claim();
    let a: serde_json::Value = serde_json::from_slice(&calls()[0].2).unwrap();
    assert_eq!(a["amount"], json!(U128(30 * USD)));
}

#[test]
fn a_settled_job_counts_as_gone() {
    // NEAR may refuse to delete a large account; Settled means the refund
    // already left, so finish goes on to read.
    let mut p = bound();
    ctx("anyone.test", 200);
    p.finish();
    assert!(job_checked(&mut p, ok(json!({ "status": "Settled" }))));
    assert_eq!(calls()[0].1, "ft_balance_of");
}
