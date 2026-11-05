//! Hire Tree job contract. One global contract (deployed once, used by code
//! hash) runs every job account:
//!
//! - the root job, e.g. `job-1.hiretree.testnet`, funded by the client;
//! - each sub-job, e.g. `t1.job-1.hiretree.testnet`, created by `hire`.
//!
//! Each account holds that job's fake USDC. `settle` runs bottom-up: pay the
//! payee, send the rest to the parent (root: back to the client), confirm the
//! real token balance is zero, then delete the account (beneficiary: parent).
//!
//! Built: init, funding, bond, hire, deliver (with or without a NEAR AI
//! Cloud receipt, verified by `env::ecrecover`), approve, settle in legs,
//! stuck-settle recovery, delete, child_closed, views.
//! Also built: the quality check (one per delivery, started here), the Job
//! Note sale to the lead's treasury, and sub-job disputes (2 of 3 checks).
//! Not built yet, and refused rather than faked:
//! - root (client) disputes: `dispute` on a root job panics (cut order 4).
//!
//! Security rules (plan section 10): state changes before promises, every
//! callback handles failure, no unwrap on promise results, callbacks are
//! `#[private]` and also assert the caller, checked math everywhere.

use ht_core::binding::{check_binding, parse_addr20_hex, parse_hash32_hex};
use ht_core::check::{self as judgecheck, Outcome, Tally, MAX_ANSWERS, MAX_TASK_BYTES};
use ht_core::receipt::{parse_signature, signed_text_from_hashes, MAX_MODEL_LEN};
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

/// Least NEAR the manager attaches to `hire`; all of it goes to the new
/// child account (its storage), and comes back up the tree on delete.
/// Measured on near-sandbox: a job account uses 463-583 bytes (0.005-0.006
/// NEAR locked); each child record adds about 60 bytes, so 16 children stay
/// under 0.02 NEAR. The manager may attach more to `hire`; it goes to the
/// child too, so the child can hire in turn.
pub const CHILD_BASE_NEAR: NearToken = NearToken::from_millinear(30);
/// Kept by the hiring job from each `hire` deposit: the child's token
/// registration (1.25 mNEAR) plus the job's own new child record (~110
/// bytes, 1.1 mNEAR), with headroom. So a job never pays for its hires out
/// of its own storage NEAR, however many it makes.
pub const HIRE_KEEP_NEAR: NearToken = NearToken::from_micronear(5_000);
/// Least NEAR the manager attaches to `hire` (0.035 NEAR).
pub const HIRE_MIN_NEAR: NearToken = NearToken::from_micronear(35_000);
/// Attached to the token's `storage_deposit` for the child: the NEP-145
/// minimum of a standard NEP-141 (125 bytes; mock-usdc's bounds are exactly
/// this). A token that wants more fails the registration and `hire` rolls back.
pub const TOKEN_STORAGE_NEAR: NearToken = NearToken::from_micronear(1_250);
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
// Each callback's static gas covers the static gas of what it schedules
// plus its own work (checked in `settle_at_minimum_gas_budget_...`).
const GAS_ON_BALANCE_CHECKED: Gas = Gas::from_tgas(25);
const GAS_ON_REST_PAID: Gas = Gas::from_tgas(45);
const GAS_ON_PAYEE_PAID: Gas = Gas::from_tgas(75);
const GAS_ON_RESUME: Gas = Gas::from_tgas(80);
const GAS_ON_RESUME_HIRE: Gas = Gas::from_tgas(20);
const GAS_ON_RESUME_NOTE: Gas = Gas::from_tgas(20);
/// A settle leg with no callback after this long counts as stuck and anyone
/// may `resume_settle` it.
pub const RESUME_AFTER_SECS: u64 = 600;
/// `resume_settle` is the manager's for the first day, then anyone's. It
/// trusts one balance read, and a refunded ft_transfer_call in flight
/// inflates that read for a block or two; the payee, who would gain from
/// it, cannot pick the moment of the read.
pub const RESUME_ANYONE_SECS: u64 = 24 * 3600;
/// Least prepaid gas `hire` accepts (create + callbacks + own work).
pub const GAS_HIRE_MIN: Gas = Gas::from_tgas(150);
/// Least prepaid gas `settle` accepts.
pub const GAS_SETTLE_MIN: Gas = Gas::from_tgas(150);

/// Most deliveries (and so quality checks) one job accepts.
pub const MAX_DELIVERIES: u32 = 3;
/// A check score at or above this passes (scores are 0..=10).
pub const PASS_SCORE: u8 = 6;
pub const MAX_SCORE: u8 = 10;
const GAS_CHECK: Gas = Gas::from_tgas(30);
const GAS_ON_CHECK: Gas = Gas::from_tgas(15);
const GAS_ON_DISPUTE: Gas = Gas::from_tgas(25);
const GAS_ON_NOTE_PAID: Gas = Gas::from_tgas(20);
/// Least prepaid gas for `deliver` when a check will be scheduled.
pub const GAS_DELIVER_MIN: Gas = Gas::from_tgas(90);
pub const GAS_DISPUTE_MIN: Gas = Gas::from_tgas(130);
/// Least gas `ft_on_transfer(buy_note)` must have left to run the sale.
pub const GAS_BUY_MIN: Gas = Gas::from_tgas(60);
/// Largest response a delivery may carry to the checker.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024;
/// How long a signed check round takes answers (then `finalize_check`).
pub const CHECK_WINDOW_SECS: u64 = 300;

/// One signed-check round: panel 0 checks a delivery, panels 1 and 2 are a
/// dispute's extra votes.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckRound {
    pub panel: u8,
    pub seq: u32,
    pub since_ns: U64,
    pub min: u8,
    pub max: u8,
    pub count: u8,
    /// sha256 of each counted answer, so one answer counts once.
    pub seen: Vec<String>,
}

impl CheckRound {
    fn new(panel: u8, seq: u32) -> Self {
        Self {
            panel,
            seq,
            since_ns: U64(env::block_timestamp()),
            min: 0,
            max: 0,
            count: 0,
            seen: Vec::new(),
        }
    }
    fn tally(&self) -> Tally {
        Tally {
            min: self.min,
            max: self.max,
            count: self.count,
        }
    }
    fn closed(&self) -> bool {
        env::block_timestamp()
            >= self
                .since_ns
                .0
                .saturating_add(CHECK_WINDOW_SECS * NS_PER_SEC)
    }
}

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
    /// `resume_hire` is reading the child's balance.
    Resuming,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    pub account: AccountId,
    pub worker: AccountId,
    /// Tokens sent to the child: its worker's pay plus its sub-budget.
    pub total: U128,
    pub state: ChildState,
    /// When the hire started (for `resume_hire`).
    pub since_ns: u64,
}

/// The one settle transfer (or balance check) in flight.
#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leg {
    Payee,
    Rest,
    Balance,
    /// Tokens above the ledger seen by the balance check go up the tree.
    Sweep,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payout {
    pub leg: Leg,
    pub to_payee: U128,
    pub to_rest: U128,
    pub attempt: u32,
    pub since_ns: u64,
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
    pub payout: Option<Payout>,
    pub checker: Option<AccountId>,
    pub treasury: Option<AccountId>,
    pub note_pending: bool,
    pub disputed: bool,
    pub check_pending: bool,
    pub failed_checks: u32,
    pub judge: Option<String>,
    pub rounds: Vec<CheckRound>,
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
    /// The settle leg in flight, if any.
    payout: Option<Payout>,
    /// Bumped on every settle leg; callbacks carry it so a superseded one
    /// cannot act.
    settle_attempt: u32,
    /// Pinned quality checker (OutLayer job runner; `mock-outlayer` in
    /// tests). None = receipt-only mode.
    checker: Option<AccountId>,
    /// The lead's treasury: the only account that may buy this job's note.
    treasury: Option<AccountId>,
    /// A note sale's worker payment is in flight.
    note_pending: bool,
    /// A dispute was raised (one per job).
    disputed: bool,
    dispute_attempt: u32,
    /// When the dispute in flight started.
    dispute_since_ns: Option<u64>,
    /// A quality check for the current delivery is in flight.
    check_pending: bool,
    check_since_ns: u64,
    /// Checks that scored below the pass mark on this job (any delivery).
    failed_checks: u32,
    /// Hashes of every delivery so far (no re-rolling the same bytes).
    delivered: Vec<[u8; 32]>,
    note_since_ns: u64,
    note_price: u128,
    /// Signed-check mode: the pinned judge model. Checks are NEAR AI
    /// receipts for a request this contract builds (see `submit_check`).
    judge: Option<String>,
    /// Open signed-check rounds.
    rounds: Vec<CheckRound>,
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

/// Did promise `idx` succeed? A success with a long result is still a
/// success (only `Failed` counts as failure).
fn result_ok(idx: u64) -> bool {
    !matches!(
        env::promise_result_checked(idx, MAX_RESULT_LEN),
        Err(near_sdk::PromiseError::Failed)
    )
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
        checker: Option<AccountId>,
        treasury: Option<AccountId>,
        judge: Option<String>,
    ) -> Self {
        require!(
            !(checker.is_some() && judge.is_some()),
            "pin a checker contract or a judge model, not both"
        );
        if let Some(m) = &judge {
            require!(
                !m.is_empty() && m.len() <= MAX_MODEL_LEN,
                "judge model name must be 1-128 bytes"
            );
        }
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
        // Root: `amount` is the lead's fee, set by the client at post (may
        // be 0); the rest of the budget goes back to the client at settle.
        if !root {
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
        let payee = if root {
            Some(manager.clone())
        } else {
            worker.clone()
        };
        let job = Self {
            parent,
            root,
            client,
            manager,
            // Root: the manager collects its fee (and bond) at settle.
            payee,
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
            payout: None,
            settle_attempt: 0,
            checker,
            treasury,
            note_pending: false,
            disputed: false,
            dispute_attempt: 0,
            dispute_since_ns: None,
            check_pending: false,
            check_since_ns: 0,
            failed_checks: 0,
            delivered: Vec::new(),
            note_since_ns: 0,
            note_price: 0,
            judge,
            rounds: Vec::new(),
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
            "buy_note" => self.receive_buy_note(sender_id, amount),
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
        // Root: the client, or the factory (our parent) forwarding the
        // client's budget at post time. Sub-job: only the parent job.
        let allowed = sender == self.parent || (self.root && self.client.as_ref() == Some(&sender));
        if !allowed {
            return self.refund(amount, "fund: sender is not the client or parent");
        }
        if self.status != Status::Open || !self.window_open() {
            return self.refund(amount, "fund: job is not open");
        }
        if self.root && self.ledger.funded.saturating_add(amount.0) < self.ledger.amount {
            return self.refund(amount, "fund: the budget must cover the lead's fee");
        }
        if let Err(e) = self.ledger.fund(amount.0) {
            return self.refund(amount, &e.to_string());
        }
        emit("funded", json!({ "from": sender, "amount": amount }));
        PromiseOrValue::Value(U128(0))
    }

    /// The lead's treasury buys the worker's payout right at the quote. The
    /// worker is registered and paid in the same call chain; if that fails,
    /// `on_note_paid` undoes the sale and the token refunds the treasury.
    fn receive_buy_note(&mut self, sender: AccountId, amount: U128) -> PromiseOrValue<U128> {
        if self.treasury.as_ref() != Some(&sender) {
            return self.refund(amount, "buy_note: only the lead's treasury may buy");
        }
        let Some(quote) = self.quote_note() else {
            return self.refund(amount, "buy_note: note is not sellable now");
        };
        if amount != quote {
            return self.refund(amount, "buy_note: amount must equal quote_note");
        }
        if env::prepaid_gas().saturating_sub(env::used_gas()) < GAS_BUY_MIN {
            return self.refund(amount, "buy_note: not enough gas (attach 100 Tgas)");
        }
        let worker = self
            .worker
            .clone()
            .unwrap_or_else(|| env::panic_str("no worker"));
        // State before promises.
        self.note_pending = true;
        self.note_since_ns = env::block_timestamp();
        self.note_price = amount.0;
        self.note_sold = true;
        self.payee = Some(sender.clone());
        emit(
            "note_sale_started",
            json!({ "buyer": sender, "worker": worker, "price": amount, "face": U128(self.ledger.amount) }),
        );
        let pay = Promise::new(self.token.clone())
            .function_call(
                "storage_deposit",
                json!({ "account_id": worker, "registration_only": true })
                    .to_string()
                    .into_bytes(),
                TOKEN_STORAGE_NEAR,
                GAS_STORAGE_DEPOSIT,
            )
            .then(self.ft_transfer(worker.clone(), amount.0))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_NOTE_PAID)
                    .on_note_paid(worker, amount),
            );
        PromiseOrValue::Promise(pay)
    }

    /// After paying the worker its note price. Returns the unused amount to
    /// the token: 0 on success, the whole price (refund) on failure.
    #[private]
    pub fn on_note_paid(&mut self, worker: AccountId, price: U128) -> U128 {
        assert_self();
        require!(self.note_pending, "no note sale in flight");
        self.note_pending = false;
        if result_ok(0) {
            emit(
                "note_sold",
                json!({ "buyer": self.payee, "worker": worker, "price": price }),
            );
            U128(0)
        } else {
            self.note_sold = false;
            self.payee = Some(worker.clone());
            emit(
                "note_sale_failed",
                json!({ "worker": worker, "refund": price }),
            );
            price
        }
    }

    /// Anyone, if a note sale's callback never ran (`RESUME_AFTER_SECS`).
    /// The token then refunded the treasury in full (the receiver's promise
    /// failed). The real balance tells whether the worker was paid anyway,
    /// out of the job's own tokens.
    pub fn resume_note(&mut self) -> Promise {
        require!(self.note_pending, "no note sale in flight");
        require!(
            env::block_timestamp()
                >= self
                    .note_since_ns
                    .saturating_add(RESUME_AFTER_SECS * NS_PER_SEC),
            "note sale is not stuck yet; wait RESUME_AFTER_SECS"
        );
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
                    .with_static_gas(GAS_ON_RESUME_NOTE)
                    .on_resume_note(),
            )
    }

    #[private]
    pub fn on_resume_note(&mut self) -> bool {
        assert_self();
        require!(self.note_pending, "no note sale in flight");
        let Some(U128(real)) = env::promise_result_checked(0, MAX_RESULT_LEN)
            .ok()
            .and_then(|b| serde_json::from_slice::<U128>(&b).ok())
        else {
            emit("note_resume_failed", json!({}));
            return false;
        };
        let price = self.note_price;
        let base = self.ledger.ft_balance;
        let worker = self
            .worker
            .clone()
            .unwrap_or_else(|| env::panic_str("no worker"));
        // Worker paid from our tokens: real == base - price (strays aside).
        let paid_from_job = real < base && base - real >= price;
        if paid_from_job {
            if let Err(e) = self.ledger.prepay(price) {
                fail(e);
            }
        }
        self.note_pending = false;
        self.note_sold = false;
        self.payee = Some(worker.clone());
        emit(
            "note_sale_resumed",
            json!({ "worker": worker, "worker_already_got": U128(if paid_from_job { price } else { 0 }) }),
        );
        paid_from_job
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
    /// the child may hire with. The manager attaches at least
    /// `HIRE_MIN_NEAR` (0.035 NEAR): the job keeps `HIRE_KEEP_NEAR` for the
    /// child's token registration and its own child record, and the rest
    /// goes to the child, so a job never pays for hires from its own storage.
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
            env::attached_deposit() >= HIRE_MIN_NEAR,
            "attach at least 0.035 NEAR for the child account's storage"
        );
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
            since_ns: env::block_timestamp(),
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
            "checker": self.checker,
            "treasury": self.treasury,
            "judge": self.judge,
        });
        // The job keeps HIRE_KEEP_NEAR (child's token registration + its own
        // child record); the rest is the child's storage.
        let child_near = env::attached_deposit().saturating_sub(HIRE_KEEP_NEAR);
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
            // Failed: either the transfer never happened, or it happened and
            // the token's own resolve step failed. Do not guess: leave the
            // hire in flight; `resume_hire` decides from the child's balance.
            Err(_) => {
                self.children[i].since_ns = env::block_timestamp();
                emit("hire_unclear", json!({ "child": self.children[i].account }));
                return false;
            }
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

    /// Anyone, if a hire's create/fund callbacks never finished after
    /// `RESUME_AFTER_SECS`. The child's real token balance decides: it holds
    /// its total -> it is open; it holds nothing -> roll back and close it.
    pub fn resume_hire(&mut self, idx: u32) -> Promise {
        let c = self
            .children
            .get(idx as usize)
            .cloned()
            .unwrap_or_else(|| env::panic_str("no such child"));
        require!(
            matches!(c.state, ChildState::Creating | ChildState::Funding),
            "hire is not in flight"
        );
        require!(
            env::block_timestamp() >= c.since_ns.saturating_add(RESUME_AFTER_SECS * NS_PER_SEC),
            "hire is not stuck yet; wait RESUME_AFTER_SECS"
        );
        // Bump the stage marker so a late original callback is refused.
        self.children[idx as usize].state = ChildState::Resuming;
        Promise::new(self.token.clone())
            .function_call(
                "ft_balance_of",
                json!({ "account_id": c.account }).to_string().into_bytes(),
                NO_DEPOSIT,
                GAS_FT_BALANCE_OF,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_RESUME_HIRE)
                    .on_resume_hire(idx),
            )
    }

    #[private]
    pub fn on_resume_hire(&mut self, idx: u32) -> bool {
        assert_self();
        let i = idx as usize;
        require!(
            self.children.get(i).map(|c| c.state) == Some(ChildState::Resuming),
            "hire is not being resumed"
        );
        let total = self.children[i].total.0;
        let real = env::promise_result_checked(0, MAX_RESULT_LEN)
            .ok()
            .and_then(|b| serde_json::from_slice::<U128>(&b).ok());
        match real {
            Some(U128(r)) if r >= total => {
                self.children_pending -= 1;
                self.children[i].state = ChildState::Open;
                self.children_open += 1;
                emit(
                    "hired",
                    json!({ "child": self.children[i].account, "resumed": true }),
                );
                true
            }
            Some(U128(0)) => {
                self.rollback_hire(i, "resume");
                Promise::new(self.children[i].account.clone())
                    .function_call(
                        "close_unfunded",
                        b"{}".to_vec(),
                        NO_DEPOSIT,
                        GAS_CLOSE_UNFUNDED,
                    )
                    .detach();
                false
            }
            _ => {
                // Unknown or partial: keep waiting; anyone may resume again.
                self.children[i].state = ChildState::Funding;
                self.children[i].since_ns = env::block_timestamp();
                emit(
                    "hire_resume_unclear",
                    json!({ "child": self.children[i].account }),
                );
                false
            }
        }
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
    /// With a NEAR AI Cloud receipt (`req`, `sig`, `model` together) the
    /// contract proves the deliverable came from a model call made for this
    /// task and this delivery (see `verify_receipt`) and sets `verified`.
    /// Without one, the delivery is unverified: paid at settle, note not
    /// sellable.
    pub fn deliver(
        &mut self,
        resp_hash: String,
        req: Option<Base64VecU8>,
        sig: Option<String>,
        model: Option<String>,
        resp: Option<Base64VecU8>,
    ) -> PromiseOrValue<bool> {
        require!(
            self.worker.as_ref() == Some(&env::predecessor_account_id()),
            "only the worker can deliver"
        );
        require!(self.window_open(), "window is closed");
        let hash = parse_hash32_hex(&resp_hash)
            .unwrap_or_else(|| env::panic_str("resp_hash must be 64 hex chars"));
        let next_status = transition(self.status, Event::Deliver).unwrap_or_else(|e| fail(e));
        require!(self.seq < MAX_DELIVERIES, "delivery limit reached");
        require!(
            !self.delivered.contains(&hash),
            "same deliverable as before: no re-rolling the check"
        );
        if let Some(r) = &resp {
            require!(r.0.len() <= MAX_RESPONSE_BYTES, "resp is larger than 16 KB");
            require!(
                env::sha256_array(&r.0) == hash,
                "resp_hash is not sha256(resp)"
            );
        }
        if self.checker.is_some() {
            require!(
                resp.is_some(),
                "a checker is pinned: send the response bytes (resp) so it can read them"
            );
        }
        if self.checker.is_some() {
            require!(
                env::prepaid_gas() >= GAS_DELIVER_MIN,
                "not enough gas for deliver (attach at least 90 Tgas)"
            );
        }
        let seq = self.seq + 1;
        let verified = match (req, sig, model) {
            (None, None, None) => false,
            (Some(req), Some(sig), Some(model)) => {
                self.verify_receipt(&req.0, &sig, &model, &hash, seq);
                true
            }
            _ => env::panic_str("receipt needs req, sig and model together"),
        };
        self.status = next_status;
        self.seq = seq;
        self.deliverable_hash = Some(hash);
        self.verified = verified;
        self.quality = None;
        self.delivered.push(hash);
        emit(
            "delivered",
            json!({ "seq": seq, "deliverable_hash": hex_of(&hash), "verified": verified }),
        );
        match self.checker.clone() {
            Some(checker) => PromiseOrValue::Promise(
                self.start_check(&checker, resp.map(|r| r.0).unwrap_or_default())
                    .then(
                        Self::ext(env::current_account_id())
                            .with_static_gas(GAS_ON_CHECK)
                            .on_check(seq),
                    ),
            ),
            None if self.judge.is_some() => {
                // Signed-check mode: open round 0; anyone submits answers.
                self.check_pending = true;
                self.check_since_ns = env::block_timestamp();
                self.rounds = vec![CheckRound::new(0, seq)];
                emit("check_open", json!({ "seq": seq, "panel": 0 }));
                PromiseOrValue::Value(verified)
            }
            None => {
                // Receipt-only mode: a verified receipt is the check.
                if verified {
                    self.status =
                        transition(self.status, Event::CheckPassed).unwrap_or_else(|e| fail(e));
                    emit(
                        "checked",
                        json!({ "seq": seq, "score": null, "mode": "receipt-only" }),
                    );
                }
                PromiseOrValue::Value(verified)
            }
        }
    }

    fn start_check(&mut self, checker: &AccountId, content: Vec<u8>) -> Promise {
        self.check_pending = true;
        self.check_since_ns = env::block_timestamp();
        self.check_call(checker, 0, &content)
    }

    /// One call to the pinned checker for the current delivery. The seed is
    /// chosen here (block randomness + seq + panel), never by the worker.
    /// The checker gets the deliverable itself (as text), not just its hash.
    fn check_call(&self, checker: &AccountId, panel: u8, content: &[u8]) -> Promise {
        let mut seed_in = env::random_seed();
        seed_in.extend_from_slice(&self.seq.to_le_bytes());
        seed_in.push(panel);
        let seed = env::sha256_array(&seed_in);
        let hash = self
            .deliverable_hash
            .unwrap_or_else(|| env::panic_str("nothing delivered"));
        Promise::new(checker.clone()).function_call(
            "check",
            json!({
                "job": env::current_account_id(),
                "spec_hash": hex_of(&self.spec_hash),
                "deliverable_hash": hex_of(&hash),
                "deliverable": String::from_utf8_lossy(content),
                "seq": self.seq,
                "seed": hex_of(&seed),
                "panel": panel,
            })
            .to_string()
            .into_bytes(),
            NO_DEPOSIT,
            GAS_CHECK,
        )
    }

    /// A score from promise result `idx`, if it is a number in 0..=10.
    fn score_at(idx: u64) -> Option<u8> {
        env::promise_result_checked(idx, MAX_RESULT_LEN)
            .ok()
            .and_then(|b| serde_json::from_slice::<u8>(&b).ok())
            .filter(|s| *s <= MAX_SCORE)
    }

    /// The checker's answer for delivery `seq`.
    #[private]
    pub fn on_check(&mut self, seq: u32) -> bool {
        assert_self();
        require!(
            self.status == Status::Delivered && seq == self.seq && self.check_pending,
            "stale check answer"
        );
        self.check_pending = false;
        self.apply_score(seq, Self::score_at(0))
    }

    fn apply_score(&mut self, seq: u32, score: Option<u8>) -> bool {
        match score {
            None => {
                emit("check_unavailable", json!({ "seq": seq }));
                false
            }
            Some(score) => {
                self.quality = Some(score);
                let pass = score >= PASS_SCORE;
                if !pass {
                    self.failed_checks += 1;
                }
                let ev = if pass {
                    Event::CheckPassed
                } else {
                    Event::CheckFailed
                };
                self.status = transition(self.status, ev).unwrap_or_else(|e| fail(e));
                emit(
                    if pass { "checked" } else { "check_failed" },
                    json!({ "seq": seq, "score": score }),
                );
                pass
            }
        }
    }

    /// Anyone, if the check answer never came after `RESUME_AFTER_SECS`:
    /// the delivery stays unscored (Delivered). An unscored delivery is paid
    /// at settle only if the manager approves it.
    pub fn resume_check(&mut self) -> bool {
        require!(self.check_pending, "no check pending");
        require!(
            self.judge.is_none(),
            "signed checks finish with finalize_check"
        );
        require!(
            env::block_timestamp()
                >= self
                    .check_since_ns
                    .saturating_add(RESUME_AFTER_SECS * NS_PER_SEC),
            "check is not stuck yet; wait RESUME_AFTER_SECS"
        );
        self.check_pending = false;
        emit(
            "check_unavailable",
            json!({ "seq": self.seq, "resumed": true }),
        );
        false
    }

    // ------------------------------------------------------------ dispute

    /// Manager only, on a sub-job, inside the window, once. Two more checks
    /// (panels 1 and 2) run; with the original score, 2 junk votes of 3
    /// uphold the dispute and the pay goes back to the parent at settle.
    pub fn dispute(&mut self, resp: Base64VecU8) -> PromiseOrValue<bool> {
        require!(
            !self.root,
            "root disputes are not built (cut, plan section 11)"
        );
        require!(
            env::predecessor_account_id() == self.manager,
            "only the manager can dispute"
        );
        require!(self.window_open(), "window is closed");
        require!(!self.disputed, "already disputed");
        require!(!self.note_pending, "note sale in flight");
        require!(!self.check_pending, "check pending: wait for its answer");
        require!(
            self.deliverable_hash == Some(env::sha256_array(&resp.0)),
            "resp is not the delivered response"
        );
        require!(
            env::prepaid_gas() >= GAS_DISPUTE_MIN,
            "not enough gas for dispute (attach at least 130 Tgas)"
        );
        require!(
            self.checker.is_some() || self.judge.is_some(),
            "no checker pinned: disputes need one"
        );
        self.status = transition(self.status, Event::Dispute).unwrap_or_else(|e| fail(e));
        self.disputed = true;
        self.dispute_attempt += 1;
        self.dispute_since_ns = Some(env::block_timestamp());
        emit("disputed", json!({ "by": self.manager, "seq": self.seq }));
        let Some(checker) = self.checker.clone() else {
            // Signed-check mode: two more rounds; `finalize_dispute` decides.
            self.rounds = vec![CheckRound::new(1, self.seq), CheckRound::new(2, self.seq)];
            return PromiseOrValue::Value(true);
        };
        PromiseOrValue::Promise(
            self.check_call(&checker, 1, &resp.0)
                .and(self.check_call(&checker, 2, &resp.0))
                .then(
                    Self::ext(env::current_account_id())
                        .with_static_gas(GAS_ON_DISPUTE)
                        .on_dispute(self.dispute_attempt),
                ),
        )
    }

    #[private]
    pub fn on_dispute(&mut self, attempt: u32) -> bool {
        assert_self();
        require!(
            self.status == Status::Disputed
                && attempt == self.dispute_attempt
                && self.dispute_since_ns.is_some(),
            "stale dispute answer"
        );
        let mut votes = vec![self.quality];
        if env::promise_results_count() == 2 {
            votes.push(Self::score_at(0));
            votes.push(Self::score_at(1));
        } else {
            votes.extend([None, None]);
        }
        // Fail closed: a vote that never came counts as junk, so a worker
        // cannot win a dispute by making the checker fail.
        let junk = votes
            .iter()
            .filter(|v| !matches!(v, Some(s) if *s >= PASS_SCORE))
            .count();
        let upheld = junk >= 2;
        self.decide_dispute(upheld, &votes)
    }

    /// Anyone, if the dispute answer never came. Both new votes are
    /// missing, so they count as junk and the dispute is upheld (fail
    /// closed: the worker gains nothing by stalling the checker).
    pub fn resume_dispute(&mut self) -> bool {
        require!(self.status == Status::Disputed, "not disputed");
        require!(
            self.judge.is_none(),
            "signed disputes finish with finalize_dispute"
        );
        let since = self
            .dispute_since_ns
            .unwrap_or_else(|| env::panic_str("no dispute in flight"));
        require!(
            env::block_timestamp() >= since.saturating_add(RESUME_AFTER_SECS * NS_PER_SEC),
            "dispute is not stuck yet; wait RESUME_AFTER_SECS"
        );
        self.dispute_attempt += 1;
        self.decide_dispute(true, &[self.quality, None, None])
    }

    // ------------------------------------------------------------ signed check

    /// The exact request body a judge call for `panel` must send to NEAR AI
    /// Cloud. `task` is the task text (its sha256 is the spec hash) and
    /// `deliverable` the delivered bytes. Anyone may call this view.
    pub fn check_request(&self, panel: u8, task: String, deliverable: Base64VecU8) -> Base64VecU8 {
        Base64VecU8(self.build_check_request(panel, &task, &deliverable.0))
    }

    fn build_check_request(&self, panel: u8, task: &str, deliverable: &[u8]) -> Vec<u8> {
        let model = self
            .judge
            .as_ref()
            .unwrap_or_else(|| env::panic_str("no judge model pinned"));
        require!(
            task.len() <= MAX_TASK_BYTES,
            "task text is larger than 4 KB"
        );
        require!(
            env::sha256_array(task.as_bytes()) == self.spec_hash,
            "task text does not match the spec hash"
        );
        require!(
            deliverable.len() <= MAX_RESPONSE_BYTES,
            "deliverable is larger than 16 KB"
        );
        require!(
            self.deliverable_hash == Some(env::sha256_array(deliverable)),
            "deliverable does not match the delivered hash"
        );
        let text = std::str::from_utf8(deliverable)
            .unwrap_or_else(|_| env::panic_str("deliverable is not UTF-8 text"));
        judgecheck::request_bytes(
            model,
            env::current_account_id().as_str(),
            self.seq,
            panel,
            task,
            text,
        )
    }

    /// Anyone, while a round is open: a NEAR AI Cloud answer to the request
    /// from `check_request`. The contract rebuilds the request bytes itself,
    /// checks the signature over `model:sha256(request):sha256(answer)`
    /// against the pinned signer, and counts the score once per distinct
    /// answer. Returns the score counted.
    pub fn submit_check(
        &mut self,
        panel: u8,
        task: String,
        deliverable: Base64VecU8,
        answer: Base64VecU8,
        sig: String,
    ) -> u8 {
        let i = self
            .rounds
            .iter()
            .position(|r| r.panel == panel && r.seq == self.seq)
            .unwrap_or_else(|| env::panic_str("no open check round for this panel"));
        require!(!self.rounds[i].closed(), "check round is closed");
        require!(
            (self.rounds[i].count as usize) < MAX_ANSWERS as usize,
            "check round is full"
        );
        require!(answer.0.len() <= 4 * 1024, "answer is larger than 4 KB");
        let req = self.build_check_request(panel, &task, &deliverable.0);
        let ans_hash = env::sha256_array(&answer.0);
        let key = hex_of(&ans_hash);
        require!(
            !self.rounds[i].seen.contains(&key),
            "this answer was already counted"
        );
        let model = self.judge.clone().unwrap_or_default();
        self.verify_signed(&req, &ans_hash, &sig, &model);
        let content = serde_json::from_slice::<serde_json::Value>(&answer.0)
            .ok()
            .and_then(|v| {
                v["choices"][0]["message"]["content"]
                    .as_str()
                    .map(String::from)
            })
            .unwrap_or_else(|| env::panic_str("answer has no choices[0].message.content"));
        let score = judgecheck::parse_score(&content)
            .unwrap_or_else(|| env::panic_str("answer is not {\"score\": 0-10}"));
        let r = &mut self.rounds[i];
        let mut t = r.tally();
        t.add(score);
        r.min = t.min;
        r.max = t.max;
        r.count = t.count;
        r.seen.push(key);
        emit(
            "check_answer",
            json!({ "panel": panel, "seq": self.seq, "score": score, "by": env::predecessor_account_id() }),
        );
        score
    }

    fn round_outcome(&self, panel: u8) -> Outcome {
        self.rounds
            .iter()
            .find(|r| r.panel == panel && r.seq == self.seq)
            .map(|r| r.tally().outcome())
            .unwrap_or(Outcome::NoAnswer)
    }

    /// Anyone, after the delivery's round closed. Lowest answer counts;
    /// answers 3+ apart, or none, leave the delivery unscored (paid only if
    /// the manager approves; not counted as a failed check).
    pub fn finalize_check(&mut self) -> bool {
        require!(self.judge.is_some(), "not in signed-check mode");
        require!(self.check_pending, "no check pending");
        let r = self
            .rounds
            .iter()
            .find(|r| r.panel == 0 && r.seq == self.seq)
            .unwrap_or_else(|| env::panic_str("no open check round"));
        require!(r.closed(), "check round is still open");
        let outcome = r.tally().outcome();
        self.check_pending = false;
        self.rounds.clear();
        let seq = self.seq;
        match outcome {
            Outcome::Score(s) => self.apply_score(seq, Some(s)),
            Outcome::Unclear => {
                emit("check_unclear", json!({ "seq": seq }));
                false
            }
            Outcome::NoAnswer => self.apply_score(seq, None),
        }
    }

    /// Anyone, after the dispute rounds closed. Votes: the original score
    /// and panels 1 and 2; a panel with no answer or split answers counts as
    /// junk (fail closed). Two junk votes of three uphold the dispute.
    pub fn finalize_dispute(&mut self) -> bool {
        require!(self.judge.is_some(), "not in signed-check mode");
        require!(self.status == Status::Disputed, "not disputed");
        require!(
            self.rounds.iter().all(|r| r.closed()),
            "dispute rounds are still open"
        );
        let vote = |o: Outcome| match o {
            Outcome::Score(s) => Some(s),
            _ => None,
        };
        let votes = vec![
            self.quality,
            vote(self.round_outcome(1)),
            vote(self.round_outcome(2)),
        ];
        let junk = votes
            .iter()
            .filter(|v| !matches!(v, Some(s) if *s >= PASS_SCORE))
            .count();
        self.rounds.clear();
        self.dispute_attempt += 1;
        self.decide_dispute(junk >= 2, &votes)
    }

    fn decide_dispute(&mut self, upheld: bool, votes: &[Option<u8>]) -> bool {
        let ev = if upheld {
            Event::DisputeUpheld
        } else {
            Event::DisputeRejected
        };
        self.status = transition(self.status, ev).unwrap_or_else(|e| fail(e));
        self.dispute_since_ns = None;
        emit(
            "dispute_decided",
            json!({ "upheld": upheld, "votes": votes, "status": self.status.as_str() }),
        );
        upheld
    }

    /// Panics unless the receipt is valid for this task and delivery:
    /// 1. `req` carries exactly one tag `HT1:<this account>:<spec>:<seq>`;
    /// 2. the text `model:sha256(req):resp_hash` (EIP-191) recovers, with
    ///    low-s enforced, to the pinned NEAR AI Cloud signing address.
    fn verify_receipt(&self, req: &[u8], sig: &str, model: &str, resp_hash: &[u8; 32], seq: u32) {
        require!(
            !model.is_empty() && model.len() <= MAX_MODEL_LEN,
            "receipt: model name must be 1-128 bytes"
        );
        let me = env::current_account_id();
        if let Err(e) = check_binding(req, me.as_str(), &self.spec_hash, seq) {
            env::panic_str(&format!("receipt: {e}"));
        }
        self.verify_signed(req, resp_hash, sig, model);
    }

    /// Panics unless `sig` signs `model:sha256(req):resp_hash` (EIP-191,
    /// low-s) with the pinned NEAR AI Cloud key.
    fn verify_signed(&self, req: &[u8], resp_hash: &[u8; 32], sig: &str, model: &str) {
        let sig = parse_signature(sig).unwrap_or_else(|e| fail(e));
        let req_hash: [u8; 32] = env::sha256_array(req);
        let text = signed_text_from_hashes(model, &req_hash, resp_hash);
        let digest = eip191_digest_env(text.as_bytes());
        let pk = env::ecrecover(&digest, &sig.rs, sig.v, true)
            .unwrap_or_else(|| env::panic_str("receipt: bad signature (no key recovered)"));
        let addr = env::keccak256_array(pk);
        require!(
            addr[12..] == self.signer[..],
            "receipt: signer is not the pinned NEAR AI Cloud key"
        );
    }

    /// Price of this job's note for the lead's treasury, if sellable now.
    /// Needs: Checked, verified receipt, unsold, payee unpaid, a treasury
    /// pinned, no sale in flight. Price: amount minus 1% (fee rounds down).
    pub fn quote_note(&self) -> Option<U128> {
        // A note needs a real quality check (not receipt-only mode) and a
        // clean record: no failed check on any delivery of this job.
        if self.note_pending
            || self.treasury.is_none()
            || (self.checker.is_none() && self.judge.is_none())
            || self.failed_checks > 0
        {
            return None;
        }
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
        require!(!self.check_pending, "check pending: wait for its answer");
        let approvable = if self.root {
            self.status.can_settle_from()
        } else {
            // A refunded (disputed) sub-job may close early too: it only
            // returns the pay to the parent.
            matches!(
                self.status,
                Status::Delivered | Status::Checked | Status::Refunded
            )
        };
        require!(approvable, "job cannot be approved in this status");
        self.approved = true;
        self.window_end_ns = self.window_end_ns.min(env::block_timestamp());
        emit("approved", json!({ "by": caller }));
    }

    // ------------------------------------------------------------ settle

    /// Anyone, after the window or approval, once every child is closed.
    ///
    /// Runs in legs, one transfer at a time, so a stuck settle can always be
    /// resolved from the real token balance alone (see `resume_settle`):
    /// 1. payee leg: register the payee on the token (a no-op refund if it
    ///    already is), then pay it `amount` (only if the work was delivered);
    /// 2. rest leg: send the rest to the parent (root: back to the client);
    /// 3. balance leg: ask the token for the real balance; delete only at zero.
    ///
    /// A failed transfer keeps its tokens here, the job goes back to its
    /// status before settle, and settle can run again without paying anyone
    /// twice.
    pub fn settle(&mut self) -> PromiseOrValue<bool> {
        require!(
            env::prepaid_gas() >= GAS_SETTLE_MIN,
            "not enough gas for settle (attach at least 150 Tgas)"
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
        require!(!self.note_pending, "note sale in flight");
        require!(
            !self.check_pending,
            "check pending: wait for its answer or resume_check"
        );
        // Paid: checked work; or, without a checker, any delivery; or an
        // unscored delivery the manager approved. Unscored work is never
        // paid by default (fail closed).
        let pay_payee = match prev {
            Status::Checked => true,
            Status::Delivered => self.checker.is_none() || (!self.root && self.approved),
            _ => false,
        };
        // Root: fee + bond to the manager, the rest back to the client.
        let split = if self.root {
            self.ledger.settle_split_root()
        } else {
            self.ledger.settle_split(pay_payee)
        }
        .unwrap_or_else(|e| fail(e));
        self.status = settling;
        self.status_before_settle = Some(prev);
        emit(
            "settle_started",
            json!({ "to_payee": U128(split.to_payee), "to_rest": U128(split.to_rest) }),
        );
        if split.to_payee > 0 {
            PromiseOrValue::Promise(self.start_payee_leg(split))
        } else {
            self.start_rest_leg(split)
        }
    }

    fn next_attempt(&mut self, leg: Leg, split: Split) -> u32 {
        self.settle_attempt = self
            .settle_attempt
            .checked_add(1)
            .unwrap_or_else(|| env::panic_str("attempt overflow"));
        self.payout = Some(Payout {
            leg,
            to_payee: U128(split.to_payee),
            to_rest: U128(split.to_rest),
            attempt: self.settle_attempt,
            since_ns: env::block_timestamp(),
        });
        self.settle_attempt
    }

    fn start_payee_leg(&mut self, split: Split) -> Promise {
        let payee = self
            .payee
            .clone()
            .unwrap_or_else(|| env::panic_str("no payee"));
        if let Err(e) = self.ledger.begin_payout(Split {
            to_payee: split.to_payee,
            to_rest: 0,
        }) {
            fail(e);
        }
        let attempt = self.next_attempt(Leg::Payee, split);
        // An unregistered payee would block the transfer forever, so the job
        // registers it (NEP-145 refunds the deposit if already registered).
        self.register(payee.clone())
            .then(self.ft_transfer(payee, split.to_payee))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_PAYEE_PAID)
                    .on_payee_paid(attempt),
            )
    }

    fn start_rest_leg(&mut self, split: Split) -> PromiseOrValue<bool> {
        if split.to_rest == 0 {
            return self.after_transfers(split);
        }
        if let Err(e) = self.ledger.begin_payout(Split {
            to_payee: 0,
            to_rest: split.to_rest,
        }) {
            fail(e);
        }
        let attempt = self.next_attempt(Leg::Rest, split);
        let dest = self.rest_dest();
        // On the root the rest goes back to the client, who may have
        // unregistered since: register it first (refunded if already).
        let transfer = if self.root {
            self.register(dest.clone())
                .then(self.ft_transfer(dest, split.to_rest))
        } else {
            self.ft_transfer(dest, split.to_rest)
        };
        PromiseOrValue::Promise(
            transfer.then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_REST_PAID)
                    .on_rest_paid(attempt),
            ),
        )
    }

    /// Both legs done: anything left means a transfer failed, so stop and
    /// let settle run again; otherwise check the real balance.
    fn after_transfers(&mut self, split: Split) -> PromiseOrValue<bool> {
        if self.ledger.ft_balance != 0 {
            self.settle_failed("transfer failed");
            return PromiseOrValue::Value(false);
        }
        let attempt = self.next_attempt(Leg::Balance, split);
        PromiseOrValue::Promise(self.check_balance(attempt))
    }

    fn register(&self, account: AccountId) -> Promise {
        Promise::new(self.token.clone()).function_call(
            "storage_deposit",
            json!({ "account_id": account, "registration_only": true })
                .to_string()
                .into_bytes(),
            TOKEN_STORAGE_NEAR,
            GAS_STORAGE_DEPOSIT,
        )
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

    fn check_balance(&self, attempt: u32) -> Promise {
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
                    .on_balance_checked(attempt),
            )
    }

    fn settle_failed(&mut self, why: &str) {
        let prev = self
            .status_before_settle
            .take()
            .unwrap_or_else(|| env::panic_str("no status to restore"));
        self.status =
            transition(self.status, Event::SettleFailed(prev)).unwrap_or_else(|e| fail(e));
        self.payout = None;
        emit(
            "settle_failed",
            json!({ "why": why, "status": self.status.as_str() }),
        );
    }

    /// The payout in flight, if `leg` and `attempt` match it. A stale
    /// callback (superseded by `resume_settle`) panics without changing state.
    fn current_payout(&self, leg: Leg, attempt: u32) -> Payout {
        require!(self.status == Status::Settling, "not settling");
        let p = self
            .payout
            .clone()
            .unwrap_or_else(|| env::panic_str("no payout in flight"));
        require!(
            p.leg == leg && p.attempt == attempt,
            "stale settle callback"
        );
        p
    }

    fn resolve_payee(&mut self, p: &Payout, ok: bool) {
        let s = Split {
            to_payee: p.to_payee.0,
            to_rest: 0,
        };
        if let Err(e) = self.ledger.payout_result(s, ok, true) {
            fail(e);
        }
        emit(
            "paid",
            json!({ "leg": "payee", "ok": ok, "to": self.payee, "amount": p.to_payee }),
        );
    }

    fn resolve_rest(&mut self, p: &Payout, ok: bool) {
        let s = Split {
            to_payee: 0,
            to_rest: p.to_rest.0,
        };
        if let Err(e) = self.ledger.payout_result(s, true, ok) {
            fail(e);
        }
        if ok && !self.root {
            self.returned_to_parent = self
                .returned_to_parent
                .checked_add(p.to_rest.0)
                .unwrap_or_else(|| env::panic_str("overflow"));
        }
        emit(
            "paid",
            json!({ "leg": "rest", "ok": ok, "amount": p.to_rest }),
        );
    }

    fn split_of(p: &Payout) -> Split {
        Split {
            to_payee: p.to_payee.0,
            to_rest: p.to_rest.0,
        }
    }

    /// After the payee transfer. On failure the payee's tokens stay here;
    /// the rest leg still runs, so the parent is never held up by the payee.
    #[private]
    pub fn on_payee_paid(&mut self, attempt: u32) -> PromiseOrValue<bool> {
        assert_self();
        let p = self.current_payout(Leg::Payee, attempt);
        self.resolve_payee(&p, result_ok(0));
        self.start_rest_leg(Self::split_of(&p))
    }

    /// After the rest transfer.
    #[private]
    pub fn on_rest_paid(&mut self, attempt: u32) -> PromiseOrValue<bool> {
        assert_self();
        let p = self.current_payout(Leg::Rest, attempt);
        self.resolve_rest(&p, result_ok(0));
        self.after_transfers(Self::split_of(&p))
    }

    /// After `ft_balance_of(self)`. Deletes the account only if the token
    /// reports zero. Stray tokens (sent without `ft_transfer_call`) are
    /// added to the ledger and go to the parent on the next settle.
    #[private]
    pub fn on_balance_checked(&mut self, attempt: u32) -> PromiseOrValue<bool> {
        assert_self();
        self.current_payout(Leg::Balance, attempt);
        let real = env::promise_result_checked(0, MAX_RESULT_LEN)
            .ok()
            .and_then(|b| serde_json::from_slice::<U128>(&b).ok());
        self.apply_real_balance(real)
    }

    fn apply_real_balance(&mut self, real: Option<U128>) -> PromiseOrValue<bool> {
        match real {
            None => {
                self.settle_failed("balance check failed");
                PromiseOrValue::Value(false)
            }
            Some(U128(0)) => PromiseOrValue::Promise(self.finalize()),
            // Never book a surplus: it may be an incoming ft_transfer_call
            // that the token will refund a block later (a phantom). Try to
            // send it up the tree instead; only a real surplus can leave.
            Some(U128(surplus)) => {
                let attempt = self.next_attempt(
                    Leg::Sweep,
                    Split {
                        to_payee: 0,
                        to_rest: surplus,
                    },
                );
                PromiseOrValue::Promise(
                    self.ft_transfer(self.rest_dest(), surplus).then(
                        Self::ext(env::current_account_id())
                            .with_static_gas(GAS_ON_REST_PAID)
                            .on_swept(attempt),
                    ),
                )
            }
        }
    }

    fn rest_dest(&self) -> AccountId {
        if self.root {
            self.client
                .clone()
                .unwrap_or_else(|| env::panic_str("root has no client"))
        } else {
            self.parent.clone()
        }
    }

    /// After sweeping tokens above the ledger. Sent: they count as returned
    /// and the balance is checked again. Not sent: they were a phantom (or
    /// the transfer failed); nothing changed, settle again.
    #[private]
    pub fn on_swept(&mut self, attempt: u32) -> PromiseOrValue<bool> {
        assert_self();
        let p = self.current_payout(Leg::Sweep, attempt);
        if !result_ok(0) {
            self.settle_failed("surplus could not be sent; settle again");
            return PromiseOrValue::Value(false);
        }
        self.returned_to_parent = self
            .returned_to_parent
            .checked_add(p.to_rest.0)
            .unwrap_or_else(|| env::panic_str("overflow"));
        emit(
            "swept",
            json!({ "amount": p.to_rest, "to": self.rest_dest() }),
        );
        let attempt = self.next_attempt(Leg::Balance, Self::split_of(&p));
        PromiseOrValue::Promise(self.check_balance(attempt))
    }

    // ------------------------------------------------------------ recovery

    /// Anyone, when a settle has been stuck in one leg for `RESUME_AFTER_SECS`
    /// (its callback ran out of gas or panicked). Asks the token for the real
    /// balance and works out from it alone whether the leg's transfer
    /// happened, then carries on. Bumps the attempt number first, so a late
    /// original callback can no longer act.
    pub fn resume_settle(&mut self) -> Promise {
        require!(self.status == Status::Settling, "not settling");
        require!(
            env::prepaid_gas() >= GAS_SETTLE_MIN,
            "not enough gas for resume_settle (attach at least 150 Tgas)"
        );
        let p = self
            .payout
            .clone()
            .unwrap_or_else(|| env::panic_str("no payout in flight"));
        let ready = p.since_ns.saturating_add(RESUME_AFTER_SECS * NS_PER_SEC);
        require!(
            env::block_timestamp() >= ready,
            "settle is not stuck yet; wait RESUME_AFTER_SECS"
        );
        require!(
            env::predecessor_account_id() == self.manager
                || env::block_timestamp()
                    >= p.since_ns.saturating_add(RESUME_ANYONE_SECS * NS_PER_SEC),
            "only the manager may resume in the first day"
        );
        let attempt = self.next_attempt(p.leg, Self::split_of(&p));
        emit(
            "settle_resumed",
            json!({ "leg": p.leg, "attempt": attempt }),
        );
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
                    .with_static_gas(GAS_ON_RESUME)
                    .on_resume_balance(attempt),
            )
    }

    /// The real balance after a stuck leg. `base` is what the ledger expects
    /// if the leg's transfer happened. Only one transfer is ever in flight:
    /// at least `base + leg amount` means it did not happen. Tokens above the
    /// ledger are never booked here; the balance leg sweeps them if real.
    #[private]
    pub fn on_resume_balance(&mut self, attempt: u32) -> PromiseOrValue<bool> {
        assert_self();
        require!(self.status == Status::Settling, "not settling");
        let p = self
            .payout
            .clone()
            .unwrap_or_else(|| env::panic_str("no payout in flight"));
        require!(p.attempt == attempt, "stale settle callback");
        let Some(U128(real)) = env::promise_result_checked(0, MAX_RESULT_LEN)
            .ok()
            .and_then(|b| serde_json::from_slice::<U128>(&b).ok())
        else {
            // Token did not answer: stay stuck; anyone may resume again later.
            emit("settle_resume_failed", json!({ "leg": p.leg }));
            return PromiseOrValue::Value(false);
        };
        let base = self.ledger.ft_balance;
        let leg_amount = match p.leg {
            Leg::Payee => p.to_payee.0,
            Leg::Rest => p.to_rest.0,
            Leg::Balance => 0,
            Leg::Sweep => {
                // The ledger never counted the surplus; whether or not it
                // left, a fresh settle sorts it out.
                self.settle_failed("sweep resumed; settle again");
                return PromiseOrValue::Value(false);
            }
        };
        let failed_at = base
            .checked_add(leg_amount)
            .unwrap_or_else(|| env::panic_str("overflow"));
        // Anything above the ledger is never booked here: the balance leg
        // sweeps it later, and only if it really is there.
        let happened = !(real >= failed_at && leg_amount > 0);
        match p.leg {
            Leg::Payee => self.resolve_payee(&p, happened),
            Leg::Rest => self.resolve_rest(&p, happened),
            Leg::Balance => return self.apply_real_balance(Some(U128(real.saturating_sub(base)))),
            Leg::Sweep => unreachable!(),
        }
        match p.leg {
            Leg::Payee => self.start_rest_leg(Self::split_of(&p)),
            _ => self.after_transfers(Self::split_of(&p)),
        }
    }

    fn finalize(&mut self) -> Promise {
        self.status = transition(self.status, Event::SettleOk).unwrap_or_else(|e| fail(e));
        self.status_before_settle = None;
        self.payout = None;
        let me = env::current_account_id();
        emit(
            "settled",
            json!({ "account": me, "beneficiary": self.parent, "returned_to_parent": U128(self.returned_to_parent) }),
        );
        let delete = Promise::new(me).delete_account(self.parent.clone());
        if self.root {
            return delete;
        }
        // A callback may not return a joint promise (the runtime refuses it,
        // found on the sandbox), so the parent notice is detached.
        Promise::new(self.parent.clone())
            .function_call(
                "child_closed",
                json!({ "returned": U128(self.returned_to_parent) })
                    .to_string()
                    .into_bytes(),
                NO_DEPOSIT,
                GAS_CHILD_CLOSED,
            )
            .detach();
        delete
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
            payout: self.payout.clone(),
            checker: self.checker.clone(),
            treasury: self.treasury.clone(),
            note_pending: self.note_pending,
            disputed: self.disputed,
            check_pending: self.check_pending,
            failed_checks: self.failed_checks,
            judge: self.judge.clone(),
            rounds: self.rounds.clone(),
        }
    }
}

/// EIP-191 personal-message digest with the host's keccak (cheaper in wasm
/// than core's pure-Rust one; the core version is the tested reference).
fn eip191_digest_env(msg: &[u8]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(msg.len() + 32);
    buf.extend_from_slice(b"\x19Ethereum Signed Message:\n");
    buf.extend_from_slice(msg.len().to_string().as_bytes());
    buf.extend_from_slice(msg);
    env::keccak256_array(&buf)
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
