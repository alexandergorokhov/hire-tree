//! Hire Tree factory (`hiretree.<net>`). A client posts and funds a job in
//! one `ft_transfer_call(factory, budget, msg)`; the factory creates
//! `job-<n>.<factory>` running the global job contract by code hash, inits
//! it with the pinned signer and checker, registers it on the token and
//! forwards the budget. If anything fails, the whole budget is returned as
//! unused, so the token refunds the client.

use ht_core::binding::{parse_addr20_hex, parse_hash32_hex};
use near_sdk::json_types::{Base58CryptoHash, U128, U64};
use near_sdk::serde::Deserialize;
use near_sdk::serde_json::{self, json};
use near_sdk::store::{LookupMap, LookupSet};
use near_sdk::{
    env, near, require, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseOrValue,
};

/// NEAR the factory gives each root job (its storage; refunded at delete).
pub const ROOT_NEAR: NearToken = NearToken::from_millinear(50);
const TOKEN_STORAGE_NEAR: NearToken = NearToken::from_micronear(1_250);
const GAS_INIT: Gas = Gas::from_tgas(15);
const GAS_STORAGE_DEPOSIT: Gas = Gas::from_tgas(10);
const GAS_FT_TRANSFER_CALL: Gas = Gas::from_tgas(40);
const GAS_ON_POSTED: Gas = Gas::from_tgas(15);
const GAS_FT_TRANSFER: Gas = Gas::from_tgas(10);
/// Least gas `ft_on_transfer` must have left to post a job.
pub const GAS_POST_MIN: Gas = Gas::from_tgas(110);
/// The shortest window a post may ask for: the fund call lands a few blocks
/// after the job is created, so a shorter window could close before it.
pub const MIN_WINDOW_SECS: u64 = 60;
pub const MAX_WINDOW_SECS: u64 = 30 * 24 * 3600;
/// NEAR a client prepays per post (`prepay_post`): the root's storage plus
/// its token registration, with headroom. Without it, anyone could make the
/// factory lock 51 mNEAR per post for the cost of gas.
pub const POST_NEAR: NearToken = NearToken::from_millinear(60);
/// Smallest budget a post accepts (1 fUSD at 6 decimals).
pub const MIN_BUDGET: u128 = 1_000_000;
/// What a caller attaches for a storage entry it makes the factory keep (a
/// cancelled nonce, a new credit): about 100 bytes at 1e19 yocto per byte.
pub const STORAGE_NEAR: NearToken = NearToken::from_millinear(1);

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde", deny_unknown_fields)]
struct PostMsg {
    manager: AccountId,
    spec_hash: String,
    window_secs: u64,
    treasury: Option<AccountId>,
    /// The lead's fee cap. At settle the lead earns it pro rata to the root's
    /// parts that passed, over at least half the work budget; the rest goes
    /// back to the client.
    lead_fee: Option<U128>,
}

/// A job intent, signed off-chain by its client (see `fill_intent`).
#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde", deny_unknown_fields)]
struct Intent {
    /// This factory: an intent cannot be replayed on another one.
    factory: AccountId,
    client: AccountId,
    budget: U128,
    spec_hash: String,
    window_secs: u64,
    max_lead_fee: U128,
    starts_ns: U64,
    ends_ns: U64,
    nonce: U128,
    /// The only accounts that may fill this intent. Left out: anyone may.
    /// The filler becomes the job's manager and spends its budget, so an
    /// open intent trusts whoever fills first.
    #[serde(default)]
    leads: Option<Vec<AccountId>>,
}

/// The bytes a client signs: a domain tag, then the intent JSON exactly.
pub fn intent_message(intent: &str) -> Vec<u8> {
    let mut m = b"hiretree-intent:".to_vec();
    m.extend_from_slice(intent.as_bytes());
    m
}

/// The lead fee of a Dutch auction: 0 at `start`, rising linearly to `max`
/// at `end` (and `max` after it).
pub fn dutch_fee(max: u128, start: u64, end: u64, now: u64) -> u128 {
    let (now, start, end) = (now as u128, start as u128, end as u128);
    if now <= start || end <= start {
        return 0;
    }
    if now >= end {
        return max;
    }
    // max * elapsed / span without overflow: split max like platform_fee.
    let (elapsed, span) = (now - start, end - start);
    max / span * elapsed + max % span * elapsed / span
}

fn bs58_decode(s: &str) -> Option<Vec<u8>> {
    const A: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut out: Vec<u8> = vec![];
    for c in s.bytes() {
        let mut carry = A.iter().position(|&x| x == c)? as u32;
        for b in out.iter_mut().rev() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            out.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let zeros = s.bytes().take_while(|&c| c == b'1').count();
    let mut v = vec![0u8; zeros];
    v.extend(out);
    Some(v)
}

fn bs58_encode(bytes: &[u8]) -> String {
    const A: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut digits: Vec<u8> = vec![];
    for &b in bytes {
        let mut carry = b as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = bytes.iter().take_while(|&&b| b == 0).count();
    let mut s: String = "1".repeat(zeros);
    s.extend(digits.iter().rev().map(|&d| A[d as usize] as char));
    s
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use near_sdk::base64::Engine;
    near_sdk::base64::engine::general_purpose::STANDARD
        .decode(s)
        .ok()
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostState {
    Pending,
    Posted,
    Failed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Posted {
    pub job: AccountId,
    pub client: AccountId,
    pub manager: AccountId,
    pub budget: U128,
    pub state: PostState,
}

/// Most payers the factory keeps a paid-money sum for, per worker. Storage
/// is bounded: 16 entries are at most ~1.4 KB (about 0.014 NEAR) per worker.
pub const MAX_PAYER_ENTRIES: usize = 16;
/// Most payers a buyer may list in `get_record_from` (and `award_best`).
pub const MAX_LISTED_PAYERS: usize = 16;
/// A payment under this (1 fUSD) never gets its own entry: it counts in the
/// worker's total and in `other`, so dust from many accounts cannot fill
/// the list.
pub const MIN_PAYER_ENTRY: u128 = MIN_BUDGET;

/// One payer's sum in a worker's breakdown.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayerPaid {
    pub payer: AccountId,
    pub paid: U128,
}

/// What each manager paid a worker, bounded. The sums of `entries` and
/// `other` always equal the record's `paid_value`. `other` holds pay with no
/// entry: dust, a payer that lost its place (see [`PayerBook::credit`]), pay
/// from the worker itself, and pay reported without a payer.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PayerBook {
    pub entries: Vec<PayerPaid>,
    pub other: U128,
}

impl PayerBook {
    /// Add `v` paid by `payer` (None: nobody countable). A payer that has an
    /// entry adds to it. A new payer needs `v >= MIN_PAYER_ENTRY`; with the
    /// list full it must also pay strictly more than the smallest entry,
    /// which then moves to `other` (the oldest of equal smallest ones). So
    /// spraying small payments from many accounts never displaces anyone:
    /// each displacement costs more than the entry it removes, paid to the
    /// worker, and an entry only grows.
    pub fn credit(&mut self, payer: Option<&AccountId>, v: u128) {
        let Some(p) = payer else {
            self.other = U128(self.other.0.saturating_add(v));
            return;
        };
        if let Some(e) = self.entries.iter_mut().find(|e| &e.payer == p) {
            e.paid = U128(e.paid.0.saturating_add(v));
            return;
        }
        if v < MIN_PAYER_ENTRY {
            self.other = U128(self.other.0.saturating_add(v));
            return;
        }
        let new = PayerPaid {
            payer: p.clone(),
            paid: U128(v),
        };
        if self.entries.len() < MAX_PAYER_ENTRIES {
            self.entries.push(new);
            return;
        }
        let (i, least) = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(i, e)| (e.paid.0, *i))
            .map(|(i, e)| (i, e.paid.0))
            .expect("a full list is not empty");
        if v > least {
            let out = std::mem::replace(&mut self.entries[i], new);
            self.other = U128(self.other.0.saturating_add(out.paid.0));
        } else {
            self.other = U128(self.other.0.saturating_add(v));
        }
    }

    /// Paid money and payer count from the listed payers only.
    pub fn from_listed(&self, listed: &[AccountId]) -> (u128, u32) {
        self.entries
            .iter()
            .filter(|e| listed.contains(&e.payer))
            .fold((0u128, 0u32), |(m, n), e| {
                (m.saturating_add(e.paid.0), n + (e.paid.0 > 0) as u32)
            })
    }
}

/// A worker's track record, reported by the jobs this factory's tree made.
/// Evidence, not proof: a lead can hire itself to build one.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub jobs: u32,
    pub delivered: u32,
    pub checked: u32,
    pub junk: u32,
    pub paid: u32,
    /// Sum and count of check scores, for an average.
    pub score_sum: u32,
    pub scored: u32,
    /// fUSD of pay the worker was paid, and of pay it lost (junk, fired):
    /// a record weighted by money, not by job count.
    #[serde(default)]
    pub paid_value: U128,
    #[serde(default)]
    pub unpaid_value: U128,
    /// How many different managers have paid this worker. A record built by
    /// hiring yourself, or one friend, shows 1.
    #[serde(default)]
    pub payers: u32,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Factory {
    owner: AccountId,
    token: AccountId,
    code_hash: [u8; 32],
    signer: String,
    checker: Option<AccountId>,
    /// Signed-check mode: the judge model new jobs pin (instead of a
    /// checker contract).
    judge: Option<String>,
    next: u32,
    jobs: LookupMap<u32, Posted>,
    /// Prepaid NEAR per client, in yocto.
    credits: LookupMap<AccountId, u128>,
    records: LookupMap<AccountId, Record>,
    /// Per worker: who paid it how much (bounded, see [`PayerBook`]).
    payer_books: LookupMap<AccountId, PayerBook>,
    /// Platform fee on each posted budget, in basis points (max 100 = 1%).
    fee_bps: u16,
    /// Fees kept so far and not yet withdrawn.
    fees: u128,
    /// fUSD each client keeps here for its signed job intents.
    deposits: LookupMap<AccountId, u128>,
    /// The ed25519 key each client signs its intents with.
    intent_keys: LookupMap<AccountId, [u8; 32]>,
    /// (client, nonce) of every intent filled or cancelled.
    used_nonces: LookupSet<(AccountId, u128)>,
    /// (worker, manager) pairs where that manager paid that worker.
    payers: LookupSet<(AccountId, AccountId)>,
    /// Judge rubric version new jobs pin (`ht_core::check::rubric`).
    rubric: u8,
}

/// The highest platform fee the owner may set: 1%.
pub const MAX_FEE_BPS: u16 = 100;

/// The platform fee on a budget, rounded down.
pub fn platform_fee(budget: u128, bps: u16) -> u128 {
    budget / 10_000 * bps as u128 + budget % 10_000 * bps as u128 / 10_000
}

fn emit(event: &str, data: serde_json::Value) {
    let line =
        json!({ "standard": "hiretree", "version": "0.1.0", "event": event, "data": [data] });
    env::log_str(&format!("EVENT_JSON:{line}"));
}

#[near]
impl Factory {
    #[init]
    pub fn new(
        owner: AccountId,
        token: AccountId,
        code_hash: Base58CryptoHash,
        signer: String,
        checker: Option<AccountId>,
    ) -> Self {
        require!(
            parse_addr20_hex(&signer).is_some(),
            "signer must be a 20-byte hex address"
        );
        Self {
            owner,
            token,
            code_hash: code_hash.into(),
            signer,
            checker,
            judge: None,
            next: 1,
            jobs: LookupMap::new(b"j"),
            credits: LookupMap::new(b"c"),
            records: LookupMap::new(b"r"),
            payer_books: LookupMap::new(b"b"),
            fee_bps: 0,
            fees: 0,
            deposits: LookupMap::new(b"d"),
            intent_keys: LookupMap::new(b"k"),
            used_nonces: LookupSet::new(b"u"),
            payers: LookupSet::new(b"p"),
            rubric: ht_core::check::RUBRIC_LATEST,
        }
    }

    /// Owner only. New jobs use the new values; existing jobs keep theirs.
    #[allow(clippy::too_many_arguments)]
    pub fn set_settings(
        &mut self,
        code_hash: Option<Base58CryptoHash>,
        signer: Option<String>,
        checker: Option<AccountId>,
        clear_checker: Option<bool>,
        judge: Option<String>,
        clear_judge: Option<bool>,
        fee_bps: Option<u16>,
        rubric: Option<u8>,
    ) {
        require!(env::predecessor_account_id() == self.owner, "owner only");
        if let Some(r) = rubric {
            require!(
                ht_core::check::rubric(r).is_some(),
                "unknown judge rubric version"
            );
            self.rubric = r;
        }
        if let Some(f) = fee_bps {
            require!(f <= MAX_FEE_BPS, "fee_bps is at most 100 (1%)");
            self.fee_bps = f;
        }
        if let Some(h) = code_hash {
            self.code_hash = h.into();
        }
        if let Some(s) = signer {
            require!(
                parse_addr20_hex(&s).is_some(),
                "signer must be a 20-byte hex address"
            );
            self.signer = s;
        }
        if checker.is_some() {
            self.checker = checker;
        }
        if clear_checker == Some(true) {
            self.checker = None;
        }
        if judge.is_some() {
            self.judge = judge;
        }
        if clear_judge == Some(true) {
            self.judge = None;
        }
        require!(
            !(self.checker.is_some() && self.judge.is_some()),
            "pin a checker contract or a judge model, not both"
        );
        emit(
            "settings",
            json!({ "signer": self.signer, "checker": self.checker, "judge": self.judge, "fee_bps": self.fee_bps, "rubric": self.rubric }),
        );
    }

    /// A client prepays NEAR for its posts: each post uses `POST_NEAR`
    /// (0.06 NEAR). The root's storage comes back to the factory when the
    /// root deletes; it is the posting fee.
    #[payable]
    pub fn prepay_post(&mut self, account_id: Option<AccountId>) -> U128 {
        let who = account_id.unwrap_or_else(env::predecessor_account_id);
        let add = env::attached_deposit().as_yoctonear();
        // A credit is a storage entry for any account id the caller names:
        // it must at least pay for itself.
        require!(
            add >= STORAGE_NEAR.as_yoctonear(),
            "prepay at least 0.001 NEAR"
        );
        let now = self.credits.get(&who).copied().unwrap_or(0) + add;
        self.credits.insert(who.clone(), now);
        emit("prepaid", json!({ "account": who, "credit": U128(now) }));
        U128(now)
    }

    pub fn get_credit(&self, account_id: AccountId) -> U128 {
        U128(self.credits.get(&account_id).copied().unwrap_or(0))
    }

    fn refund(&self, amount: U128, reason: &str) -> PromiseOrValue<U128> {
        emit("refund", json!({ "amount": amount, "reason": reason }));
        PromiseOrValue::Value(amount)
    }

    /// NEP-141 receiver. `msg` = a post (`{manager, spec_hash, window_secs,
    /// treasury?, lead_fee?}`): post and fund a job; anything not accepted is
    /// refunded. `msg` = `"deposit"`: keep the tokens as the sender's intent
    /// deposit, which a signed job intent spends (`fill_intent`).
    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        if env::predecessor_account_id() != self.token {
            return self.refund(amount, "wrong token");
        }
        if msg == "deposit" {
            let now = self.deposits.get(&sender_id).copied().unwrap_or(0) + amount.0;
            self.deposits.insert(sender_id.clone(), now);
            emit(
                "deposited",
                json!({ "account": sender_id, "deposit": U128(now) }),
            );
            return PromiseOrValue::Value(U128(0));
        }
        let Ok(m) = serde_json::from_str::<PostMsg>(&msg) else {
            return self.refund(
                amount,
                "msg must be {manager, spec_hash, window_secs, treasury?, lead_fee?}",
            );
        };
        if let Err(reason) = self.check_post(&sender_id, amount.0, &m) {
            return self.refund(amount, reason);
        }
        PromiseOrValue::Promise(self.launch(sender_id, amount.0, m, false))
    }

    /// Everything a post must satisfy before the factory spends anything.
    fn check_post(
        &self,
        client: &AccountId,
        amount: u128,
        m: &PostMsg,
    ) -> Result<(), &'static str> {
        if amount < MIN_BUDGET {
            return Err("budget below the minimum (1 fUSD)");
        }
        if parse_hash32_hex(&m.spec_hash).is_none() {
            return Err("spec_hash must be 64 hex chars");
        }
        let funded = amount - platform_fee(amount, self.fee_bps);
        if m.lead_fee.map(|f| f.0).unwrap_or(0) > funded {
            return Err("lead_fee is larger than the budget");
        }
        if m.window_secs < MIN_WINDOW_SECS || m.window_secs > MAX_WINDOW_SECS {
            return Err("window_secs out of range");
        }
        if env::prepaid_gas().saturating_sub(env::used_gas()) < GAS_POST_MIN {
            return Err("not enough gas to post (attach 200 Tgas)");
        }
        if self.credits.get(client).copied().unwrap_or(0) < POST_NEAR.as_yoctonear() {
            return Err("prepay 0.06 NEAR with prepay_post first");
        }
        Ok(())
    }

    /// Create, register and fund `job-<n>`; `on_posted` settles the money.
    /// `from_deposit`: the budget came from the client's intent deposit, so
    /// anything unused goes back to that deposit instead of to the token.
    fn launch(
        &mut self,
        client: AccountId,
        amount: u128,
        m: PostMsg,
        from_deposit: bool,
    ) -> Promise {
        let fee = platform_fee(amount, self.fee_bps);
        let funded = amount - fee;
        let lead_fee = m.lead_fee.map(|f| f.0).unwrap_or(0);
        let credit = self.credits.get(&client).copied().unwrap_or(0);
        self.credits
            .insert(client.clone(), credit - POST_NEAR.as_yoctonear());
        let n = self.next;
        self.next += 1;
        let job: AccountId = format!("job-{n}.{}", env::current_account_id())
            .parse()
            .unwrap_or_else(|_| env::panic_str("job account id too long"));
        self.jobs.insert(
            n,
            Posted {
                job: job.clone(),
                client: client.clone(),
                manager: m.manager.clone(),
                budget: U128(amount),
                state: PostState::Pending,
            },
        );
        let init = json!({
            "client": client, "manager": m.manager, "worker": null,
            "token": self.token, "amount": U128(lead_fee), "spec_hash": m.spec_hash,
            "window_secs": m.window_secs, "signer": self.signer,
            "code_hash": Base58CryptoHash::from(self.code_hash),
            "checker": self.checker, "treasury": m.treasury, "judge": self.judge,
            "rubric": self.rubric,
        });
        emit(
            "post_started",
            json!({ "n": n, "job": job, "client": client, "budget": U128(amount), "platform_fee": U128(fee) }),
        );
        let create = Promise::new(job.clone())
            .create_account()
            .transfer(ROOT_NEAR)
            .use_global_contract(self.code_hash)
            .function_call(
                "new",
                init.to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                GAS_INIT,
            );
        let register = Promise::new(self.token.clone()).function_call(
            "storage_deposit",
            json!({ "account_id": job, "registration_only": true })
                .to_string()
                .into_bytes(),
            TOKEN_STORAGE_NEAR,
            GAS_STORAGE_DEPOSIT,
        );
        // The job accepts root funding from its client or its parent (us).
        let fund = Promise::new(self.token.clone()).function_call(
            "ft_transfer_call",
            json!({ "receiver_id": job, "amount": U128(funded), "msg": "fund" })
                .to_string()
                .into_bytes(),
            NearToken::from_yoctonear(1),
            GAS_FT_TRANSFER_CALL,
        );
        create.then(register).then(fund).then(
            Self::ext(env::current_account_id())
                .with_static_gas(GAS_ON_POSTED)
                .on_posted(n, U128(amount), Some(U128(fee)), Some(from_deposit)),
        )
    }

    /// The client registers the ed25519 key it signs job intents with (base58
    /// or "ed25519:<base58>"). Replacing it voids intents signed with the old one.
    pub fn set_intent_key(&mut self, public_key: String) {
        let who = env::predecessor_account_id();
        let raw = public_key.strip_prefix("ed25519:").unwrap_or(&public_key);
        let bytes = bs58_decode(raw).unwrap_or_else(|| env::panic_str("public_key must be base58"));
        let key: [u8; 32] = bytes
            .try_into()
            .unwrap_or_else(|_| env::panic_str("public_key must be 32 bytes"));
        self.intent_keys.insert(who.clone(), key);
        emit(
            "intent_key",
            json!({ "account": who, "public_key": public_key }),
        );
    }

    /// Intent-based settlement. A client signs a job intent off-chain (no
    /// transaction): "get this spec done for `budget`; a lead may keep up to
    /// `max_lead_fee`". Any lead agent may fill it; the lead's fee rises from 0
    /// at `starts_ns` to `max_lead_fee` at `ends_ns` (a Dutch auction), so lead
    /// agents compete on how soon they fill. The fee a fill locks in is a
    /// cap: the job pays the lead only the part its paid parts earn. The first valid fill posts the
    /// job from the client's deposit with the caller as its manager; the
    /// nonce is then spent, so an intent fills once.
    pub fn fill_intent(
        &mut self,
        intent: String,
        signature: String,
        treasury: Option<AccountId>,
    ) -> Promise {
        let lead = env::predecessor_account_id();
        let i: Intent = serde_json::from_str(&intent)
            .unwrap_or_else(|_| env::panic_str("intent must be the signed intent JSON"));
        require!(
            i.factory == env::current_account_id(),
            "intent is for another factory"
        );
        require!(lead != i.client, "the client cannot fill its own intent");
        if let Some(leads) = &i.leads {
            require!(
                leads.contains(&lead),
                "this intent names its leads: you are not one of them"
            );
        }
        let key = self
            .intent_keys
            .get(&i.client)
            .copied()
            .unwrap_or_else(|| env::panic_str("the client has no intent key"));
        let sig: [u8; 64] = base64_decode(&signature)
            .and_then(|b| b.try_into().ok())
            .unwrap_or_else(|| env::panic_str("signature must be 64 bytes, base64"));
        require!(
            env::ed25519_verify(&sig, intent_message(&intent), &key),
            "bad intent signature"
        );
        let now = env::block_timestamp();
        require!(now < i.ends_ns.0, "intent expired");
        require!(i.starts_ns.0 < i.ends_ns.0, "intent window is empty");
        let used_key = (i.client.clone(), i.nonce.0);
        require!(
            !self.used_nonces.contains(&used_key),
            "intent already filled or cancelled"
        );
        let have = self.deposits.get(&i.client).copied().unwrap_or(0);
        require!(
            have >= i.budget.0,
            "the client's deposit is below the budget"
        );
        let lead_fee = dutch_fee(i.max_lead_fee.0, i.starts_ns.0, i.ends_ns.0, now);
        let m = PostMsg {
            manager: lead.clone(),
            spec_hash: i.spec_hash.clone(),
            window_secs: i.window_secs,
            treasury,
            lead_fee: Some(U128(lead_fee)),
        };
        if let Err(reason) = self.check_post(&i.client, i.budget.0, &m) {
            env::panic_str(reason);
        }
        self.used_nonces.insert(used_key);
        self.deposits.insert(i.client.clone(), have - i.budget.0);
        emit(
            "intent_filled",
            json!({ "client": i.client, "nonce": i.nonce, "lead": lead, "lead_fee": U128(lead_fee), "budget": i.budget }),
        );
        self.launch(i.client, i.budget.0, m, true)
    }

    /// The client cancels an intent it signed (before anyone fills it). A
    /// nonce not yet spent attaches `STORAGE_NEAR`: the factory keeps the
    /// entry for good, so it is not free to make.
    #[payable]
    pub fn cancel_intent(&mut self, nonce: U128) {
        let who = env::predecessor_account_id();
        let key = (who.clone(), nonce.0);
        if !self.used_nonces.contains(&key) {
            require!(
                env::attached_deposit() >= STORAGE_NEAR,
                "attach 0.001 NEAR for the nonce's storage"
            );
            self.used_nonces.insert(key);
        }
        emit("intent_cancelled", json!({ "client": who, "nonce": nonce }));
    }

    /// The client takes back (part of) its intent deposit.
    pub fn withdraw_deposit(&mut self, amount: U128) -> Promise {
        let who = env::predecessor_account_id();
        let have = self.deposits.get(&who).copied().unwrap_or(0);
        require!(
            amount.0 > 0 && amount.0 <= have,
            "amount exceeds the deposit"
        );
        self.deposits.insert(who.clone(), have - amount.0);
        Promise::new(self.token.clone())
            .function_call(
                "ft_transfer",
                json!({ "receiver_id": who, "amount": amount })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(1),
                GAS_FT_TRANSFER,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_POSTED)
                    .on_deposit_withdrawn(who, amount),
            )
    }

    /// A failed withdrawal goes back to the deposit.
    #[private]
    pub fn on_deposit_withdrawn(&mut self, account: AccountId, amount: U128) -> bool {
        let ok = env::promise_result_checked(0, 64).is_ok();
        if !ok {
            let have = self.deposits.get(&account).copied().unwrap_or(0);
            self.deposits.insert(account, have + amount.0);
        }
        ok
    }

    pub fn get_intent_key(&self, account_id: AccountId) -> Option<String> {
        self.intent_keys
            .get(&account_id)
            .map(|k| format!("ed25519:{}", bs58_encode(k)))
    }

    pub fn get_deposit(&self, account_id: AccountId) -> U128 {
        U128(self.deposits.get(&account_id).copied().unwrap_or(0))
    }

    /// The lead fee a fill would get now, and whether the nonce is still open.
    pub fn quote_intent(&self, intent: String) -> serde_json::Value {
        let i: Intent = serde_json::from_str(&intent)
            .unwrap_or_else(|_| env::panic_str("intent must be the signed intent JSON"));
        let now = env::block_timestamp();
        json!({
            "lead_fee": U128(dutch_fee(i.max_lead_fee.0, i.starts_ns.0, i.ends_ns.0, now)),
            "open": now < i.ends_ns.0 && !self.used_nonces.contains(&(i.client.clone(), i.nonce.0)),
            "deposit": U128(self.deposits.get(&i.client).copied().unwrap_or(0)),
        })
    }

    /// After forwarding the budget. Returns what the token should refund to
    /// the client: 0 if the job kept everything.
    #[private]
    pub fn on_posted(
        &mut self,
        n: u32,
        amount: U128,
        fee: Option<U128>,
        from_deposit: Option<bool>,
    ) -> U128 {
        let fee = fee.map(|f| f.0).unwrap_or(0);
        let funded = amount.0 - fee;
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "callback: caller must be this contract"
        );
        require!(
            self.jobs.get(&n).map(|p| p.state) == Some(PostState::Pending),
            "post is not pending"
        );
        let used = match env::promise_result_checked(0, 128) {
            // Unreadable answer: assume the tokens left, so the factory never
            // refunds the client from other clients' tokens.
            Ok(b) => serde_json::from_slice::<U128>(&b)
                .map(|u| u.0.min(funded))
                .unwrap_or(funded),
            Err(_) => 0,
        };
        let mut p = self
            .jobs
            .get(&n)
            .cloned()
            .unwrap_or_else(|| env::panic_str("unknown job"));
        p.state = if used == funded {
            PostState::Posted
        } else {
            PostState::Failed
        };
        if p.state == PostState::Failed {
            // Give the posting NEAR back as credit (a failed create is
            // refunded to the factory by the runtime).
            let c = self.credits.get(&p.client).copied().unwrap_or(0);
            self.credits
                .insert(p.client.clone(), c + POST_NEAR.as_yoctonear());
        }
        emit(
            "posted",
            json!({ "n": n, "job": p.job, "state": p.state, "used": U128(used) }),
        );
        // Posted: keep the fee. Failed: refund everything not used, fee too.
        let keep = if p.state == PostState::Posted { fee } else { 0 };
        self.fees += keep;
        let back = amount.0 - used - keep;
        if from_deposit == Some(true) {
            // An intent fill: no token call to refund through, so what was
            // not used goes back to the client's deposit.
            let have = self.deposits.get(&p.client).copied().unwrap_or(0);
            self.deposits.insert(p.client.clone(), have + back);
            self.jobs.insert(n, p);
            return U128(0);
        }
        self.jobs.insert(n, p);
        U128(back)
    }

    /// Owner only: send the platform fees kept so far to the owner.
    pub fn withdraw_fees(&mut self) -> Promise {
        require!(env::predecessor_account_id() == self.owner, "owner only");
        let x = self.fees;
        require!(x > 0, "no fees to withdraw");
        self.fees = 0;
        Promise::new(self.token.clone())
            .function_call(
                "ft_transfer",
                json!({ "receiver_id": self.owner, "amount": U128(x) })
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(1),
                GAS_FT_TRANSFER,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_POSTED)
                    .on_fees_withdrawn(U128(x)),
            )
    }

    /// A failed withdrawal puts the fees back.
    #[private]
    pub fn on_fees_withdrawn(&mut self, amount: U128) -> bool {
        let ok = env::promise_result_checked(0, 64).is_ok();
        if !ok {
            self.fees += amount.0;
        }
        emit("fees_withdrawn", json!({ "amount": amount, "ok": ok }));
        ok
    }

    /// Called by a settled job of this factory's tree (any account under
    /// `.<factory>`: only the factory and its jobs can create those, and the
    /// jobs run the pinned global code). Counts one finished job.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        worker: AccountId,
        delivered: bool,
        checked: bool,
        junk: bool,
        paid: bool,
        quality: Option<u8>,
        amount: Option<U128>,
        payer: Option<AccountId>,
    ) {
        let caller = env::predecessor_account_id();
        require!(
            caller
                .as_str()
                .ends_with(&format!(".{}", env::current_account_id())),
            "only jobs of this factory report records"
        );
        let mut r = self.records.get(&worker).cloned().unwrap_or_default();
        r.jobs += 1;
        r.delivered += delivered as u32;
        r.checked += checked as u32;
        r.junk += junk as u32;
        r.paid += paid as u32;
        if let Some(q) = quality.filter(|q| *q <= 10) {
            r.score_sum += q as u32;
            r.scored += 1;
        }
        let v = amount.map(|a| a.0).unwrap_or(0);
        if paid {
            r.paid_value = U128(r.paid_value.0.saturating_add(v));
            let payer = payer.filter(|p| *p != worker);
            if let Some(p) = &payer {
                if self.payers.insert((worker.clone(), p.clone())) {
                    r.payers += 1;
                }
            }
            let mut book = self.payer_books.get(&worker).cloned().unwrap_or_default();
            book.credit(payer.as_ref(), v);
            self.payer_books.insert(worker.clone(), book);
        } else {
            r.unpaid_value = U128(r.unpaid_value.0.saturating_add(v));
        }
        emit(
            "record",
            json!({ "worker": worker, "by": caller, "record": r }),
        );
        self.records.insert(worker, r);
    }

    pub fn get_record(&self, account_id: AccountId) -> Record {
        self.records.get(&account_id).cloned().unwrap_or_default()
    }

    /// The record as a buyer who trusts only the `payers` managers sees it:
    /// `paid_value` and `payers` count only what those managers paid (from
    /// the bounded breakdown, so an entry that lost its place counts as
    /// nothing: the read errs toward distrust). Everything else, unpaid
    /// money included, is the whole record's. At most `MAX_LISTED_PAYERS`.
    pub fn get_record_from(&self, account_id: AccountId, payers: Vec<AccountId>) -> Record {
        require!(payers.len() <= MAX_LISTED_PAYERS, "list at most 16 payers");
        let mut r = self.get_record(account_id.clone());
        let (paid, n) = self
            .payer_books
            .get(&account_id)
            .map(|b| b.from_listed(&payers))
            .unwrap_or((0, 0));
        r.paid_value = U128(paid.min(r.paid_value.0));
        r.payers = n;
        r
    }

    /// Who paid this worker how much (the bounded breakdown).
    pub fn get_payers(&self, account_id: AccountId) -> PayerBook {
        self.payer_books
            .get(&account_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_job(&self, n: u32) -> Option<Posted> {
        self.jobs.get(&n).cloned()
    }

    pub fn get_settings(&self) -> serde_json::Value {
        json!({
            "owner": self.owner, "token": self.token, "fee_bps": self.fee_bps, "fees": U128(self.fees),
            "code_hash": Base58CryptoHash::from(self.code_hash),
            "signer": self.signer, "checker": self.checker, "judge": self.judge,
            "next": self.next, "rubric": self.rubric,
        })
    }
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    use near_sdk::mock::MockAction;
    use near_sdk::test_utils::{get_created_receipts, VMContextBuilder};
    use near_sdk::{testing_env, PromiseResult};

    const F: &str = "hiretree.test";
    const TOKEN: &str = "usdc.test";
    const BUDGET: u128 = 100 * MIN_BUDGET;

    fn acc(s: &str) -> AccountId {
        s.parse().unwrap()
    }

    fn ctx(pred: &str, results: Vec<PromiseResult>) {
        testing_env!(
            VMContextBuilder::new()
                .current_account_id(acc(F))
                .predecessor_account_id(acc(pred))
                .prepaid_gas(Gas::from_tgas(300))
                .account_balance(NearToken::from_near(10))
                .build(),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            Default::default(),
            results
        );
    }

    fn factory() -> Factory {
        ctx("owner.test", vec![]);
        let mut f = Factory::new(
            acc("owner.test"),
            acc(TOKEN),
            [7u8; 32].into(),
            "11".repeat(20),
            Some(acc("checker.test")),
        );
        prepay(&mut f, "client.test", 1);
        f
    }

    fn prepay(f: &mut Factory, who: &str, posts: u128) {
        testing_env!(VMContextBuilder::new()
            .current_account_id(acc(F))
            .predecessor_account_id(acc(who))
            .attached_deposit(NearToken::from_yoctonear(POST_NEAR.as_yoctonear() * posts))
            .build());
        f.prepay_post(None);
    }

    fn msg() -> String {
        json!({ "manager": "lead.test", "spec_hash": "ab".repeat(32), "window_secs": 3600,
                "treasury": "treasury.lead.test" })
        .to_string()
    }

    fn value(r: PromiseOrValue<U128>) -> u128 {
        match r {
            PromiseOrValue::Value(v) => v.0,
            PromiseOrValue::Promise(_) => panic!("expected a value"),
        }
    }

    #[test]
    fn post_job_creates_and_funds_root() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        assert!(matches!(
            f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg()),
            PromiseOrValue::Promise(_)
        ));
        let receipts = get_created_receipts();
        let create = receipts
            .iter()
            .find(|r| r.receiver_id == acc("job-1.hiretree.test"))
            .expect("create");
        assert!(create
            .actions
            .iter()
            .any(|a| matches!(a, MockAction::CreateAccount { .. })));
        let init = create
            .actions
            .iter()
            .find_map(|a| match a {
                MockAction::FunctionCallWeight {
                    method_name, args, ..
                } if method_name == b"new" => {
                    Some(serde_json::from_slice::<serde_json::Value>(args).unwrap())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(init["client"], "client.test");
        assert_eq!(init["manager"], "lead.test");
        assert_eq!(init["checker"], "checker.test");
        assert_eq!(init["treasury"], "treasury.lead.test");
        assert_eq!(init["signer"], "11".repeat(20));
        let methods: Vec<String> = receipts
            .iter()
            .flat_map(|r| r.actions.iter())
            .filter_map(|a| match a {
                MockAction::FunctionCallWeight { method_name, .. } => {
                    Some(String::from_utf8(method_name.clone()).unwrap())
                }
                _ => None,
            })
            .collect();
        for m in ["new", "storage_deposit", "ft_transfer_call", "on_posted"] {
            assert!(methods.contains(&m.to_string()), "{m}");
        }
        assert_eq!(f.get_job(1).unwrap().state, PostState::Pending);
        // The job kept everything: nothing to refund.
        ctx(
            F,
            vec![PromiseResult::Successful(
                format!("\"{BUDGET}\"").into_bytes(),
            )],
        );
        assert_eq!(f.on_posted(1, U128(BUDGET), None, None), U128(0));
        assert_eq!(f.get_job(1).unwrap().state, PostState::Posted);
    }

    #[test]
    fn a_window_shorter_than_a_minute_is_refused() {
        // The fund lands a few blocks after the job is created: a 1 s window
        // would close first and leave a root that never got its budget.
        let mut f = factory();
        ctx(TOKEN, vec![]);
        for w in [1u64, MIN_WINDOW_SECS - 1] {
            let m = json!({ "manager": "l.test", "spec_hash": "ab".repeat(32), "window_secs": w })
                .to_string();
            assert_eq!(
                value(f.ft_on_transfer(acc("c.test"), U128(BUDGET), m)),
                BUDGET
            );
        }
    }

    #[test]
    fn bad_post_msg_refunded() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        assert_eq!(
            value(f.ft_on_transfer(acc("c.test"), U128(BUDGET), "fund".into())),
            BUDGET
        );
        let bad_hash =
            json!({ "manager": "l.test", "spec_hash": "zz", "window_secs": 60 }).to_string();
        assert_eq!(
            value(f.ft_on_transfer(acc("c.test"), U128(BUDGET), bad_hash)),
            BUDGET
        );
        let bad_window =
            json!({ "manager": "l.test", "spec_hash": "ab".repeat(32), "window_secs": 0 })
                .to_string();
        assert_eq!(
            value(f.ft_on_transfer(acc("c.test"), U128(BUDGET), bad_window)),
            BUDGET
        );
        let extra =
            json!({ "manager": "l.test", "spec_hash": "ab".repeat(32), "window_secs": 60, "x": 1 })
                .to_string();
        assert_eq!(
            value(f.ft_on_transfer(acc("c.test"), U128(BUDGET), extra)),
            BUDGET
        );
        assert!(get_created_receipts().is_empty());
        assert!(f.get_job(1).is_none());
    }

    #[test]
    fn wrong_token_refunded() {
        let mut f = factory();
        ctx("fake-usdc.test", vec![]);
        assert_eq!(value(f.ft_on_transfer(acc("c.test"), U128(5), msg())), 5);
        ctx(TOKEN, vec![]);
        assert_eq!(value(f.ft_on_transfer(acc("c.test"), U128(0), msg())), 0);
        assert!(get_created_receipts().is_empty());
    }

    #[test]
    fn failed_post_refunds_client() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Failed]);
        assert_eq!(f.on_posted(1, U128(BUDGET), None, None), U128(BUDGET));
        assert_eq!(f.get_job(1).unwrap().state, PostState::Failed);
        // A partial answer refunds the rest.
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Successful(b"\"0\"".to_vec())]);
        assert_eq!(f.on_posted(2, U128(BUDGET), None, None), U128(BUDGET));
    }

    #[test]
    fn lead_fee_goes_into_the_root_init_and_cannot_exceed_the_budget() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        let over = json!({ "manager": "lead.test", "spec_hash": "ab".repeat(32), "window_secs": 60,
                           "lead_fee": (BUDGET + 1).to_string() })
        .to_string();
        assert_eq!(
            value(f.ft_on_transfer(acc("client.test"), U128(BUDGET), over)),
            BUDGET
        );
        let ok = json!({ "manager": "lead.test", "spec_hash": "ab".repeat(32), "window_secs": 60,
                         "lead_fee": "5000000" })
        .to_string();
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), ok);
        let init = get_created_receipts()
            .into_iter()
            .flat_map(|r| r.actions)
            .find_map(|a| match a {
                MockAction::FunctionCallWeight {
                    method_name, args, ..
                } if method_name == b"new" => {
                    Some(serde_json::from_slice::<serde_json::Value>(&args).unwrap())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(init["amount"], "5000000");
    }

    #[test]
    fn post_without_prepaid_near_refunded() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        assert_eq!(
            value(f.ft_on_transfer(acc("eve.test"), U128(BUDGET), msg())),
            BUDGET
        );
        assert!(get_created_receipts().is_empty());
    }

    #[test]
    fn one_credit_one_post() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        assert!(matches!(
            f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg()),
            PromiseOrValue::Promise(_)
        ));
        assert_eq!(f.get_credit(acc("client.test")).0, 0);
        ctx(TOKEN, vec![]);
        assert_eq!(
            value(f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg())),
            BUDGET
        );
    }

    #[test]
    fn dust_budget_refunded() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        assert_eq!(
            value(f.ft_on_transfer(acc("client.test"), U128(MIN_BUDGET - 1), msg())),
            MIN_BUDGET - 1
        );
        assert_eq!(f.get_credit(acc("client.test")).0, POST_NEAR.as_yoctonear());
    }

    #[test]
    fn failed_post_gives_the_credit_back() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Failed]);
        f.on_posted(1, U128(BUDGET), None, None);
        assert_eq!(f.get_credit(acc("client.test")).0, POST_NEAR.as_yoctonear());
    }

    #[test]
    fn unreadable_token_answer_counts_as_all_used() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Successful(b"garbage".to_vec())]);
        assert_eq!(f.on_posted(1, U128(BUDGET), None, None), U128(0));
    }

    #[test]
    #[should_panic(expected = "post is not pending")]
    fn on_posted_cannot_run_twice() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Failed]);
        f.on_posted(1, U128(BUDGET), None, None);
        f.on_posted(1, U128(BUDGET), None, None);
    }

    #[test]
    #[should_panic(expected = "owner only")]
    fn only_owner_changes_settings() {
        let mut f = factory();
        ctx("eve.test", vec![]);
        f.set_settings(
            None,
            Some("22".repeat(20)),
            None,
            None,
            None,
            None,
            None,
            None,
        );
    }

    #[test]
    fn owner_changes_settings_for_new_jobs() {
        let mut f = factory();
        ctx("owner.test", vec![]);
        f.set_settings(
            None,
            Some("22".repeat(20)),
            None,
            Some(true),
            None,
            None,
            None,
            None,
        );
        assert_eq!(f.get_settings()["signer"], "22".repeat(20));
        assert!(f.get_settings()["checker"].is_null());
    }

    #[test]
    fn judge_mode_replaces_the_checker_and_reaches_new_jobs() {
        let mut f = factory();
        ctx("owner.test", vec![]);
        f.set_settings(
            None,
            None,
            None,
            Some(true),
            Some("judge-model".into()),
            None,
            None,
            None,
        );
        assert_eq!(f.get_settings()["judge"], "judge-model");
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        let init = get_created_receipts()
            .into_iter()
            .flat_map(|r| r.actions)
            .find_map(|a| match a {
                MockAction::FunctionCallWeight {
                    method_name, args, ..
                } if method_name == b"new" => {
                    Some(serde_json::from_slice::<serde_json::Value>(&args).unwrap())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(init["judge"], "judge-model");
        assert!(init["checker"].is_null());
        assert_eq!(init["rubric"], 2, "new jobs pin the latest judge rubric");
    }

    #[test]
    fn the_owner_can_pin_an_older_judge_rubric_but_not_an_unknown_one() {
        let mut f = factory();
        assert_eq!(f.get_settings()["rubric"], ht_core::check::RUBRIC_LATEST);
        ctx("owner.test", vec![]);
        // A replay of a run recorded under rubric 1 needs jobs that build
        // the same request bytes.
        f.set_settings(None, None, None, None, None, None, None, Some(1));
        assert_eq!(f.get_settings()["rubric"], 1);
        let bad = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.set_settings(None, None, None, None, None, None, None, Some(9))
        }));
        assert!(bad.is_err());
        assert_eq!(f.get_settings()["rubric"], 1);
    }

    #[test]
    fn jobs_of_this_factory_build_a_workers_record() {
        let mut f = factory();
        for (who, junk) in [
            ("t1.job-1.hiretree.test", false),
            ("job-2.hiretree.test", true),
        ] {
            ctx(who, vec![]);
            f.record(
                acc("w.test"),
                true,
                !junk,
                junk,
                !junk,
                Some(if junk { 2 } else { 8 }),
                None,
                None,
            );
        }
        let r = f.get_record(acc("w.test"));
        assert_eq!(
            (r.jobs, r.delivered, r.checked, r.junk, r.paid),
            (2, 2, 1, 1, 1)
        );
        assert_eq!((r.score_sum, r.scored), (10, 2));
    }

    #[test]
    #[should_panic(expected = "only jobs of this factory report records")]
    fn strangers_cannot_write_records() {
        let mut f = factory();
        ctx("hiretree.test.evil.test", vec![]);
        f.record(acc("w.test"), true, true, false, true, Some(10), None, None);
    }

    #[test]
    fn a_record_counts_money_and_distinct_payers() {
        let mut f = factory();
        let jobs = [
            ("t1.job-1.hiretree.test", true, 20, "lead-a.test"),
            ("t2.job-1.hiretree.test", true, 30, "lead-a.test"),
            ("t1.job-2.hiretree.test", true, 10, "lead-b.test"),
            ("t1.job-3.hiretree.test", false, 15, "lead-b.test"),
            // Paying yourself never adds a payer.
            ("t1.job-4.hiretree.test", true, 5, "w.test"),
        ];
        for (job, paid, v, payer) in jobs {
            ctx(job, vec![]);
            f.record(
                acc("w.test"),
                true,
                paid,
                !paid,
                paid,
                None,
                Some(U128(v * MIN_BUDGET)),
                Some(acc(payer)),
            );
        }
        let r = f.get_record(acc("w.test"));
        assert_eq!(r.paid_value.0, 65 * MIN_BUDGET);
        assert_eq!(r.unpaid_value.0, 15 * MIN_BUDGET);
        assert_eq!(r.payers, 2);
    }

    /// A paid job reported by a job of the tree: `v` whole fUSD from `payer`.
    fn paid(f: &mut Factory, worker: &str, payer: &str, v: u128) {
        ctx("t1.job-1.hiretree.test", vec![]);
        f.record(
            acc(worker),
            true,
            true,
            false,
            true,
            None,
            Some(U128(v * MIN_BUDGET)),
            Some(acc(payer)),
        );
    }

    fn entry(f: &Factory, worker: &str, payer: &str) -> Option<u128> {
        f.get_payers(acc(worker))
            .entries
            .iter()
            .find(|e| e.payer == acc(payer))
            .map(|e| e.paid.0 / MIN_BUDGET)
    }

    #[test]
    fn the_payer_breakdown_adds_up_to_paid_value() {
        let mut f = factory();
        paid(&mut f, "w.test", "lead-a.test", 20);
        paid(&mut f, "w.test", "lead-a.test", 10);
        paid(&mut f, "w.test", "lead-b.test", 7);
        // Paying yourself fills no entry; pay with no payer neither.
        paid(&mut f, "w.test", "w.test", 5);
        ctx("t2.job-1.hiretree.test", vec![]);
        f.record(
            acc("w.test"),
            true,
            true,
            false,
            true,
            None,
            Some(U128(3 * MIN_BUDGET)),
            None,
        );
        // An unpaid job adds to nothing here.
        ctx("t3.job-1.hiretree.test", vec![]);
        f.record(
            acc("w.test"),
            true,
            false,
            true,
            false,
            None,
            Some(U128(9 * MIN_BUDGET)),
            Some(acc("lead-c.test")),
        );
        let b = f.get_payers(acc("w.test"));
        let sum: u128 = b.entries.iter().map(|e| e.paid.0).sum::<u128>() + b.other.0;
        assert_eq!(sum, f.get_record(acc("w.test")).paid_value.0);
        assert_eq!(entry(&f, "w.test", "lead-a.test"), Some(30));
        assert_eq!(entry(&f, "w.test", "lead-b.test"), Some(7));
        assert_eq!(entry(&f, "w.test", "w.test"), None);
        assert_eq!(b.other.0, 8 * MIN_BUDGET);
        assert_eq!(f.get_payers(acc("nobody.test")), PayerBook::default());
    }

    #[test]
    fn a_read_from_listed_payers_counts_only_their_pay() {
        let mut f = factory();
        paid(&mut f, "w.test", "lead-a.test", 20);
        paid(&mut f, "w.test", "lead-b.test", 15);
        paid(&mut f, "w.test", "sybil-1.test", 40);
        paid(&mut f, "w.test", "sybil-2.test", 40);
        let all = f.get_record(acc("w.test"));
        assert_eq!((all.paid_value.0, all.payers), (115 * MIN_BUDGET, 4));
        let r = f.get_record_from(acc("w.test"), vec![acc("lead-a.test"), acc("lead-b.test")]);
        assert_eq!((r.paid_value.0, r.payers), (35 * MIN_BUDGET, 2));
        // Nothing else of the record moves.
        assert_eq!(
            (r.jobs, r.paid, r.unpaid_value),
            (all.jobs, all.paid, all.unpaid_value)
        );
        // Listing the same payer twice counts it once.
        let r = f.get_record_from(acc("w.test"), vec![acc("lead-a.test"), acc("lead-a.test")]);
        assert_eq!((r.paid_value.0, r.payers), (20 * MIN_BUDGET, 1));
        // Nobody listed, nobody counts; an unknown worker has nothing.
        let r = f.get_record_from(acc("w.test"), vec![]);
        assert_eq!((r.paid_value.0, r.payers), (0, 0));
        assert_eq!(
            f.get_record_from(acc("x.test"), vec![acc("lead-a.test")]),
            Record::default()
        );
    }

    #[test]
    fn unpaid_money_stays_whoever_is_listed() {
        let mut f = factory();
        ctx("t1.job-1.hiretree.test", vec![]);
        f.record(
            acc("w.test"),
            true,
            false,
            true,
            false,
            None,
            Some(U128(12 * MIN_BUDGET)),
            Some(acc("sybil.test")),
        );
        let r = f.get_record_from(acc("w.test"), vec![acc("lead-a.test")]);
        assert_eq!(r.unpaid_value.0, 12 * MIN_BUDGET);
    }

    #[test]
    #[should_panic(expected = "list at most 16 payers")]
    fn listing_more_than_sixteen_payers_is_refused() {
        let f = factory();
        let many: Vec<AccountId> = (0..17).map(|i| acc(&format!("p{i}.test"))).collect();
        f.get_record_from(acc("w.test"), many);
    }

    #[test]
    fn dust_never_takes_a_place() {
        let mut f = factory();
        ctx("t1.job-1.hiretree.test", vec![]);
        for i in 0..40 {
            f.record(
                acc("w.test"),
                true,
                true,
                false,
                true,
                None,
                Some(U128(MIN_BUDGET - 1)),
                Some(acc(&format!("d{i}.test"))),
            );
        }
        let b = f.get_payers(acc("w.test"));
        assert!(b.entries.is_empty());
        assert_eq!(b.other.0, 40 * (MIN_BUDGET - 1));
        // The payer count still counts them (as before).
        assert_eq!(f.get_record(acc("w.test")).payers, 40);
        // And a listed read sees none of them.
        let r = f.get_record_from(acc("w.test"), vec![acc("d1.test")]);
        assert_eq!((r.paid_value.0, r.payers), (0, 0));
    }

    #[test]
    fn the_breakdown_is_bounded() {
        let mut f = factory();
        let mut x = 12345u64;
        for _ in 0..400 {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            paid(
                &mut f,
                "w.test",
                &format!("p{}.test", (x >> 33) % 90),
                u128::from(1 + (x >> 50) % 40),
            );
            let b = f.get_payers(acc("w.test"));
            assert!(b.entries.len() <= MAX_PAYER_ENTRIES);
            let sum: u128 = b.entries.iter().map(|e| e.paid.0).sum::<u128>() + b.other.0;
            assert_eq!(sum, f.get_record(acc("w.test")).paid_value.0);
        }
        assert_eq!(f.get_payers(acc("w.test")).entries.len(), MAX_PAYER_ENTRIES);
    }

    #[test]
    fn spraying_small_payers_cannot_evict_an_honest_one() {
        let mut f = factory();
        // Three honest managers, then a sybil sprays 200 accounts at 1..=9
        // fUSD each, into a list that has room for 13 of them.
        for (p, v) in [
            ("lead-a.test", 30),
            ("lead-b.test", 25),
            ("lead-c.test", 20),
        ] {
            paid(&mut f, "w.test", p, v);
        }
        for i in 0..200 {
            paid(&mut f, "w.test", &format!("spray{i}.test"), 1 + i % 9);
        }
        let b = f.get_payers(acc("w.test"));
        assert_eq!(b.entries.len(), MAX_PAYER_ENTRIES);
        for p in ["lead-a.test", "lead-b.test", "lead-c.test"] {
            assert!(entry(&f, "w.test", p).is_some(), "{p} kept its place");
        }
        let r = f.get_record_from(
            acc("w.test"),
            vec![acc("lead-a.test"), acc("lead-b.test"), acc("lead-c.test")],
        );
        assert_eq!((r.paid_value.0, r.payers), (75 * MIN_BUDGET, 3));
    }

    #[test]
    fn a_full_list_is_displaced_only_by_paying_more_than_its_smallest() {
        let mut f = factory();
        for i in 0..MAX_PAYER_ENTRIES {
            paid(&mut f, "w.test", &format!("p{i}.test"), 10 + i as u128);
        }
        // p0 holds the smallest entry, 10. Paying 10 is not enough...
        paid(&mut f, "w.test", "new.test", 10);
        assert_eq!(entry(&f, "w.test", "new.test"), None);
        assert_eq!(entry(&f, "w.test", "p0.test"), Some(10));
        // ...paying 11 takes its place, and its 10 moves to `other`.
        paid(&mut f, "w.test", "new.test", 11);
        assert_eq!(entry(&f, "w.test", "new.test"), Some(11));
        assert_eq!(entry(&f, "w.test", "p0.test"), None);
        let b = f.get_payers(acc("w.test"));
        assert_eq!(b.entries.len(), MAX_PAYER_ENTRIES);
        assert_eq!(b.other.0, 20 * MIN_BUDGET);
        // The evicted payer counts as nothing in a listed read (distrust),
        // but is still one of the payers in the plain record.
        let r = f.get_record_from(acc("w.test"), vec![acc("p0.test")]);
        assert_eq!((r.paid_value.0, r.payers), (0, 0));
        assert_eq!(f.get_record(acc("w.test")).payers, 17);
    }

    #[test]
    fn an_entry_only_grows_and_is_never_the_one_displaced_by_its_own_top_up() {
        let mut f = factory();
        for i in 0..MAX_PAYER_ENTRIES {
            paid(&mut f, "w.test", &format!("p{i}.test"), 10);
        }
        // Equal smallest entries: the oldest goes first.
        paid(&mut f, "w.test", "new.test", 11);
        assert_eq!(entry(&f, "w.test", "p0.test"), None);
        assert_eq!(entry(&f, "w.test", "p1.test"), Some(10));
        // A small top-up of an existing entry always lands on it.
        paid(&mut f, "w.test", "p1.test", 1);
        assert_eq!(entry(&f, "w.test", "p1.test"), Some(11));
    }

    #[test]
    #[should_panic(expected = "not both")]
    fn checker_and_judge_together_refused() {
        let mut f = factory();
        ctx("owner.test", vec![]);
        f.set_settings(
            None,
            None,
            None,
            None,
            Some("judge-model".into()),
            None,
            None,
            None,
        );
    }

    #[test]
    #[should_panic(expected = "callback: caller must be this contract")]
    fn on_posted_is_private() {
        let mut f = factory();
        ctx("eve.test", vec![PromiseResult::Failed]);
        f.on_posted(1, U128(BUDGET), None, None);
    }

    #[test]
    fn the_platform_fee_comes_off_the_top_and_is_kept_only_when_posted() {
        assert_eq!(platform_fee(70_000_000, 50), 350_000);
        assert_eq!(platform_fee(1, 100), 0);
        let mut f = factory();
        ctx("owner.test", vec![]);
        f.set_settings(None, None, None, None, None, None, Some(50), None);
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        let fee = platform_fee(BUDGET, 50);
        let fund = get_created_receipts()
            .into_iter()
            .flat_map(|r| r.actions)
            .find_map(|a| match a {
                near_sdk::mock::MockAction::FunctionCallWeight {
                    method_name, args, ..
                } if method_name == b"ft_transfer_call" => Some(String::from_utf8(args).unwrap()),
                _ => None,
            })
            .unwrap();
        assert!(fund.contains(&format!("\"{}\"", BUDGET - fee)), "{fund}");
        // The job kept all it was sent: the factory keeps the fee.
        ctx(
            F,
            vec![PromiseResult::Successful(
                serde_json::to_vec(&U128(BUDGET - fee)).unwrap(),
            )],
        );
        assert_eq!(f.on_posted(1, U128(BUDGET), Some(U128(fee)), None), U128(0));
        assert_eq!(f.get_settings()["fees"], fee.to_string());
        // A failed post refunds everything, fee included.
        ctx(TOKEN, vec![]);
        prepay(&mut f, "client.test", 1);
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Failed]);
        assert_eq!(
            f.on_posted(2, U128(BUDGET), Some(U128(fee)), None),
            U128(BUDGET)
        );
        assert_eq!(f.get_settings()["fees"], fee.to_string());
    }

    #[test]
    #[should_panic(expected = "fee_bps is at most 100 (1%)")]
    fn the_fee_is_capped_at_one_percent() {
        let mut f = factory();
        ctx("owner.test", vec![]);
        f.set_settings(None, None, None, None, None, None, Some(101), None);
    }

    #[test]
    #[should_panic(expected = "owner only")]
    fn only_the_owner_withdraws_fees() {
        let mut f = factory();
        ctx("client.test", vec![]);
        f.withdraw_fees();
    }

    // ---------------------------------------------------- signed job intents

    use ed25519_dalek::{Signer, SigningKey};

    const T0: u64 = 1_000_000_000_000;
    const SPAN: u64 = 600_000_000_000; // 600 s

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32])
    }

    fn at(pred: &str, ts: u64) {
        testing_env!(VMContextBuilder::new()
            .current_account_id(acc(F))
            .predecessor_account_id(acc(pred))
            .prepaid_gas(Gas::from_tgas(300))
            .block_timestamp(ts)
            .account_balance(NearToken::from_near(10))
            .build());
    }

    fn intent(factory: &str, budget: u128, nonce: u128) -> String {
        json!({ "factory": factory, "client": "client.test", "budget": U128(budget),
                "spec_hash": "ab".repeat(32), "window_secs": 3600, "max_lead_fee": U128(10 * MIN_BUDGET),
                "starts_ns": U64(T0), "ends_ns": U64(T0 + SPAN), "nonce": U128(nonce) })
        .to_string()
    }

    fn sign(intent: &str) -> String {
        use near_sdk::base64::Engine;
        let sig = key().sign(&intent_message(intent));
        near_sdk::base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
    }

    /// A factory where client.test registered its key and deposited `dep`.
    fn with_deposit(dep: u128) -> Factory {
        let mut f = factory();
        at("client.test", T0);
        let pk = format!("ed25519:{}", bs58_encode(key().verifying_key().as_bytes()));
        f.set_intent_key(pk.clone());
        assert_eq!(f.get_intent_key(acc("client.test")), Some(pk));
        ctx(TOKEN, vec![]);
        assert_eq!(
            value(f.ft_on_transfer(acc("client.test"), U128(dep), "deposit".into())),
            0
        );
        f
    }

    #[test]
    fn the_lead_fee_rises_from_zero_to_the_max() {
        assert_eq!(dutch_fee(100, T0, T0 + SPAN, T0 - 1), 0);
        assert_eq!(dutch_fee(100, T0, T0 + SPAN, T0), 0);
        assert_eq!(dutch_fee(100, T0, T0 + SPAN, T0 + SPAN / 4), 25);
        assert_eq!(dutch_fee(100, T0, T0 + SPAN, T0 + SPAN), 100);
        assert_eq!(
            dutch_fee(u128::MAX, T0, T0 + SPAN, T0 + SPAN / 2),
            u128::MAX / 2
        );
    }

    #[test]
    fn a_lead_fills_a_signed_intent_and_the_job_is_posted() {
        let mut f = with_deposit(BUDGET);
        let i = intent(F, BUDGET, 1);
        // A quarter of the way through, the lead fee is a quarter of the max.
        at("lead.test", T0 + SPAN / 4);
        let _ = f.fill_intent(i.clone(), sign(&i), Some(acc("treasury.lead.test")));
        assert_eq!(f.get_deposit(acc("client.test")).0, 0);
        let p = f.get_job(1).unwrap();
        assert_eq!(
            (p.client.as_str(), p.manager.as_str()),
            ("client.test", "lead.test")
        );
        let init = get_created_receipts()
            .iter()
            .find(|r| r.receiver_id == acc("job-1.hiretree.test"))
            .and_then(|r| {
                r.actions.iter().find_map(|a| match a {
                    MockAction::FunctionCallWeight {
                        method_name, args, ..
                    } if method_name == b"new" => {
                        Some(serde_json::from_slice::<serde_json::Value>(args).unwrap())
                    }
                    _ => None,
                })
            })
            .expect("job init");
        assert_eq!(init["amount"], json!(U128(10 * MIN_BUDGET / 4)));
        assert_eq!(init["manager"], json!("lead.test"));
        assert_eq!(f.quote_intent(i)["open"], json!(false));
    }

    #[test]
    #[should_panic(expected = "intent already filled or cancelled")]
    fn an_intent_fills_only_once() {
        let mut f = with_deposit(2 * BUDGET);
        let i = intent(F, BUDGET, 1);
        at("lead.test", T0 + 1);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
        prepay(&mut f, "client.test", 1);
        at("lead2.test", T0 + 2);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
    }

    #[test]
    #[should_panic(expected = "bad intent signature")]
    fn a_tampered_intent_is_refused() {
        let mut f = with_deposit(2 * BUDGET);
        let signed = intent(F, BUDGET, 1);
        let sig = sign(&signed);
        // Same signature, bigger budget.
        at("lead.test", T0 + 1);
        let _ = f.fill_intent(intent(F, 2 * BUDGET, 1), sig, None);
    }

    #[test]
    #[should_panic(expected = "intent is for another factory")]
    fn an_intent_for_another_factory_is_refused() {
        let mut f = with_deposit(BUDGET);
        let i = intent("other.test", BUDGET, 1);
        at("lead.test", T0 + 1);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
    }

    #[test]
    #[should_panic(expected = "intent expired")]
    fn an_expired_intent_is_refused() {
        let mut f = with_deposit(BUDGET);
        let i = intent(F, BUDGET, 1);
        at("lead.test", T0 + SPAN);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
    }

    #[test]
    #[should_panic(expected = "intent already filled or cancelled")]
    fn a_cancelled_intent_is_refused() {
        let mut f = with_deposit(BUDGET);
        pay("client.test", T0, STORAGE_NEAR.as_yoctonear());
        f.cancel_intent(U128(1));
        let i = intent(F, BUDGET, 1);
        at("lead.test", T0 + 1);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
    }

    #[test]
    #[should_panic(expected = "the client's deposit is below the budget")]
    fn an_intent_above_the_deposit_is_refused() {
        let mut f = with_deposit(BUDGET - 1);
        let i = intent(F, BUDGET, 1);
        at("lead.test", T0 + 1);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
    }

    #[test]
    #[should_panic(expected = "the client cannot fill its own intent")]
    fn the_client_cannot_fill_its_own_intent() {
        let mut f = with_deposit(BUDGET);
        let i = intent(F, BUDGET, 1);
        at("client.test", T0 + 1);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
    }

    #[test]
    fn a_failed_intent_post_goes_back_to_the_deposit() {
        let mut f = with_deposit(BUDGET);
        let i = intent(F, BUDGET, 1);
        at("lead.test", T0 + 1);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
        // The fund call failed: nothing used. The token refunds nothing (the
        // budget came from the deposit), so the deposit gets it all back.
        ctx(F, vec![PromiseResult::Failed]);
        assert_eq!(f.on_posted(1, U128(BUDGET), None, Some(true)), U128(0));
        assert_eq!(f.get_deposit(acc("client.test")).0, BUDGET);
        assert_eq!(f.get_job(1).unwrap().state, PostState::Failed);
    }

    // ------------------------------------------------ security review 6

    fn pay(pred: &str, ts: u64, yocto: u128) {
        testing_env!(VMContextBuilder::new()
            .current_account_id(acc(F))
            .predecessor_account_id(acc(pred))
            .prepaid_gas(Gas::from_tgas(300))
            .block_timestamp(ts)
            .attached_deposit(NearToken::from_yoctonear(yocto))
            .account_balance(NearToken::from_near(10))
            .build());
    }

    #[test]
    #[should_panic(expected = "attach 0.001 NEAR for the nonce's storage")]
    fn cancelling_an_intent_pays_for_its_storage() {
        // Each cancelled nonce is a new entry the factory stores for good:
        // free, anyone could fill the factory's storage with them.
        let mut f = with_deposit(BUDGET);
        at("eve.test", T0);
        f.cancel_intent(U128(7));
    }

    #[test]
    fn cancelling_twice_pays_once() {
        let mut f = with_deposit(BUDGET);
        pay("client.test", T0, STORAGE_NEAR.as_yoctonear());
        f.cancel_intent(U128(1));
        at("client.test", T0);
        f.cancel_intent(U128(1));
    }

    #[test]
    #[should_panic(expected = "prepay at least 0.001 NEAR")]
    fn a_dust_prepay_is_refused() {
        // 1 yocto used to open a credit entry for any account id.
        let mut f = factory();
        pay("eve.test", T0, 1);
        f.prepay_post(Some(acc("anyone-1.test")));
    }

    fn intent_for(leads: &[&str]) -> String {
        json!({ "factory": F, "client": "client.test", "budget": U128(BUDGET),
                "spec_hash": "ab".repeat(32), "window_secs": 3600, "max_lead_fee": U128(10 * MIN_BUDGET),
                "starts_ns": U64(T0), "ends_ns": U64(T0 + SPAN), "nonce": U128(1), "leads": leads })
        .to_string()
    }

    #[test]
    #[should_panic(expected = "this intent names its leads: you are not one of them")]
    fn an_intent_that_names_its_leads_is_filled_only_by_them() {
        // Whoever fills becomes the job's manager and spends its budget, so
        // a client can sign for leads it trusts instead of for anyone.
        let mut f = with_deposit(BUDGET);
        let i = intent_for(&["lead.test", "lead2.test"]);
        at("eve.test", T0 + 1);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
    }

    #[test]
    fn a_named_lead_fills_the_intent() {
        let mut f = with_deposit(BUDGET);
        let i = intent_for(&["lead.test", "lead2.test"]);
        at("lead2.test", T0 + 1);
        let _ = f.fill_intent(i.clone(), sign(&i), None);
        assert_eq!(f.get_job(1).unwrap().manager, acc("lead2.test"));
    }
}
