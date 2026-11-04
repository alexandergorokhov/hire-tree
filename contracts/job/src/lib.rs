//! Hire Tree job contract. One global contract (deployed once, used by code
//! hash) runs every job account:
//!
//! - the root job, e.g. `job-1.hiretree.testnet`, funded by the client;
//! - each sub-job, e.g. `t1.job-1.hiretree.testnet`, created by `hire`.
//!
//! Each account holds that job's fake USDC. `settle` runs bottom-up: pay the
//! payee, send the rest to the parent (root: to the manager), confirm the
//! real token balance is zero, then delete the account (beneficiary: parent).
//!
//! Built so far (days S and S+1): init, funding, bond, hire, deliver without
//! a receipt, approve, settle, delete, child_closed, views.
//! Not built yet, and refused rather than faked:
//! - receipt verification (`env::ecrecover`, day S+2): `deliver` with a
//!   receipt panics; `verified` stays false;
//! - the quality check (OutLayer, day S+3): nothing reaches `Checked`;
//! - note sale and disputes (day S+4): `buy_note` is refunded in full.
//!
//! Security rules (plan section 10): state changes before promises, every
//! callback handles failure, no unwrap on promise results, callbacks are
//! `#[private]` and also assert the caller, checked math everywhere.

use ht_core::binding::{parse_addr20_hex, parse_hash32_hex};
use ht_core::{transition, Event, Ledger, Split, Status};
use near_sdk::json_types::{Base58CryptoHash, Base64VecU8, U128, U64};
use near_sdk::serde_json::{self, json};
use near_sdk::{
    env, near, require, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseOrValue,
};

#[cfg(test)]
mod tests;

/// Most children one job may hire (bounds gas and state size).
pub const MAX_CHILDREN: usize = 16;
/// Longest approval window: 30 days.
pub const MAX_WINDOW_SECS: u64 = 30 * 24 * 3600;
const NS_PER_SEC: u64 = 1_000_000_000;

/// NEAR a new child account gets from the job's own balance (its storage).
/// The manager may attach more to `hire`; it goes to the child too, so the
/// child can hire in turn. Tuned on the sandbox day.
pub const CHILD_BASE_NEAR: NearToken = NearToken::from_millinear(50);
/// Attached to the token's `storage_deposit` for the child (excess refunded).
pub const TOKEN_STORAGE_NEAR: NearToken = NearToken::from_micronear(12_500);
const ONE_YOCTO: NearToken = NearToken::from_yoctonear(1);
const NO_DEPOSIT: NearToken = NearToken::from_yoctonear(0);

const GAS_INIT: Gas = Gas::from_tgas(15);
const GAS_STORAGE_DEPOSIT: Gas = Gas::from_tgas(10);
const GAS_FT_TRANSFER: Gas = Gas::from_tgas(10);
const GAS_FT_TRANSFER_CALL: Gas = Gas::from_tgas(50);
const GAS_FT_BALANCE_OF: Gas = Gas::from_tgas(5);
const GAS_CLOSE_UNFUNDED: Gas = Gas::from_tgas(10);
const GAS_CHILD_CLOSED: Gas = Gas::from_tgas(10);
const GAS_ON_CHILD_FUNDED: Gas = Gas::from_tgas(25);
const GAS_ON_CHILD_CREATED: Gas = Gas::from_tgas(100);
const GAS_ON_BALANCE_CHECKED: Gas = Gas::from_tgas(30);
const GAS_ON_PAID: Gas = Gas::from_tgas(50);
/// Least prepaid gas `hire` accepts (create + callbacks + own work).
pub const GAS_HIRE_MIN: Gas = Gas::from_tgas(150);
/// Least prepaid gas `settle` accepts.
pub const GAS_SETTLE_MIN: Gas = Gas::from_tgas(100);

/// Largest promise result we read (a U128 JSON string is ~42 bytes).
const MAX_RESULT_LEN: usize = 128;

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildState {
    /// Account create + init in flight.
    Creating,
    /// Token registration + funding in flight.
    Funding,
    /// Funded and running.
    Open,
    /// Settled and deleted; its remainder came back.
    Closed,
    /// Hire failed and was rolled back.
    Failed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    pub account: AccountId,
    pub worker: AccountId,
    /// Tokens sent to the child: its worker's pay plus its sub-budget.
    pub total: U128,
    pub state: ChildState,
}

#[near(serializers = [json])]
pub struct JobView {
    pub account: AccountId,
    pub parent: AccountId,
    pub root: bool,
    pub client: Option<AccountId>,
    pub manager: AccountId,
    pub worker: Option<AccountId>,
    pub payee: Option<AccountId>,
    pub token: AccountId,
    pub status: String,
    pub amount: U128,
    pub funded: U128,
    pub bond: U128,
    pub allocated: U128,
    pub unallocated: U128,
    pub ft_balance: U128,
    pub payee_paid: bool,
    pub returned_to_parent: U128,
    pub spec_hash: String,
    pub window_end_ns: U64,
    pub approved: bool,
    pub seq: u32,
    pub deliverable_hash: Option<String>,
    pub verified: bool,
    pub quality: Option<u8>,
    pub note_sold: bool,
    pub children: Vec<Child>,
    pub children_open: u32,
    pub children_pending: u32,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Job {
    parent: AccountId,
    root: bool,
    client: Option<AccountId>,
    manager: AccountId,
    worker: Option<AccountId>,
    token: AccountId,
    spec_hash: [u8; 32],
    window_end_ns: u64,
    approved: bool,
    /// Pinned NEAR AI Cloud signing address (used from day S+2).
    signer: [u8; 20],
    /// Code hash of the global job contract; children use the same code.
    code_hash: [u8; 32],
    status: Status,
    status_before_settle: Option<Status>,
    deliverable_hash: Option<[u8; 32]>,
    verified: bool,
    quality: Option<u8>,
    payee: Option<AccountId>,
    note_sold: bool,
    seq: u32,
    children: Vec<Child>,
    children_open: u32,
    /// Hires whose create/fund callbacks have not finished.
    children_pending: u32,
    ledger: Ledger,
    /// Tokens sent to the parent so far; reported in `child_closed`.
    returned_to_parent: u128,
}

fn emit(event: &str, data: serde_json::Value) {
    let line = json!({
        "standard": "hiretree",
        "version": "0.1.0",
        "event": event,
        "data": [data],
    });
    env::log_str(&format!("EVENT_JSON:{line}"));
}

fn fail<T: std::fmt::Display>(e: T) -> ! {
    env::panic_str(&e.to_string())
}

/// Callbacks are `#[private]`; this repeats the check inside the body so the
/// unit tests (which call methods directly) cover it too.
fn assert_self() {
    require!(
        env::predecessor_account_id() == env::current_account_id(),
        "callback: caller must be this contract"
    );
}

fn result_ok(idx: u64) -> bool {
    env::promise_result_checked(idx, MAX_RESULT_LEN).is_ok()
}

#[near]
impl Job {
    /// Called by the factory (root job) or the parent job (sub-job) in the
    /// same batch that creates the account. The caller becomes the parent.
    #[init]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: Option<AccountId>,
        manager: AccountId,
        worker: Option<AccountId>,
        token: AccountId,
        amount: U128,
        spec_hash: String,
        window_secs: u64,
        signer: String,
        code_hash: Base58CryptoHash,
    ) -> Self {
        let parent = env::predecessor_account_id();
        let me = env::current_account_id();
        require!(
            me.as_str().ends_with(&format!(".{parent}")),
            "job account must be a sub-account of its creator"
        );
        let root = match (&client, &worker) {
            (Some(_), None) => true,
            (None, Some(_)) => false,
            _ => env::panic_str("exactly one of client (root job) or worker (sub-job)"),
        };
        if root {
            require!(
                amount.0 == 0,
                "a root job has no own pay (amount must be 0)"
            );
        } else {
            require!(amount.0 > 0, "a sub-job's amount must be greater than zero");
        }
        require!(
            window_secs > 0 && window_secs <= MAX_WINDOW_SECS,
            "window_secs out of range"
        );
        let spec = parse_hash32_hex(&spec_hash)
            .unwrap_or_else(|| env::panic_str("spec_hash must be 64 hex chars"));
        let signer = parse_addr20_hex(&signer)
            .unwrap_or_else(|| env::panic_str("signer must be a 20-byte hex address"));
        let window_end_ns = window_secs
            .checked_mul(NS_PER_SEC)
            .and_then(|w| env::block_timestamp().checked_add(w))
            .unwrap_or_else(|| env::panic_str("window overflow"));
        let job = Self {
            parent,
            root,
            client,
            manager,
            payee: worker.clone(),
            worker,
            token,
            spec_hash: spec,
            window_end_ns,
            approved: false,
            signer,
            code_hash: code_hash.into(),
            status: Status::Open,
            status_before_settle: None,
            deliverable_hash: None,
            verified: false,
            quality: None,
            note_sold: false,
            seq: 0,
            children: Vec::new(),
            children_open: 0,
            children_pending: 0,
            ledger: Ledger::new(amount.0),
            returned_to_parent: 0,
        };
        emit(
            "job_created",
            json!({ "account": me, "parent": job.parent, "root": root, "amount": amount }),
        );
        job
    }

    // ------------------------------------------------------------ token in

    /// NEP-141 receiver. `msg`: `fund`, `bond` or `buy_note`. Anything not
    /// accepted is refunded in full (returned as unused), with no state change.
    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        if env::predecessor_account_id() != self.token {
            return self.refund(amount, "wrong token");
        }
        if amount.0 == 0 {
            return self.refund(amount, "zero amount");
        }
        match msg.as_str() {
            "fund" => self.receive_fund(sender_id, amount),
            "bond" => self.receive_bond(sender_id, amount),
            // TODO(day S+4): note sale to the lead's pinned treasury. Until
            // receipts (S+2) and the quality check (S+3) exist, no note is
            // sellable, so refuse instead of faking a sale.
            "buy_note" => self.refund(
                amount,
                "note not sellable: needs a verified receipt and a passed check (not built yet)",
            ),
            _ => self.refund(amount, "unknown msg"),
        }
    }

    fn refund(&self, amount: U128, reason: &str) -> PromiseOrValue<U128> {
        emit("refund", json!({ "amount": amount, "reason": reason }));
        PromiseOrValue::Value(amount)
    }

    fn window_open(&self) -> bool {
        !self.approved && env::block_timestamp() < self.window_end_ns
    }

    fn receive_fund(&mut self, sender: AccountId, amount: U128) -> PromiseOrValue<U128> {
        let allowed = if self.root {
            self.client.as_ref() == Some(&sender)
        } else {
            sender == self.parent
        };
        if !allowed {
            return self.refund(amount, "fund: sender is not the client or parent");
        }
        if self.status != Status::Open || !self.window_open() {
            return self.refund(amount, "fund: job is not open");
        }
        if let Err(e) = self.ledger.fund(amount.0) {
            return self.refund(amount, &e.to_string());
        }
        emit("funded", json!({ "from": sender, "amount": amount }));
        PromiseOrValue::Value(U128(0))
    }

    fn receive_bond(&mut self, sender: AccountId, amount: U128) -> PromiseOrValue<U128> {
        if !self.root || sender != self.manager {
            return self.refund(amount, "bond: only the manager, on the root job");
        }
        if self.status != Status::Open || !self.window_open() {
            return self.refund(amount, "bond: job is not open");
        }
        if let Err(e) = self.ledger.add_bond(amount.0) {
            return self.refund(amount, &e.to_string());
        }
        emit("bond", json!({ "from": sender, "amount": amount }));
        PromiseOrValue::Value(U128(0))
    }

    // ------------------------------------------------------------ hire

    /// Manager only. Creates `t{n}.<this account>` from the global contract
    /// (by code hash), registers it on the token and sends it
    /// `amount + sub` tokens. `amount` is the worker's pay; `sub` is a budget
    /// the child may hire with. Attached NEAR goes to the child on top of
    /// `CHILD_BASE_NEAR`.
    #[payable]
    pub fn hire(
        &mut self,
        worker: AccountId,
        amount: U128,
        spec_hash: String,
        sub: Option<U128>,
    ) -> Promise {
        require!(
            env::predecessor_account_id() == self.manager,
            "only the manager can hire"
        );
        require!(self.status == Status::Open, "job is not open");
        require!(self.window_open(), "window is closed");
        require!(self.children.len() < MAX_CHILDREN, "too many children");
        require!(amount.0 > 0, "amount must be greater than zero");
        require!(
            env::prepaid_gas() >= GAS_HIRE_MIN,
            "not enough gas for hire (attach at least 150 Tgas)"
        );
        let me = env::current_account_id();
        require!(worker != me, "worker cannot be the job itself");
        let spec = parse_hash32_hex(&spec_hash)
            .unwrap_or_else(|| env::panic_str("spec_hash must be 64 hex chars"));
        let total = amount
            .0
            .checked_add(sub.map(|s| s.0).unwrap_or(0))
            .unwrap_or_else(|| env::panic_str("amount overflow"));

        let idx = self.children.len();
        let child: AccountId = format!("t{}.{}", idx + 1, me)
            .parse()
            .unwrap_or_else(|_| env::panic_str("child account id is invalid or too long"));

        // State before promises.
        if let Err(e) = self.ledger.allocate(total) {
            fail(e);
        }
        self.children.push(Child {
            account: child.clone(),
            worker: worker.clone(),
            total: U128(total),
            state: ChildState::Creating,
        });
        self.children_pending += 1;

        let remaining_secs = (self.window_end_ns - env::block_timestamp()) / NS_PER_SEC;
        let args = json!({
            "client": null,
            "manager": self.manager,
            "worker": worker,
            "token": self.token,
            "amount": amount,
            "spec_hash": hex_of(&spec),
            "window_secs": remaining_secs.max(1),
            "signer": hex_of(&self.signer),
            "code_hash": Base58CryptoHash::from(self.code_hash),
        });
        let child_near = CHILD_BASE_NEAR
            .checked_add(env::attached_deposit())
            .unwrap_or_else(|| env::panic_str("deposit overflow"));
        emit(
            "hire_started",
            json!({ "child": child, "worker": worker, "amount": amount, "total": U128(total) }),
        );
        Promise::new(child)
            .create_account()
            .transfer(child_near)
            .use_global_contract(self.code_hash)
            .function_call("new", args.to_string().into_bytes(), NO_DEPOSIT, GAS_INIT)
            .then(
                Self::ext(me)
                    .with_static_gas(GAS_ON_CHILD_CREATED)
                    .on_child_created(idx as u32),
            )
    }

    /// After create + init. On failure: roll back the allocation (the NEAR
    /// sent with the failed create is refunded to this account by the runtime).
    #[private]
    pub fn on_child_created(&mut self, idx: u32) -> PromiseOrValue<bool> {
        assert_self();
        let i = idx as usize;
        require!(
            self.children.get(i).map(|c| c.state) == Some(ChildState::Creating),
            "child is not being created"
        );
        if !result_ok(0) {
            self.rollback_hire(i, "create");
            return PromiseOrValue::Value(false);
        }
        self.children[i].state = ChildState::Funding;
        let child = self.children[i].account.clone();
        let total = self.children[i].total;
        let register = Promise::new(self.token.clone()).function_call(
            "storage_deposit",
            json!({ "account_id": child, "registration_only": true })
                .to_string()
                .into_bytes(),
            TOKEN_STORAGE_NEAR,
            GAS_STORAGE_DEPOSIT,
        );
        // If registration failed, ft_transfer_call fails too and the next
        // callback rolls back.
        let fund = Promise::new(self.token.clone()).function_call(
            "ft_transfer_call",
            json!({ "receiver_id": child, "amount": total, "msg": "fund" })
                .to_string()
                .into_bytes(),
            ONE_YOCTO,
            GAS_FT_TRANSFER_CALL,
        );
        PromiseOrValue::Promise(
            register.then(fund).then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_CHILD_FUNDED)
                    .on_child_funded(idx),
            ),
        )
    }

    /// After `ft_transfer_call(child, total, "fund")`. Its result is the
    /// amount the child kept. Anything it did not keep is still ours.
    #[private]
    pub fn on_child_funded(&mut self, idx: u32) -> bool {
        assert_self();
        let i = idx as usize;
        require!(
            self.children.get(i).map(|c| c.state) == Some(ChildState::Funding),
            "child is not being funded"
        );
        let total = self.children[i].total.0;
        let used = match env::promise_result_checked(0, MAX_RESULT_LEN) {
            // The token reverted the transfer: nothing moved.
            Err(_) => 0,
            Ok(bytes) => match serde_json::from_slice::<U128>(&bytes) {
                Ok(u) => u.0.min(total),
                // Unreadable answer from the token: assume the tokens left,
                // so they are never spent twice.
                Err(_) => {
                    emit(
                        "anomaly",
                        json!({ "child": self.children[i].account, "what": "unreadable ft_transfer_call result" }),
                    );
                    total
                }
            },
        };
        if used == 0 {
            self.rollback_hire(i, "fund");
            // The child exists but holds nothing: ask it to delete itself
            // (its NEAR comes back here).
            Promise::new(self.children[i].account.clone())
                .function_call(
                    "close_unfunded",
                    b"{}".to_vec(),
                    NO_DEPOSIT,
                    GAS_CLOSE_UNFUNDED,
                )
                .detach();
            return false;
        }
        self.children_pending -= 1;
        if used < total {
            // Not expected (a job keeps all or nothing); keep the books right.
            if let Err(e) = self.ledger.release(total - used) {
                fail(e);
            }
            self.children[i].total = U128(used);
            emit(
                "anomaly",
                json!({ "child": self.children[i].account, "what": "partial funding", "used": U128(used) }),
            );
        }
        self.children[i].state = ChildState::Open;
        self.children_open += 1;
        emit(
            "hired",
            json!({ "child": self.children[i].account, "worker": self.children[i].worker, "total": U128(used) }),
        );
        true
    }

    fn rollback_hire(&mut self, i: usize, stage: &str) {
        let total = self.children[i].total.0;
        if let Err(e) = self.ledger.release(total) {
            fail(e);
        }
        self.children[i].state = ChildState::Failed;
        self.children_pending -= 1;
        emit(
            "hire_failed",
            json!({ "child": self.children[i].account, "stage": stage }),
        );
    }

    /// Parent only: delete this sub-job if its funding never arrived.
    pub fn close_unfunded(&mut self) -> Promise {
        require!(
            env::predecessor_account_id() == self.parent,
            "only the parent can close an unfunded job"
        );
        require!(
            !self.root && self.ledger.funded == 0 && self.ledger.ft_balance == 0,
            "job has funds"
        );
        let settling = transition(self.status, Event::StartSettle).unwrap_or_else(|e| fail(e));
        self.status = transition(settling, Event::SettleOk).unwrap_or_else(|e| fail(e));
        emit(
            "closed_unfunded",
            json!({ "account": env::current_account_id() }),
        );
        Promise::new(env::current_account_id()).delete_account(self.parent.clone())
    }

    // ------------------------------------------------------------ deliver

    /// Worker only, while the window is open. Stores the deliverable hash
    /// (`sha256(response)`, 64 hex) and increments `seq`.
    ///
    /// Receipt verification is not built yet (day S+2): passing `req`, `sig`
    /// or `model` is refused. A delivery without a receipt waits for settle.
    pub fn deliver(
        &mut self,
        resp_hash: String,
        req: Option<Base64VecU8>,
        sig: Option<String>,
        model: Option<String>,
    ) {
        require!(
            self.worker.as_ref() == Some(&env::predecessor_account_id()),
            "only the worker can deliver"
        );
        // TODO(day S+2): rebuild the signed text, EIP-191 digest, ecrecover,
        // pinned signer, HT1 binding tag (ht_core::binding), used seq.
        require!(
            req.is_none() && sig.is_none() && model.is_none(),
            "receipt verification is not built yet: deliver with resp_hash only"
        );
        require!(self.window_open(), "window is closed");
        let hash = parse_hash32_hex(&resp_hash)
            .unwrap_or_else(|| env::panic_str("resp_hash must be 64 hex chars"));
        self.status = transition(self.status, Event::Deliver).unwrap_or_else(|e| fail(e));
        self.seq = self
            .seq
            .checked_add(1)
            .unwrap_or_else(|| env::panic_str("seq overflow"));
        self.deliverable_hash = Some(hash);
        self.verified = false;
        // TODO(day S+3): start the one quality check per delivery here.
        emit(
            "delivered",
            json!({ "seq": self.seq, "deliverable_hash": hex_of(&hash), "verified": false }),
        );
    }

    /// Price of this job's note for the lead's treasury, if sellable now.
    /// Always `null` until receipts and the quality check are built.
    pub fn quote_note(&self) -> Option<U128> {
        self.ledger
            .sellable_quote(self.status, self.verified, self.note_sold)
            .map(U128)
    }

    /// Root: the client accepts early. Sub-job: the manager (the sub-job's
    /// client) accepts delivered work early; it cannot approve before a
    /// delivery, or it could close the window on its own worker. Ends the
    /// window so `settle` may run now.
    pub fn approve(&mut self) {
        let caller = env::predecessor_account_id();
        let allowed = if self.root {
            self.client.as_ref() == Some(&caller)
        } else {
            caller == self.manager
        };
        require!(
            allowed,
            "only the client (root) or manager (sub-job) can approve"
        );
        require!(!self.approved, "already approved");
        let approvable = if self.root {
            self.status.can_settle_from()
        } else {
            matches!(self.status, Status::Delivered | Status::Checked)
        };
        require!(approvable, "job cannot be approved in this status");
        self.approved = true;
        self.window_end_ns = self.window_end_ns.min(env::block_timestamp());
        emit("approved", json!({ "by": caller }));
    }

    // ------------------------------------------------------------ settle

    /// Anyone, after the window or approval, once every child is closed.
    /// Pays the payee its amount (only if the work was delivered), sends the
    /// rest to the parent (root: to the manager). Deletion happens later, in
    /// a callback, only after every transfer succeeded and the token reports
    /// a zero balance.
    pub fn settle(&mut self) -> Promise {
        require!(
            env::prepaid_gas() >= GAS_SETTLE_MIN,
            "not enough gas for settle (attach at least 100 Tgas)"
        );
        let prev = self.status;
        let settling = transition(prev, Event::StartSettle).unwrap_or_else(|e| fail(e));
        require!(
            self.approved || env::block_timestamp() >= self.window_end_ns,
            "window is still open"
        );
        require!(
            self.children_open == 0 && self.children_pending == 0,
            "children still open"
        );
        let pay_payee = matches!(prev, Status::Delivered | Status::Checked);
        let split = self
            .ledger
            .settle_split(pay_payee)
            .unwrap_or_else(|e| fail(e));
        if let Err(e) = self.ledger.begin_payout(split) {
            fail(e);
        }
        self.status = settling;
        self.status_before_settle = Some(prev);
        emit(
            "settle_started",
            json!({ "to_payee": U128(split.to_payee), "to_rest": U128(split.to_rest) }),
        );

        let mut transfers: Option<Promise> = None;
        if split.to_payee > 0 {
            let payee = self
                .payee
                .clone()
                .unwrap_or_else(|| env::panic_str("no payee"));
            transfers = Some(self.ft_transfer(payee, split.to_payee));
        }
        if split.to_rest > 0 {
            let dest = if self.root {
                self.manager.clone()
            } else {
                self.parent.clone()
            };
            let t = self.ft_transfer(dest, split.to_rest);
            transfers = Some(match transfers {
                Some(p) => p.and(t),
                None => t,
            });
        }
        match transfers {
            Some(p) => p.then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_PAID)
                    .on_paid(U128(split.to_payee), U128(split.to_rest)),
            ),
            // Nothing to send: go straight to the balance check.
            None => self.check_balance_then_delete(),
        }
    }

    fn ft_transfer(&self, to: AccountId, amount: u128) -> Promise {
        Promise::new(self.token.clone()).function_call(
            "ft_transfer",
            json!({ "receiver_id": to, "amount": U128(amount), "memo": "hiretree settle" })
                .to_string()
                .into_bytes(),
            ONE_YOCTO,
            GAS_FT_TRANSFER,
        )
    }

    fn check_balance_then_delete(&self) -> Promise {
        Promise::new(self.token.clone())
            .function_call(
                "ft_balance_of",
                json!({ "account_id": env::current_account_id() })
                    .to_string()
                    .into_bytes(),
                NO_DEPOSIT,
                GAS_FT_BALANCE_OF,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_BALANCE_CHECKED)
                    .on_balance_checked(),
            )
    }

    fn settle_failed(&mut self, why: &str) {
        let prev = self
            .status_before_settle
            .take()
            .unwrap_or_else(|| env::panic_str("no status to restore"));
        self.status =
            transition(self.status, Event::SettleFailed(prev)).unwrap_or_else(|e| fail(e));
        emit(
            "settle_failed",
            json!({ "why": why, "status": self.status.as_str() }),
        );
    }

    /// After the settle transfers. A failed transfer's tokens stay here and
    /// the job goes back to its status before settle; settle can run again.
    #[private]
    pub fn on_paid(&mut self, to_payee: U128, to_rest: U128) -> PromiseOrValue<bool> {
        assert_self();
        require!(self.status == Status::Settling, "not settling");
        let split = Split {
            to_payee: to_payee.0,
            to_rest: to_rest.0,
        };
        let expected = u64::from(split.to_payee > 0) + u64::from(split.to_rest > 0);
        let (payee_ok, rest_ok) = if env::promise_results_count() != expected {
            // Not the shape we scheduled: treat every transfer as failed.
            (split.to_payee == 0, split.to_rest == 0)
        } else {
            let mut idx = 0;
            let payee_ok = if split.to_payee > 0 {
                idx += 1;
                result_ok(idx - 1)
            } else {
                true
            };
            let rest_ok = if split.to_rest > 0 {
                result_ok(idx)
            } else {
                true
            };
            (payee_ok, rest_ok)
        };
        if let Err(e) = self.ledger.payout_result(split, payee_ok, rest_ok) {
            fail(e);
        }
        if rest_ok && split.to_rest > 0 && !self.root {
            self.returned_to_parent = self
                .returned_to_parent
                .checked_add(split.to_rest)
                .unwrap_or_else(|| env::panic_str("overflow"));
        }
        emit(
            "paid",
            json!({ "payee_ok": payee_ok, "rest_ok": rest_ok, "to_payee": to_payee, "to_rest": to_rest }),
        );
        if self.ledger.ft_balance != 0 {
            self.settle_failed("transfer failed");
            return PromiseOrValue::Value(false);
        }
        PromiseOrValue::Promise(self.check_balance_then_delete())
    }

    /// After `ft_balance_of(self)`. Deletes the account only if the token
    /// reports zero. Stray tokens (sent without `ft_transfer_call`) are
    /// added to the ledger and go to the parent on the next settle.
    #[private]
    pub fn on_balance_checked(&mut self) -> PromiseOrValue<bool> {
        assert_self();
        require!(self.status == Status::Settling, "not settling");
        let real = env::promise_result_checked(0, MAX_RESULT_LEN)
            .ok()
            .and_then(|b| serde_json::from_slice::<U128>(&b).ok());
        match real {
            None => {
                self.settle_failed("balance check failed");
                PromiseOrValue::Value(false)
            }
            Some(U128(0)) => PromiseOrValue::Promise(self.finalize()),
            Some(U128(stray)) => {
                if let Err(e) = self.ledger.credit(stray) {
                    fail(e);
                }
                self.settle_failed("stray tokens found; settle again");
                PromiseOrValue::Value(false)
            }
        }
    }

    fn finalize(&mut self) -> Promise {
        self.status = transition(self.status, Event::SettleOk).unwrap_or_else(|e| fail(e));
        self.status_before_settle = None;
        let me = env::current_account_id();
        emit(
            "settled",
            json!({ "account": me, "beneficiary": self.parent, "returned_to_parent": U128(self.returned_to_parent) }),
        );
        let delete = Promise::new(me).delete_account(self.parent.clone());
        if self.root {
            return delete;
        }
        let notify = Promise::new(self.parent.clone()).function_call(
            "child_closed",
            json!({ "returned": U128(self.returned_to_parent) })
                .to_string()
                .into_bytes(),
            NO_DEPOSIT,
            GAS_CHILD_CLOSED,
        );
        notify.and(delete)
    }

    /// Called by a settled child just before it is deleted. Only a known,
    /// open child is accepted. `returned` is what it sent back to us.
    pub fn child_closed(&mut self, returned: U128) {
        let caller = env::predecessor_account_id();
        let i = self
            .children
            .iter()
            .position(|c| c.account == caller && c.state == ChildState::Open)
            .unwrap_or_else(|| env::panic_str("caller is not an open child of this job"));
        self.children[i].state = ChildState::Closed;
        self.children_open = self
            .children_open
            .checked_sub(1)
            .unwrap_or_else(|| env::panic_str("children_open underflow"));
        if let Err(e) = self.ledger.credit(returned.0) {
            fail(e);
        }
        emit(
            "child_closed",
            json!({ "child": caller, "returned": returned }),
        );
    }

    // ------------------------------------------------------------ views

    pub fn get_job(&self) -> JobView {
        JobView {
            account: env::current_account_id(),
            parent: self.parent.clone(),
            root: self.root,
            client: self.client.clone(),
            manager: self.manager.clone(),
            worker: self.worker.clone(),
            payee: self.payee.clone(),
            token: self.token.clone(),
            status: self.status.as_str().to_string(),
            amount: U128(self.ledger.amount),
            funded: U128(self.ledger.funded),
            bond: U128(self.ledger.bond),
            allocated: U128(self.ledger.allocated),
            unallocated: U128(self.ledger.unallocated()),
            ft_balance: U128(self.ledger.ft_balance),
            payee_paid: self.ledger.payee_paid,
            returned_to_parent: U128(self.returned_to_parent),
            spec_hash: hex_of(&self.spec_hash),
            window_end_ns: U64(self.window_end_ns),
            approved: self.approved,
            seq: self.seq,
            deliverable_hash: self.deliverable_hash.as_ref().map(|h| hex_of(h)),
            verified: self.verified,
            quality: self.quality,
            note_sold: self.note_sold,
            children: self.children.clone(),
            children_open: self.children_open,
            children_pending: self.children_pending,
        }
    }
}

fn hex_of(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}
