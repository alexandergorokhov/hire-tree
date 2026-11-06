//! Hire Tree pool (collective purchasing). Several clients club together to
//! buy one job. One pool per account (`pool-<n>.<op>`), so the pool's token
//! balance is always exactly what its members are owed.
//!
//! 1. `new`: the organiser sets the job (manager, spec hash, window, lead
//!    fee), the target budget and how long joining stays open.
//! 2. Members join with `ft_transfer_call(pool, amount, "join")`. The pool
//!    keeps at most what the target still needs and refunds the rest.
//! 3. Target reached: anyone calls `launch` (attaching 0.06 NEAR). The pool
//!    prepays the factory and posts the job as its client, in one
//!    `ft_transfer_call`. If the factory refuses, the budget comes back.
//! 4. `bind(n)`: anyone tells the pool which factory job is its own; the
//!    factory's record must name this pool as the client.
//! 5. Members vote to approve the delivered job early; more than half of
//!    the stake approves it (the pool is the job's client).
//! 6. `finish`: once the job account is gone (it deletes itself after
//!    settle, sending the unspent budget back), or joining closed short of
//!    the target, or the launch failed, the pool reads its own balance.
//!    Each member then `claim`s its pro-rata share.

use near_sdk::json_types::{U128, U64};
use near_sdk::serde::Deserialize;
use near_sdk::serde_json::{self, json};
use near_sdk::{
    env, near, require, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseOrValue,
};

/// What the factory asks per post (`prepay_post`).
pub const POST_NEAR: NearToken = NearToken::from_millinear(60);
/// Smallest target (1 fUSD at 6 decimals, the factory's minimum budget).
pub const MIN_TARGET: u128 = 1_000_000;
pub const MAX_MEMBERS: usize = 16;
pub const MIN_JOIN_SECS: u64 = 10;
pub const MAX_JOIN_SECS: u64 = 30 * 24 * 3600;
const GAS_PREPAY: Gas = Gas::from_tgas(10);
/// The factory needs 110 Tgas left in `ft_on_transfer`; the token's own
/// transfer and resolve take the rest.
const GAS_POST: Gas = Gas::from_tgas(200);
const GAS_ON_LAUNCHED: Gas = Gas::from_tgas(10);
/// Least gas `launch` must be given.
pub const GAS_LAUNCH_MIN: Gas = Gas::from_tgas(240);
const GAS_VIEW: Gas = Gas::from_tgas(5);
const GAS_ON_BOUND: Gas = Gas::from_tgas(10);
const GAS_APPROVE: Gas = Gas::from_tgas(10);
const GAS_ON_APPROVED: Gas = Gas::from_tgas(5);
const GAS_ON_JOB_GONE: Gas = Gas::from_tgas(25);
const GAS_ON_BALANCE: Gas = Gas::from_tgas(10);
const GAS_TRANSFER: Gas = Gas::from_tgas(10);
const GAS_ON_CLAIMED: Gas = Gas::from_tgas(5);

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Taking members.
    Open,
    /// `launch` sent the budget to the factory; waiting for its answer.
    Launching,
    /// The factory kept the budget: the job exists.
    Posted,
    /// The factory refused the post; the budget is back.
    LaunchFailed,
    /// `finish` is reading the job and the balance.
    Finishing,
    /// Members may claim their share of `payout`.
    Done,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub account: AccountId,
    pub stake: U128,
    pub voted: bool,
    pub claimed: bool,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobTerms {
    pub manager: AccountId,
    pub spec_hash: String,
    pub window_secs: u64,
    pub lead_fee: U128,
    pub treasury: Option<AccountId>,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Pool {
    token: AccountId,
    factory: AccountId,
    terms: JobTerms,
    target: u128,
    closes_ns: u64,
    members: Vec<Member>,
    raised: u128,
    state: State,
    /// The factory's job number and account, once bound.
    job_n: Option<u32>,
    job: Option<AccountId>,
    approve_sent: bool,
    /// What members share at `Done`: the pool's whole balance then.
    payout: u128,
    /// State to return to if `finish` cannot complete.
    before_finish: State,
}

fn emit(event: &str, data: serde_json::Value) {
    let line =
        json!({ "standard": "hiretree", "version": "0.1.0", "event": event, "data": [data] });
    env::log_str(&format!("EVENT_JSON:{line}"));
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn result_bytes() -> Option<Vec<u8>> {
    env::promise_result_checked(0, 4096).ok()
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct FactoryPost {
    job: AccountId,
    client: AccountId,
    budget: U128,
    state: String,
}

#[near]
impl Pool {
    #[init]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        token: AccountId,
        factory: AccountId,
        manager: AccountId,
        spec_hash: String,
        window_secs: u64,
        lead_fee: U128,
        treasury: Option<AccountId>,
        target: U128,
        join_secs: u64,
    ) -> Self {
        require!(is_hex64(&spec_hash), "spec_hash must be 64 hex chars");
        require!(target.0 >= MIN_TARGET, "target is below 1 fUSD");
        require!(lead_fee.0 <= target.0, "lead_fee is larger than the target");
        require!(
            (MIN_JOIN_SECS..=MAX_JOIN_SECS).contains(&join_secs),
            "join_secs out of range"
        );
        let closes_ns = env::block_timestamp() + join_secs * 1_000_000_000;
        emit(
            "pool_opened",
            json!({ "target": target, "manager": manager, "closes_ns": U64(closes_ns) }),
        );
        Self {
            token,
            factory,
            terms: JobTerms {
                manager,
                spec_hash,
                window_secs,
                lead_fee,
                treasury,
            },
            target: target.0,
            closes_ns,
            members: vec![],
            raised: 0,
            state: State::Open,
            job_n: None,
            job: None,
            approve_sent: false,
            payout: 0,
            before_finish: State::Open,
        }
    }

    fn refund(&self, amount: U128, reason: &str) -> PromiseOrValue<U128> {
        emit(
            "join_refused",
            json!({ "amount": amount, "reason": reason }),
        );
        PromiseOrValue::Value(amount)
    }

    /// NEP-141 receiver: `msg` "join" adds the sender as a member. Keeps at
    /// most what the target still needs; refunds the rest.
    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        if env::predecessor_account_id() != self.token {
            return self.refund(amount, "wrong token");
        }
        if msg != "join" {
            return self.refund(amount, "msg must be \"join\"");
        }
        if self.state != State::Open {
            return self.refund(amount, "the pool is not open");
        }
        if env::block_timestamp() >= self.closes_ns {
            return self.refund(amount, "joining is closed");
        }
        let need = self.target - self.raised;
        let take = amount.0.min(need);
        if take == 0 {
            return self.refund(amount, "the target is already reached");
        }
        match self.members.iter_mut().find(|m| m.account == sender_id) {
            Some(m) => m.stake = U128(m.stake.0 + take),
            None => {
                if self.members.len() >= MAX_MEMBERS {
                    return self.refund(amount, "the pool is full");
                }
                self.members.push(Member {
                    account: sender_id.clone(),
                    stake: U128(take),
                    voted: false,
                    claimed: false,
                });
            }
        }
        self.raised += take;
        emit(
            "joined",
            json!({ "member": sender_id, "amount": U128(take), "raised": U128(self.raised) }),
        );
        PromiseOrValue::Value(U128(amount.0 - take))
    }

    /// Anyone, once the target is raised: prepay the factory and post the
    /// job with the whole budget. Attach 0.06 NEAR and 240+ Tgas.
    #[payable]
    pub fn launch(&mut self) -> Promise {
        require!(self.state == State::Open, "the pool is not open");
        require!(self.raised == self.target, "the target is not reached yet");
        require!(
            env::attached_deposit() >= POST_NEAR,
            "attach 0.06 NEAR for the factory's post"
        );
        require!(
            env::prepaid_gas() >= GAS_LAUNCH_MIN,
            "attach at least 240 Tgas"
        );
        self.state = State::Launching;
        let msg = json!({
            "manager": self.terms.manager, "spec_hash": self.terms.spec_hash,
            "window_secs": self.terms.window_secs, "treasury": self.terms.treasury,
            "lead_fee": self.terms.lead_fee,
        });
        emit("launch", json!({ "budget": U128(self.target) }));
        Promise::new(self.factory.clone())
            .function_call(
                "prepay_post",
                b"{}".to_vec(),
                env::attached_deposit(),
                GAS_PREPAY,
            )
            .then(
                Promise::new(self.token.clone()).function_call(
                    "ft_transfer_call",
                    json!({ "receiver_id": self.factory, "amount": U128(self.target),
                            "msg": msg.to_string() })
                    .to_string()
                    .into_bytes(),
                    NearToken::from_yoctonear(1),
                    GAS_POST,
                ),
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_LAUNCHED)
                    .on_launched(),
            )
    }

    /// The token's answer: how much the factory kept. All of it: posted.
    /// Anything else (refused, or no answer): the post failed and the
    /// budget is, or will be, back here; `finish` shares what is held.
    #[private]
    pub fn on_launched(&mut self) -> State {
        require!(self.state == State::Launching, "not launching");
        let used = result_bytes()
            .and_then(|b| serde_json::from_slice::<U128>(&b).ok())
            .map(|u| u.0)
            .unwrap_or(0);
        self.state = if used == self.target {
            State::Posted
        } else {
            State::LaunchFailed
        };
        emit(
            "launched",
            json!({ "state": self.state, "used": U128(used) }),
        );
        self.state
    }

    /// Anyone: name the factory job this pool posted. The factory's record
    /// must show this pool as the client, with the pool's budget.
    pub fn bind(&mut self, n: u32) -> Promise {
        require!(self.state == State::Posted, "the pool has not posted a job");
        require!(self.job.is_none(), "already bound");
        Promise::new(self.factory.clone())
            .function_call(
                "get_job",
                json!({ "n": n }).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                GAS_VIEW,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_BOUND)
                    .on_bound(n),
            )
    }

    #[private]
    pub fn on_bound(&mut self, n: u32) -> AccountId {
        require!(self.job.is_none(), "already bound");
        let post: FactoryPost = result_bytes()
            .and_then(|b| serde_json::from_slice::<Option<FactoryPost>>(&b).ok())
            .flatten()
            .unwrap_or_else(|| env::panic_str("the factory has no such job"));
        require!(
            post.client == env::current_account_id(),
            "that job's client is not this pool"
        );
        require!(
            post.budget.0 == self.target,
            "that job's budget is not this pool's"
        );
        require!(post.state == "Posted", "that job is not posted");
        emit("bound", json!({ "n": n, "job": post.job }));
        self.job_n = Some(n);
        self.job = Some(post.job.clone());
        post.job
    }

    /// A member approves the job early. When members holding more than half
    /// the stake have voted, the pool (the job's client) approves it.
    pub fn vote_approve(&mut self) -> Option<Promise> {
        let who = env::predecessor_account_id();
        let job = self
            .job
            .clone()
            .unwrap_or_else(|| env::panic_str("no job bound yet"));
        require!(self.state == State::Posted, "the job is not running");
        let m = self
            .members
            .iter_mut()
            .find(|m| m.account == who)
            .unwrap_or_else(|| env::panic_str("only members can vote"));
        m.voted = true;
        let yes: u128 = self
            .members
            .iter()
            .filter(|m| m.voted)
            .map(|m| m.stake.0)
            .sum();
        emit(
            "voted",
            json!({ "member": who, "yes": U128(yes), "raised": U128(self.raised) }),
        );
        if yes * 2 <= self.raised || self.approve_sent {
            return None;
        }
        self.approve_sent = true;
        Some(
            Promise::new(job)
                .function_call(
                    "approve",
                    b"{}".to_vec(),
                    NearToken::from_yoctonear(0),
                    GAS_APPROVE,
                )
                .then(
                    Self::ext(env::current_account_id())
                        .with_static_gas(GAS_ON_APPROVED)
                        .on_approved(),
                ),
        )
    }

    /// The job refused (e.g. nothing delivered yet): any member may vote
    /// again later to resend.
    #[private]
    pub fn on_approved(&mut self) -> bool {
        let ok = env::promise_result_checked(0, 64).is_ok();
        if !ok {
            self.approve_sent = false;
        }
        emit("approve_sent", json!({ "ok": ok }));
        ok
    }

    /// Anyone. Joining closed short of the target, or the launch failed:
    /// share the balance. Posted and bound: only once the job account is
    /// gone (deleted after settle, so its refund has arrived).
    pub fn finish(&mut self) -> Promise {
        let prev = self.state;
        match prev {
            State::Open => require!(
                env::block_timestamp() >= self.closes_ns && self.raised < self.target,
                "the pool is still open"
            ),
            State::LaunchFailed => {}
            State::Posted => require!(self.job.is_some(), "bind the job first"),
            _ => env::panic_str("cannot finish in this state"),
        }
        self.before_finish = prev;
        self.state = State::Finishing;
        match (&self.job, prev) {
            (Some(job), State::Posted) => Promise::new(job.clone())
                .function_call(
                    "get_job",
                    b"{}".to_vec(),
                    NearToken::from_yoctonear(0),
                    GAS_VIEW,
                )
                .then(
                    Self::ext(env::current_account_id())
                        .with_static_gas(GAS_ON_JOB_GONE)
                        .on_job_checked(),
                ),
            _ => self.read_balance(),
        }
    }

    fn read_balance(&self) -> Promise {
        Promise::new(self.token.clone())
            .function_call(
                "ft_balance_of",
                json!({ "account_id": env::current_account_id() })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(0),
                GAS_VIEW,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_BALANCE)
                    .on_balance(),
            )
    }

    /// The job answered: it still exists, so it has not settled. Back to
    /// Posted. No answer: it deleted itself; read the balance.
    #[private]
    pub fn on_job_checked(&mut self) -> Option<Promise> {
        require!(self.state == State::Finishing, "not finishing");
        if env::promise_result_checked(0, 16_384).is_ok() {
            self.state = self.before_finish;
            emit(
                "finish_refused",
                json!({ "reason": "the job has not settled yet" }),
            );
            return None;
        }
        Some(self.read_balance())
    }

    #[private]
    pub fn on_balance(&mut self) -> U128 {
        require!(self.state == State::Finishing, "not finishing");
        let Some(bal) = result_bytes().and_then(|b| serde_json::from_slice::<U128>(&b).ok()) else {
            self.state = self.before_finish;
            emit("finish_refused", json!({ "reason": "balance unreadable" }));
            return U128(0);
        };
        self.payout = bal.0;
        self.state = State::Done;
        emit("finished", json!({ "payout": bal }));
        bal
    }

    /// A member takes its share: payout x stake / raised.
    pub fn claim(&mut self) -> Promise {
        require!(self.state == State::Done, "the pool is not finished");
        let who = env::predecessor_account_id();
        let i = self
            .members
            .iter()
            .position(|m| m.account == who)
            .unwrap_or_else(|| env::panic_str("only members can claim"));
        require!(!self.members[i].claimed, "already claimed");
        let share = self.share(i);
        require!(share > 0, "nothing to claim");
        self.members[i].claimed = true;
        Promise::new(self.token.clone())
            .function_call(
                "ft_transfer",
                json!({ "receiver_id": who, "amount": U128(share) })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(1),
                GAS_TRANSFER,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_CLAIMED)
                    .on_claimed(i as u32, U128(share)),
            )
    }

    #[private]
    pub fn on_claimed(&mut self, i: u32, share: U128) -> bool {
        let ok = env::promise_result_checked(0, 64).is_ok();
        if !ok {
            self.members[i as usize].claimed = false;
        }
        emit(
            "claimed",
            json!({ "member": self.members[i as usize].account, "amount": share, "ok": ok }),
        );
        ok
    }

    fn share(&self, i: usize) -> u128 {
        if self.raised == 0 {
            return 0;
        }
        self.payout * self.members[i].stake.0 / self.raised
    }

    pub fn get_pool(&self) -> serde_json::Value {
        json!({
            "token": self.token, "factory": self.factory, "terms": self.terms,
            "target": U128(self.target), "raised": U128(self.raised),
            "closes_ns": U64(self.closes_ns), "members": self.members,
            "state": self.state, "job_n": self.job_n, "job": self.job,
            "approve_sent": self.approve_sent, "payout": U128(self.payout),
        })
    }
}

#[cfg(test)]
mod tests;
