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
const GAS_ON_BALANCE_CHECKED: Gas = Gas::from_tgas(35);
/// The track-record report to the registry (factory) at delete.
pub const GAS_RECORD: Gas = Gas::from_tgas(10);
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
/// `award_best`: gas for each record read (a factory view), and for its own
/// work and the callback's before the hire it schedules.
pub const GAS_GET_RECORD: Gas = Gas::from_tgas(5);
pub const GAS_AWARD_OWN: Gas = Gas::from_tgas(20);
/// An award must leave the winner at least this much of the window: the
/// child's window is what is left of this job's, and a winner that cannot
/// deliver in time loses its bond to the buyer.
pub const MIN_WORK_SECS: u64 = 60;
const TOO_LATE: &str = "too late to award: the winner needs at least 60 s of the window to deliver";
/// Most managers a buyer may list in `award_best`'s `payers` (the factory
/// refuses a longer list too).
pub const MAX_LISTED_PAYERS: usize = 16;
/// The largest record a read may return.
const MAX_RECORD_LEN: usize = 2048;
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

/// Bidding: at most this many open-or-awarded slots per job, and bids per slot.
pub const MAX_BID_SLOTS: usize = 16;
/// Storage deposit for a new bid (about 100 bytes at 1e19 yocto per byte).
pub const BID_NEAR: NearToken = NearToken::from_millinear(1);
/// A sealed bid also stores its 32-byte commit (as hex), so it pays more.
pub const SEALED_BID_NEAR: NearToken = NearToken::from_millinear(2);
pub const MAX_BIDS: usize = 16;
/// A bid slot stays open between 10 seconds and 1 hour.
pub const MIN_BID_SECS: u64 = 10;
pub const MAX_BID_SECS: u64 = 3600;

/// One worker's offer on a bid slot.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bid {
    pub worker: AccountId,
    pub price: U128,
    /// NEAR the bidder staked (yocto); 0 if the slot asks for none.
    #[serde(default)]
    pub bond: U128,
    /// Sealed slot: `sha256("<price>:<salt>:<bidder>")` hex until revealed
    /// (the price is 0 until then).
    #[serde(default)]
    pub commit: Option<String>,
    /// Underwriting: the premium (yocto NEAR, out of the bidder's own bond)
    /// the bidder offers whoever backs it (`seek_backer`).
    #[serde(default)]
    pub offer: U128,
    /// The underwriter that backed this bid, and the NEAR it put at risk.
    #[serde(default)]
    pub backer: Option<AccountId>,
    #[serde(default)]
    pub backed: U128,
}

/// A subtask the manager put out for bids: workers offer a price up to
/// `max` until `closes_ns`; then the manager awards one bidder, who is hired
/// at exactly its own bid.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BidSlot {
    pub spec_hash: String,
    pub max: U128,
    pub closes_ns: U64,
    pub bids: Vec<Bid>,
    pub awarded: Option<AccountId>,
    /// NEAR (yocto) each bidder must stake. Losers get it back at the
    /// award; the winner gets it back if its work is paid, and the buyer
    /// keeps it if not.
    #[serde(default)]
    pub bond: U128,
    /// Sealed slot: bids are hashes until `closes_ns`, revealed until
    /// `reveal_ns`; 0 = an open slot.
    #[serde(default)]
    pub reveal_ns: U64,
}

/// The largest bond a slot may ask for: 5 NEAR.
pub const MAX_BOND: NearToken = NearToken::from_near(5);

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
    /// Hired by this job's own worker out of its pay (contractor mode).
    #[serde(default)]
    pub by_worker: bool,
    /// The worker's bond (yocto NEAR), held here until the child closes.
    #[serde(default)]
    pub bond: U128,
    /// Its underwriter, the NEAR it backed the worker with, and the premium
    /// it earns out of the worker's bond if the work is paid.
    #[serde(default)]
    pub backer: Option<AccountId>,
    #[serde(default)]
    pub backed: U128,
    #[serde(default)]
    pub premium: U128,
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
    pub registry: AccountId,
    pub by_bid: bool,
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
    /// Bid slots, in the order they were opened.
    bid_slots: Vec<BidSlot>,
    /// Where workers' track records live (the factory).
    registry: AccountId,
    /// The worker won this job with its own bid (an award), so it took the
    /// job on. A plain `hire` asks nobody.
    by_bid: bool,
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

/// The fields of a factory `Record` the award rule weighs (`payers` is only
/// reported, in the event).
#[near(serializers = [json])]
#[derive(Default)]
struct RecordView {
    #[serde(default)]
    paid_value: U128,
    #[serde(default)]
    unpaid_value: U128,
    #[serde(default)]
    payers: u32,
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
        registry: Option<AccountId>,
        bid: Option<bool>,
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
            bid_slots: Vec::new(),
            // Root: its creator (the factory). Sub-jobs inherit it.
            registry: registry.unwrap_or_else(env::predecessor_account_id),
            by_bid: bid == Some(true),
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
        let by_worker = self.hiring_role("hire");
        self.require_hire_resources(env::prepaid_gas());
        self.hire_inner(
            worker,
            amount,
            spec_hash,
            sub,
            by_worker,
            env::attached_deposit(),
            false,
        )
    }

    /// What every hire needs attached (also an `award`'s, and an
    /// `award_best`'s, whose hire happens in a callback).
    fn require_hire_resources(&self, prepaid: Gas) {
        require!(
            env::attached_deposit() >= HIRE_MIN_NEAR,
            "attach at least 0.035 NEAR for the child account's storage"
        );
        require!(
            prepaid >= GAS_HIRE_MIN,
            "not enough gas for hire (attach at least 150 Tgas)"
        );
    }

    /// The manager may hire in any job. A sub-job's own worker may hire too
    /// (a contractor): it pays for each sub-hire out of its own pay, the
    /// sub-job's manager is the worker, and unspent or refunded pay comes
    /// back to it. Returns true for a worker hire.
    fn hiring_role(&self, what: &str) -> bool {
        let caller = env::predecessor_account_id();
        if caller == self.manager {
            return false;
        }
        if !self.root && self.worker.as_ref() == Some(&caller) {
            require!(
                !self.note_sold && !self.note_pending,
                "the note is sold: the pay is no longer yours to spend"
            );
            return true;
        }
        env::panic_str(&format!("only the manager or this job's worker can {what}"))
    }

    /// Manager only, while the job is open: put a subtask out for bids.
    /// `max` is the most the manager will pay; it must fit the free budget.
    /// Returns the slot number.
    pub fn open_bids(
        &mut self,
        spec_hash: String,
        max: U128,
        secs: u64,
        bond: Option<U128>,
        sealed: Option<bool>,
    ) -> u32 {
        let by_worker = self.hiring_role("open bids");
        require!(self.status == Status::Open, "job is not open");
        require!(self.window_open(), "window is closed");
        require!(self.bid_slots.len() < MAX_BID_SLOTS, "too many bid slots");
        require!(
            (MIN_BID_SECS..=MAX_BID_SECS).contains(&secs),
            "bid time must be 10 s to 1 h"
        );
        require!(max.0 > 0, "max must be greater than zero");
        require!(
            bond.map(|b| b.0).unwrap_or(0) <= MAX_BOND.as_yoctonear(),
            "bond is at most 5 NEAR"
        );
        if by_worker {
            require!(
                max.0 < self.ledger.amount,
                "max is more than your pay: a contractor keeps part of it"
            );
        } else {
            require!(
                max.0 <= self.ledger.unallocated(),
                "max is more than the job can still hire"
            );
        }
        parse_hash32_hex(&spec_hash)
            .unwrap_or_else(|| env::panic_str("spec_hash must be 64 hex chars"));
        let closes_ns = env::block_timestamp() + secs * NS_PER_SEC;
        require!(
            closes_ns < self.window_end_ns,
            "bidding must close before the window"
        );
        let reveal_ns = if sealed == Some(true) {
            let r = closes_ns + secs * NS_PER_SEC;
            require!(
                r < self.window_end_ns,
                "the reveal must close before the window"
            );
            r
        } else {
            0
        };
        self.bid_slots.push(BidSlot {
            spec_hash: spec_hash.clone(),
            max,
            closes_ns: U64(closes_ns),
            bids: Vec::new(),
            awarded: None,
            bond: bond.unwrap_or(U128(0)),
            reveal_ns: U64(reveal_ns),
        });
        let slot = (self.bid_slots.len() - 1) as u32;
        emit(
            "bids_open",
            json!({ "slot": slot, "spec_hash": spec_hash, "max": max, "closes_ns": U64(closes_ns) }),
        );
        slot
    }

    /// Any account but the manager and the job: offer to do slot `slot` for
    /// `price` (at most the slot's max). A second bid replaces the first.
    /// A new bid attaches `BID_NEAR` to pay for its own storage (kept by the
    /// job, so fake bidders cannot fill the job's storage for free).
    #[payable]
    pub fn bid(&mut self, slot: u32, price: U128) {
        self.place_bid(slot, price, None)
    }

    /// Sealed slot, while bidding is open: commit to a price without showing
    /// it. `commit` = hex sha256 of `"<price>:<salt>:<your account>"`. A new
    /// sealed bid attaches `SEALED_BID_NEAR` (its commit takes more storage)
    /// plus the slot's bond. Reveal it with `reveal` after bidding closes.
    #[payable]
    pub fn bid_sealed(&mut self, slot: u32, commit: String) {
        parse_hash32_hex(&commit).unwrap_or_else(|| env::panic_str("commit must be 64 hex chars"));
        self.place_bid(slot, U128(0), Some(commit))
    }

    /// Sealed slot, after bidding closes and before `reveal_ns`: show the
    /// committed price. An unrevealed bid cannot win, and its bond goes to
    /// the buyer at the award.
    pub fn reveal(&mut self, slot: u32, price: U128, salt: String) {
        let me = env::predecessor_account_id();
        let s = self
            .bid_slots
            .get_mut(slot as usize)
            .unwrap_or_else(|| env::panic_str("no such bid slot"));
        require!(s.reveal_ns.0 > 0, "this slot is not sealed");
        let now = env::block_timestamp();
        require!(
            now >= s.closes_ns.0,
            "bidding is still open: reveal after it closes"
        );
        require!(now < s.reveal_ns.0, "the reveal is closed");
        require!(price.0 > 0 && price.0 <= s.max.0, "price must be 1..=max");
        let b = s
            .bids
            .iter_mut()
            .find(|b| b.worker == me)
            .unwrap_or_else(|| env::panic_str("you did not bid on this slot"));
        let c = b
            .commit
            .clone()
            .unwrap_or_else(|| env::panic_str("already revealed"));
        let h = hex_of(&env::sha256_array(
            format!("{}:{}:{}", price.0, salt, me).as_bytes(),
        ));
        require!(h == c, "price and salt do not match the commit");
        b.price = price;
        b.commit = None;
        emit(
            "reveal",
            json!({ "slot": slot, "worker": me, "price": price }),
        );
    }

    fn place_bid(&mut self, slot: u32, price: U128, commit: Option<String>) {
        require!(
            self.status == Status::Open && self.window_open(),
            "the job no longer takes bids (approved, fired, delivered or closed)"
        );
        let me = env::predecessor_account_id();
        require!(me != self.manager, "the manager cannot bid");
        require!(
            self.worker.as_ref() != Some(&me),
            "the job's worker cannot bid on its own job"
        );
        require!(me != env::current_account_id(), "the job cannot bid");
        let s = self
            .bid_slots
            .get_mut(slot as usize)
            .unwrap_or_else(|| env::panic_str("no such bid slot"));
        require!(s.awarded.is_none(), "cannot bid on the awarded slot");
        require!(env::block_timestamp() < s.closes_ns.0, "bidding is closed");
        let sealed = s.reveal_ns.0 > 0;
        require!(
            sealed == commit.is_some(),
            if sealed {
                "this slot is sealed: use bid_sealed"
            } else {
                "this slot is open: use bid"
            }
        );
        if !sealed {
            require!(price.0 > 0, "price must be greater than zero");
            require!(price.0 <= s.max.0, "bid is above the slot's max");
        }
        let attached = env::attached_deposit().as_yoctonear();
        match s.bids.iter_mut().find(|b| b.worker == me) {
            Some(b) => {
                // Storage is already paid; anything attached joins the bond
                // (it comes back with it) instead of being kept.
                b.price = price;
                b.commit = commit.clone();
                b.bond = U128(b.bond.0 + attached);
            }
            None => {
                require!(s.bids.len() < MAX_BIDS, "too many bids on this slot");
                let fee = if sealed { SEALED_BID_NEAR } else { BID_NEAR }.as_yoctonear();
                require!(
                    attached >= fee + s.bond.0,
                    match (sealed, s.bond.0 > 0) {
                        (false, false) => "attach 0.001 NEAR for the bid's storage",
                        (false, true) =>
                            "attach 0.001 NEAR for the bid's storage plus the slot's bond",
                        (true, false) => "attach 0.002 NEAR for the sealed bid's storage",
                        (true, true) =>
                            "attach 0.002 NEAR for the sealed bid's storage plus the slot's bond",
                    }
                );
                s.bids.push(Bid {
                    worker: me.clone(),
                    price,
                    bond: U128(attached - fee),
                    commit: commit.clone(),
                    offer: U128(0),
                    backer: None,
                    backed: U128(0),
                });
            }
        }
        if sealed {
            emit(
                "bid_sealed",
                json!({ "slot": slot, "worker": me, "commit": commit }),
            );
        } else {
            emit("bid", json!({ "slot": slot, "worker": me, "price": price }));
        }
    }

    /// Underwriting, step 1. A bidder offers a premium (yocto NEAR, at most
    /// half its own bond) to whoever backs its bid with more bond. The
    /// premium is paid only if the bidder wins and its work is paid.
    pub fn seek_backer(&mut self, slot: u32, premium: U128) {
        let me = env::predecessor_account_id();
        let s = self
            .bid_slots
            .get_mut(slot as usize)
            .unwrap_or_else(|| env::panic_str("no such bid slot"));
        require!(
            s.awarded.is_none() && env::block_timestamp() < s.closes_ns.0,
            "bidding is closed"
        );
        let b = s
            .bids
            .iter_mut()
            .find(|b| b.worker == me)
            .unwrap_or_else(|| env::panic_str("you did not bid on this slot"));
        require!(b.backer.is_none(), "this bid is already backed");
        require!(
            premium.0 <= b.bond.0 / 2,
            "the premium is at most half your bond"
        );
        b.offer = premium;
        emit(
            "backer_sought",
            json!({ "slot": slot, "worker": me, "premium": premium }),
        );
    }

    /// Underwriting, step 2. Anyone but the bidder and the buyer backs a bid
    /// that seeks a backer, attaching NEAR (at most MAX_BOND). Buyers see the
    /// bid's bond plus the backing. If the bidder wins and is paid, the
    /// backer gets its NEAR back plus the premium; if its work is junk, the
    /// buyer keeps both bonds; if it loses the auction, everyone is refunded.
    #[payable]
    pub fn back(&mut self, slot: u32, bidder: AccountId) {
        require!(
            self.status == Status::Open && self.window_open(),
            "the job no longer takes bids (approved, fired, delivered or closed)"
        );
        let me = env::predecessor_account_id();
        require!(me != bidder, "you cannot back your own bid");
        require!(me != self.manager, "the buyer cannot back a bid");
        let x = env::attached_deposit().as_yoctonear();
        require!(
            x > 0 && x <= MAX_BOND.as_yoctonear(),
            "attach between 1 yocto and 5 NEAR"
        );
        let s = self
            .bid_slots
            .get_mut(slot as usize)
            .unwrap_or_else(|| env::panic_str("no such bid slot"));
        require!(
            s.awarded.is_none() && env::block_timestamp() < s.closes_ns.0,
            "bidding is closed"
        );
        let b = s
            .bids
            .iter_mut()
            .find(|b| b.worker == bidder)
            .unwrap_or_else(|| env::panic_str("that account did not bid on this slot"));
        require!(b.offer.0 > 0, "that bid does not seek a backer");
        require!(b.backer.is_none(), "this bid is already backed");
        b.backer = Some(me.clone());
        b.backed = U128(x);
        emit(
            "backed",
            json!({ "slot": slot, "worker": bidder, "backer": me, "backed": U128(x), "premium": b.offer }),
        );
    }

    /// Manager only, after bidding closes: hire one bidder at its own bid.
    /// The manager may pick any bid, not only the lowest (it may weigh
    /// track records); the price is always the bidder's own.
    #[payable]
    pub fn award(&mut self, slot: u32, worker: AccountId, sub: Option<U128>) -> Promise {
        let by_worker = self.hiring_role("award");
        self.require_hire_resources(env::prepaid_gas());
        let s = self.awardable_slot(slot);
        let chosen = s
            .bids
            .iter()
            .find(|b| b.worker == worker)
            .unwrap_or_else(|| env::panic_str("that account did not bid on this slot"));
        require!(chosen.commit.is_none(), "that bid was never revealed");
        self.award_inner(
            slot,
            worker,
            sub,
            by_worker,
            env::predecessor_account_id(),
            env::attached_deposit(),
        )
    }

    /// The slot, if it can be awarded now: bidding (and any reveal) closed
    /// and nobody is awarded yet.
    fn awardable_slot(&self, slot: u32) -> BidSlot {
        let s = self
            .bid_slots
            .get(slot as usize)
            .cloned()
            .unwrap_or_else(|| env::panic_str("no such bid slot"));
        require!(s.awarded.is_none(), "slot already awarded");
        require!(
            env::block_timestamp() >= s.closes_ns.0,
            "bidding is still open"
        );
        require!(
            env::block_timestamp() >= s.reveal_ns.0,
            "the reveal is still open"
        );
        require!(self.leaves_time_to_work(), TOO_LATE);
        s
    }

    /// Is there still `MIN_WORK_SECS` of this job's window for a winner to
    /// deliver in? Its sub-job gets what is left of the window. Without
    /// this a buyer could award in the last second and keep the bond of a
    /// winner that never had time to deliver.
    fn leaves_time_to_work(&self) -> bool {
        self.window_end_ns.saturating_sub(env::block_timestamp()) >= MIN_WORK_SECS * NS_PER_SEC
    }

    /// Manager only, after bidding closes: the award rule runs on-chain. The
    /// job reads every revealed bidder's track record from the registry
    /// (the factory), then hires the bid with the lowest score under
    /// `ht_core::pick`: the bid, raised by `record_weight` times the share of
    /// money the record shows unpaid (150 = x2.5 if all of it), and for a
    /// bidder with no record, by `newcomer_pct` divided by how many times the
    /// slot's bond the bid carries (its bond plus its backing). Ties go to
    /// more paid money, then the earlier bid. `skip` names bidders the buyer
    /// cannot use (it is in the `award_best_asked` event). `min_paid`: paid
    /// money below it is no record, so a cent washed through a job a
    /// bidder paid itself does not end its newcomer premium; any unpaid
    /// money still counts. The 0.035 NEAR attached is for the child
    /// account; it comes back if no award is made. Any failed record read
    /// fails closed: no award, the slot stays open to retry or to a hand
    /// `award`. Like `award`, refused in the last `MIN_WORK_SECS` of the
    /// window. `award` stays for a buyer that picks by hand.
    ///
    /// `payers` (optional, at most 16): whose pay counts. When given, each
    /// bidder's record is read with `get_record_from` and its paid money
    /// (and the payer count in the event) comes only from those managers, so
    /// a record washed through accounts the buyer does not list is no
    /// record. Unpaid money still counts whoever it came from. `min_paid`
    /// then applies to the listed money. The list is in both events. Left
    /// out, nothing changes.
    #[payable]
    #[allow(clippy::too_many_arguments)]
    pub fn award_best(
        &mut self,
        slot: u32,
        record_weight: u32,
        newcomer_pct: u32,
        sub: Option<U128>,
        skip: Option<Vec<AccountId>>,
        min_paid: Option<U128>,
        payers: Option<Vec<AccountId>>,
    ) -> Promise {
        let by_worker = self.hiring_role("award");
        require!(
            payers.as_ref().map_or(0, |p| p.len()) <= MAX_LISTED_PAYERS,
            "list at most 16 payers"
        );
        let s = self.awardable_slot(slot);
        let skip = skip.unwrap_or_default();
        let workers: Vec<AccountId> = s
            .bids
            .iter()
            .filter(|b| b.commit.is_none() && !skip.contains(&b.worker))
            .map(|b| b.worker.clone())
            .collect();
        require!(!workers.is_empty(), "no revealed bid can be awarded");
        let mut asked = json!({ "slot": slot, "record_weight": record_weight, "newcomer_pct": newcomer_pct,
            "min_paid": min_paid, "skip": skip, "reading": workers });
        if let Some(p) = &payers {
            asked["payers"] = json!(p);
        }
        emit("award_best_asked", asked);
        require!(
            env::prepaid_gas().as_gas()
                >= GAS_HIRE_MIN.as_gas()
                    + GAS_AWARD_OWN.as_gas()
                    + GAS_GET_RECORD.as_gas() * workers.len() as u64,
            "not enough gas for award_best (attach 300 Tgas)"
        );
        self.require_hire_resources(GAS_HIRE_MIN);
        require!(self.status == Status::Open, "job is not open");
        require!(self.window_open(), "window is closed");
        let me = env::current_account_id();
        // With a payer list the factory filters the record itself, so the
        // reply stays one small record however long the breakdown is.
        let read = |w: &AccountId| {
            let (method, args) = match &payers {
                Some(p) => ("get_record_from", json!({ "account_id": w, "payers": p })),
                None => ("get_record", json!({ "account_id": w })),
            };
            Promise::new(self.registry.clone()).function_call(
                method,
                args.to_string().into_bytes(),
                NO_DEPOSIT,
                GAS_GET_RECORD,
            )
        };
        let mut reads = read(&workers[0]);
        for w in &workers[1..] {
            reads = reads.and(read(w));
        }
        reads.then(Self::ext(me).with_static_gas(GAS_HIRE_MIN).on_award_best(
            slot,
            workers,
            record_weight,
            newcomer_pct,
            sub,
            by_worker,
            env::predecessor_account_id(),
            U128(env::attached_deposit().as_yoctonear()),
            min_paid,
            payers,
        ))
    }

    /// After the record reads. Fails closed: if any read failed, or the slot
    /// was awarded meanwhile (by `award` or another `award_best`), or the job
    /// can no longer hire, nothing is awarded and the deposit goes back to
    /// the buyer. Otherwise the winner is hired through the same path as
    /// `award`.
    #[private]
    #[allow(clippy::too_many_arguments)]
    pub fn on_award_best(
        &mut self,
        slot: u32,
        workers: Vec<AccountId>,
        record_weight: u32,
        newcomer_pct: u32,
        sub: Option<U128>,
        by_worker: bool,
        buyer: AccountId,
        deposit: U128,
        min_paid: Option<U128>,
        payers: Option<Vec<AccountId>>,
    ) -> PromiseOrValue<bool> {
        assert_self();
        let refuse = |why: &str| -> PromiseOrValue<bool> {
            emit("award_best_failed", json!({ "slot": slot, "reason": why }));
            if deposit.0 > 0 {
                Promise::new(buyer.clone())
                    .transfer(NearToken::from_yoctonear(deposit.0))
                    .detach();
            }
            PromiseOrValue::Value(false)
        };
        if env::promise_results_count() != workers.len() as u64 {
            return refuse("record reads are missing");
        }
        let mut records = Vec::with_capacity(workers.len());
        for i in 0..workers.len() {
            match env::promise_result_checked(i as u64, MAX_RECORD_LEN) {
                Ok(bytes) => match serde_json::from_slice::<RecordView>(&bytes) {
                    Ok(r) => records.push(r),
                    Err(_) => return refuse("a record could not be read"),
                },
                Err(_) => return refuse("a record read failed"),
            }
        }
        let Some(s) = self.bid_slots.get(slot as usize).cloned() else {
            return refuse("no such bid slot");
        };
        if s.awarded.is_some() {
            return refuse("slot already awarded");
        }
        if self.status != Status::Open || !self.window_open() {
            return refuse("the job no longer hires");
        }
        if !self.leaves_time_to_work() {
            return refuse(TOO_LATE);
        }
        if self.children.len() >= MAX_CHILDREN {
            return refuse("too many children");
        }
        // `hire_inner` would panic on this, with the deposit left here.
        if format!("t{}.{}", self.children.len() + 1, env::current_account_id())
            .parse::<AccountId>()
            .is_err()
        {
            return refuse("the child account id would be too long");
        }
        if by_worker && (self.note_sold || self.note_pending) {
            return refuse("the note is sold: the pay is no longer yours to spend");
        }
        let floor = min_paid.map(|x| x.0).unwrap_or(0);
        // Every candidate is the revealed bid, with its on-chain bond and backing.
        let mut cands: Vec<ht_core::pick::Cand> = vec![];
        let mut inputs = vec![];
        for (w, rec) in workers.iter().zip(&records) {
            let Some(idx) = s
                .bids
                .iter()
                .position(|b| &b.worker == w && b.commit.is_none())
            else {
                return refuse("a bidder is gone from the slot");
            };
            let b = &s.bids[idx];
            // Paid money under the buyer's floor is no record (a cent
            // washed through a job you paid yourself is not a history).
            let paid_value = if rec.paid_value.0 < floor {
                0
            } else {
                rec.paid_value.0
            };
            let (jobs, paid) = ht_core::pick::weigh(paid_value, rec.unpaid_value.0);
            let v = ht_core::pick::vouch(b.bond.0, b.backed.0, s.bond.0);
            let c = (idx, b.price.0, jobs, paid, v);
            let mut input = json!({ "worker": w, "bid": b.price, "jobs": jobs, "paid": paid, "vouch": v.to_string(),
                "score": ht_core::pick::score(&c, newcomer_pct as u128, record_weight as u128).to_string() });
            if payers.is_some() {
                // What the pick read: the listed managers' pay and their count.
                input["listed_paid_value"] = json!(rec.paid_value);
                input["listed_payers"] = json!(rec.payers);
            }
            inputs.push(input);
            cands.push(c);
        }
        let Some(win) = ht_core::pick::pick(&cands, newcomer_pct as u128, record_weight as u128)
        else {
            return refuse("no bid to award");
        };
        let chosen = s.bids[win].clone();
        // Would the hire fail on money? Then say so here and refund, instead
        // of panicking with the deposit stranded.
        let total = chosen.price.0.saturating_add(sub.map(|x| x.0).unwrap_or(0));
        let mut ledger = self.ledger.clone();
        let funds = if by_worker {
            if sub.is_some() {
                Err("a contractor hires without a sub-budget")
            } else {
                ledger
                    .carve(total)
                    .and_then(|_| ledger.allocate(total))
                    .map_err(|_| "the job cannot fund this hire")
            }
        } else {
            ledger
                .allocate(total)
                .map_err(|_| "the job cannot fund this hire")
        };
        if let Err(why) = funds {
            return refuse(why);
        }
        let mut won = json!({ "slot": slot, "rule": "lowest bid x (1 + record_weight x unpaid share) x newcomer premium / vouch",
            "record_weight": record_weight, "newcomer_pct": newcomer_pct, "min_paid": U128(floor),
            "winner": chosen.worker, "price": chosen.price, "inputs": inputs });
        if let Some(p) = &payers {
            won["payers"] = json!(p);
        }
        emit("award_best", won);
        let p = self.award_inner(
            slot,
            chosen.worker,
            sub,
            by_worker,
            buyer,
            NearToken::from_yoctonear(deposit.0),
        );
        PromiseOrValue::Promise(p)
    }

    /// The award itself, for a bidder already chosen and checked: mark the
    /// slot, refund the losers, hire the winner at its bid. `award` and
    /// `on_award_best` both end here.
    fn award_inner(
        &mut self,
        slot: u32,
        worker: AccountId,
        sub: Option<U128>,
        by_worker: bool,
        buyer: AccountId,
        deposit: NearToken,
    ) -> Promise {
        let s = self.bid_slots[slot as usize].clone();
        let price = s
            .bids
            .iter()
            .find(|b| b.worker == worker)
            .map(|b| b.price)
            .unwrap_or_else(|| env::panic_str("that account did not bid on this slot"));
        self.bid_slots[slot as usize].awarded = Some(worker.clone());
        emit(
            "award",
            json!({ "slot": slot, "worker": worker, "price": price }),
        );
        // Losers get their bond back; a sealed bid never revealed loses it
        // to the buyer (no free option to hide and walk away).
        let mut winner_bond = 0;
        let mut winner_backing = (None, 0u128, 0u128);
        for b in self.bid_slots[slot as usize].bids.iter_mut() {
            let x = std::mem::replace(&mut b.bond, U128(0)).0;
            let backed = std::mem::replace(&mut b.backed, U128(0)).0;
            if b.worker == worker {
                winner_bond = x;
                winner_backing = (b.backer.clone(), backed, b.offer.0.min(x));
                continue;
            }
            if let (Some(k), true) = (b.backer.clone(), backed > 0) {
                Promise::new(k)
                    .transfer(NearToken::from_yoctonear(backed))
                    .detach();
            }
            if x > 0 {
                let to = if b.commit.is_some() {
                    buyer.clone()
                } else {
                    b.worker.clone()
                };
                Promise::new(to)
                    .transfer(NearToken::from_yoctonear(x))
                    .detach();
            }
        }
        let p = self.hire_inner(worker, price, s.spec_hash, sub, by_worker, deposit, true);
        let last = self.children.len() - 1;
        self.children[last].bond = U128(winner_bond);
        self.children[last].backer = winner_backing.0;
        self.children[last].backed = U128(winner_backing.1);
        self.children[last].premium = U128(winner_backing.2);
        p
    }

    /// One bid slot, or null.
    pub fn get_bids(&self, slot: u32) -> Option<BidSlot> {
        self.bid_slots.get(slot as usize).cloned()
    }

    #[allow(clippy::too_many_arguments)]
    fn hire_inner(
        &mut self,
        worker: AccountId,
        amount: U128,
        spec_hash: String,
        sub: Option<U128>,
        by_worker: bool,
        deposit: NearToken,
        bid: bool,
    ) -> Promise {
        require!(self.status == Status::Open, "job is not open");
        require!(self.window_open(), "window is closed");
        require!(self.children.len() < MAX_CHILDREN, "too many children");
        require!(amount.0 > 0, "amount must be greater than zero");
        require!(
            deposit >= HIRE_MIN_NEAR,
            "attach at least 0.035 NEAR for the child account's storage"
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

        // State before promises. A contractor pays out of its own pay.
        if by_worker {
            require!(sub.is_none(), "a contractor hires without a sub-budget");
            if let Err(e) = self.ledger.carve(total) {
                fail(e);
            }
        }
        if let Err(e) = self.ledger.allocate(total) {
            fail(e);
        }
        self.children.push(Child {
            account: child.clone(),
            worker: worker.clone(),
            total: U128(total),
            state: ChildState::Creating,
            since_ns: env::block_timestamp(),
            by_worker,
            bond: U128(0),
            backer: None,
            backed: U128(0),
            premium: U128(0),
        });
        self.children_pending += 1;

        let remaining_secs = (self.window_end_ns - env::block_timestamp()) / NS_PER_SEC;
        let args = json!({
            "client": null,
            "manager": if by_worker { self.worker.clone().unwrap() } else { self.manager.clone() },
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
            "registry": self.registry,
            "bid": bid,
        });
        // The job keeps HIRE_KEEP_NEAR (child's token registration + its own
        // child record); the rest is the child's storage.
        let child_near = deposit.saturating_sub(HIRE_KEEP_NEAR);
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
            if self.children[i].by_worker {
                if let Err(e) = self.ledger.uncarve(total - used) {
                    fail(e);
                }
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
        if self.children[i].by_worker {
            if let Err(e) = self.ledger.uncarve(total) {
                fail(e);
            }
        }
        self.children[i].state = ChildState::Failed;
        self.children_pending -= 1;
        // A hire that never happened earns the backer no premium.
        self.children[i].premium = U128(0);
        self.release_bond(i, true);
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
        if self.checker.is_some() || self.judge.is_some() {
            require!(
                resp.is_some(),
                "a checker or judge is pinned: send the response bytes (resp) so it can read them"
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
        // A panel with no answer, or whose answers disagree, abstains: it
        // never counts against the worker, so a buyer cannot win a dispute
        // by leaving its panels empty.
        let junk = votes
            .iter()
            .filter(|v| matches!(v, Some(s) if *s < PASS_SCORE))
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

    /// Was this job's worker paid (by settle or by a note sale)?
    fn worker_paid(&self) -> bool {
        self.note_sold
            || (self.ledger.payee_paid && self.payee.is_some() && self.payee == self.worker)
    }

    /// Send child `i`'s bond back to its worker (`to_worker`), or to the
    /// buyer that hired it (its work was not paid).
    fn release_bond(&mut self, i: usize, to_worker: bool) {
        let mut x = std::mem::replace(&mut self.children[i].bond, U128(0)).0;
        let backed = std::mem::replace(&mut self.children[i].backed, U128(0)).0;
        let premium = std::mem::replace(&mut self.children[i].premium, U128(0)).0;
        if let (Some(k), true) = (self.children[i].backer.clone(), backed > 0) {
            if to_worker {
                // Paid work: the backer gets its NEAR back plus the premium,
                // out of the worker's own bond.
                let pr = premium.min(x);
                x -= pr;
                emit(
                    "backer_paid",
                    json!({ "child": self.children[i].account, "backer": k, "backed": U128(backed), "premium": U128(pr) }),
                );
                Promise::new(k)
                    .transfer(NearToken::from_yoctonear(backed + pr))
                    .detach();
            } else {
                // Junk: the backer's NEAR goes to the buyer with the bond.
                x += backed;
            }
        }
        if x == 0 {
            return;
        }
        let to = if to_worker {
            self.children[i].worker.clone()
        } else if self.children[i].by_worker {
            self.worker.clone().unwrap_or_else(|| self.manager.clone())
        } else {
            self.manager.clone()
        };
        emit(
            if to_worker {
                "bond_returned"
            } else {
                "bond_kept"
            },
            json!({ "child": self.children[i].account, "to": to, "bond": U128(x) }),
        );
        Promise::new(to)
            .transfer(NearToken::from_yoctonear(x))
            .detach();
    }

    /// A bidder takes its bond back from a slot that was never awarded,
    /// once bidding is closed and the job's window has ended.
    pub fn reclaim_bond(&mut self, slot: u32) {
        let me = env::predecessor_account_id();
        require!(
            env::block_timestamp() >= self.window_end_ns || self.approved,
            "the window is still open"
        );
        let s = self
            .bid_slots
            .get_mut(slot as usize)
            .unwrap_or_else(|| env::panic_str("no such bid slot"));
        require!(
            s.awarded.is_none(),
            "slot was awarded: losers were refunded then"
        );
        // A backer takes back what it backed the same way.
        if let Some(b) = s.bids.iter_mut().find(|b| b.backer.as_ref() == Some(&me)) {
            let x = std::mem::replace(&mut b.backed, U128(0)).0;
            if x > 0 {
                Promise::new(me)
                    .transfer(NearToken::from_yoctonear(x))
                    .detach();
                return;
            }
        }
        let b = s
            .bids
            .iter_mut()
            .find(|b| b.worker == me)
            .unwrap_or_else(|| env::panic_str("you did not bid on this slot"));
        let x = std::mem::replace(&mut b.bond, U128(0)).0;
        require!(x > 0, "no bond to reclaim");
        Promise::new(me)
            .transfer(NearToken::from_yoctonear(x))
            .detach();
    }

    /// Bonds still held for slots that were never awarded.
    fn refund_open_bonds(&mut self) {
        for s in self.bid_slots.iter_mut().filter(|s| s.awarded.is_none()) {
            for b in s.bids.iter_mut() {
                let x = std::mem::replace(&mut b.bond, U128(0)).0;
                if x > 0 {
                    Promise::new(b.worker.clone())
                        .transfer(NearToken::from_yoctonear(x))
                        .detach();
                }
                let k = std::mem::replace(&mut b.backed, U128(0)).0;
                if let (Some(to), true) = (b.backer.clone(), k > 0) {
                    Promise::new(to)
                        .transfer(NearToken::from_yoctonear(k))
                        .detach();
                }
            }
        }
    }

    /// The worker hired sub-agents that have not closed yet.
    pub fn has_open_worker_hires(&self) -> bool {
        self.children
            .iter()
            .any(|c| c.by_worker && c.state != ChildState::Closed && c.state != ChildState::Failed)
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
            // A contractor's pay is not final until its sub-jobs close
            // (refunds come back to it), so its note waits for them.
            || self.has_open_worker_hires()
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

    /// Sub-job manager only, after the signed check failed the delivery and
    /// before any redelivery: end the job now. The worker is not paid (the
    /// status is Open, so `settle` sends everything back to the parent) and
    /// its bond goes to the buyer, who can hire the runner-up in the same
    /// window instead of waiting the window out on junk.
    pub fn fire(&mut self) {
        require!(
            !self.root,
            "a root job cannot fire: it has no buyer above it"
        );
        require!(
            env::predecessor_account_id() == self.manager,
            "only the sub-job's manager can fire"
        );
        require!(!self.approved, "already approved");
        require!(!self.check_pending, "check pending: wait for its answer");
        require!(
            self.status == Status::Open && self.failed_checks > 0,
            "fire only after a failed check, before a redelivery"
        );
        self.approved = true;
        self.window_end_ns = self.window_end_ns.min(env::block_timestamp());
        emit(
            "fired",
            json!({ "worker": self.worker, "failed_checks": self.failed_checks }),
        );
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
        // Bonds of slots never awarded go back before the account can go.
        self.refund_open_bonds();
        // Paid: checked work; or, without a checker, any delivery; or an
        // unscored delivery the manager approved. Unscored work is never
        // paid by default (fail closed).
        let pay_payee = match prev {
            Status::Checked => true,
            Status::Delivered => {
                (self.checker.is_none() && self.judge.is_none()) || (!self.root && self.approved)
            }
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
        let prev = self.status_before_settle;
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
        if let Some(worker) = self.worker.clone() {
            // The worker's track record (fire-and-forget; evidence, not proof).
            let paid = self.worker_paid();
            Promise::new(self.registry.clone())
                .function_call(
                    "record",
                    json!({
                        "worker": worker,
                        "delivered": self.seq > 0,
                        "checked": prev == Some(Status::Checked),
                        "junk": prev == Some(Status::Refunded),
                        "paid": paid,
                        "quality": self.quality,
                        // Unpaid money marks a record only if the worker
                        // took the job on: it won it with its own bid,
                        // delivered, or hired with the pay. A `hire` asks
                        // nobody, so a hire left to lapse must not stain
                        // anyone's record.
                        "amount": U128(if paid
                            || self.by_bid
                            || self.seq > 0
                            || self.children.iter().any(|c| c.by_worker)
                        {
                            self.ledger.amount
                        } else {
                            0
                        }),
                        "payer": self.manager,
                    })
                    .to_string()
                    .into_bytes(),
                    NO_DEPOSIT,
                    GAS_RECORD,
                )
                .detach();
        }
        // A callback may not return a joint promise (the runtime refuses it,
        // found on the sandbox), so the parent notice is detached.
        Promise::new(self.parent.clone())
            .function_call(
                "child_closed",
                json!({ "returned": U128(self.returned_to_parent), "paid": self.worker_paid() })
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
    pub fn child_closed(&mut self, returned: U128, paid: Option<bool>) {
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
        let allocation = self.children[i].total.0;
        let r = if self.children[i].by_worker {
            self.ledger.restore(returned.0, allocation)
        } else {
            self.ledger.credit(returned.0, allocation)
        };
        if let Err(e) = r {
            fail(e);
        }
        // No outcome reported (older code): the bond goes back.
        self.release_bond(i, paid.unwrap_or(true));
        emit(
            "child_closed",
            json!({ "child": caller, "returned": returned, "paid": paid }),
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
            registry: self.registry.clone(),
            by_bid: self.by_bid,
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
