//! Hire Tree factory (`hiretree.<net>`). A client posts and funds a job in
//! one `ft_transfer_call(factory, budget, msg)`; the factory creates
//! `job-<n>.<factory>` running the global job contract by code hash, inits
//! it with the pinned signer and checker, registers it on the token and
//! forwards the budget. If anything fails, the whole budget is returned as
//! unused, so the token refunds the client.

use ht_core::binding::{parse_addr20_hex, parse_hash32_hex};
use near_sdk::json_types::{Base58CryptoHash, U128};
use near_sdk::serde::Deserialize;
use near_sdk::serde_json::{self, json};
use near_sdk::store::LookupMap;
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

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde", deny_unknown_fields)]
struct PostMsg {
    manager: AccountId,
    spec_hash: String,
    window_secs: u64,
    treasury: Option<AccountId>,
    /// The lead's fee, paid at settle; the rest goes back to the client.
    lead_fee: Option<U128>,
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
    /// Platform fee on each posted budget, in basis points (max 100 = 1%).
    fee_bps: u16,
    /// Fees kept so far and not yet withdrawn.
    fees: u128,
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
            fee_bps: 0,
            fees: 0,
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
    ) {
        require!(env::predecessor_account_id() == self.owner, "owner only");
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
            json!({ "signer": self.signer, "checker": self.checker, "judge": self.judge, "fee_bps": self.fee_bps }),
        );
    }

    /// A client prepays NEAR for its posts: each post uses `POST_NEAR`
    /// (0.06 NEAR). The root's storage comes back to the factory when the
    /// root deletes; it is the posting fee.
    #[payable]
    pub fn prepay_post(&mut self, account_id: Option<AccountId>) -> U128 {
        let who = account_id.unwrap_or_else(env::predecessor_account_id);
        let add = env::attached_deposit().as_yoctonear();
        require!(add > 0, "attach NEAR");
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

    /// NEP-141 receiver: post a job. Anything not accepted is refunded.
    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        if env::predecessor_account_id() != self.token {
            return self.refund(amount, "wrong token");
        }
        if amount.0 < MIN_BUDGET {
            return self.refund(amount, "budget below the minimum (1 fUSD)");
        }
        let Ok(m) = serde_json::from_str::<PostMsg>(&msg) else {
            return self.refund(
                amount,
                "msg must be {manager, spec_hash, window_secs, treasury?, lead_fee?}",
            );
        };
        if parse_hash32_hex(&m.spec_hash).is_none() {
            return self.refund(amount, "spec_hash must be 64 hex chars");
        }
        // The platform fee comes off the top; the job is funded with the rest.
        let fee = platform_fee(amount.0, self.fee_bps);
        let funded = amount.0 - fee;
        let lead_fee = m.lead_fee.map(|f| f.0).unwrap_or(0);
        if lead_fee > funded {
            return self.refund(amount, "lead_fee is larger than the budget");
        }
        if m.window_secs < MIN_WINDOW_SECS || m.window_secs > MAX_WINDOW_SECS {
            return self.refund(amount, "window_secs out of range");
        }
        if env::prepaid_gas().saturating_sub(env::used_gas()) < GAS_POST_MIN {
            return self.refund(amount, "not enough gas to post (attach 200 Tgas)");
        }
        let credit = self.credits.get(&sender_id).copied().unwrap_or(0);
        if credit < POST_NEAR.as_yoctonear() {
            return self.refund(amount, "prepay 0.06 NEAR with prepay_post first");
        }
        self.credits
            .insert(sender_id.clone(), credit - POST_NEAR.as_yoctonear());
        let n = self.next;
        self.next += 1;
        let job: AccountId = format!("job-{n}.{}", env::current_account_id())
            .parse()
            .unwrap_or_else(|_| env::panic_str("job account id too long"));
        self.jobs.insert(
            n,
            Posted {
                job: job.clone(),
                client: sender_id.clone(),
                manager: m.manager.clone(),
                budget: amount,
                state: PostState::Pending,
            },
        );
        let init = json!({
            "client": sender_id, "manager": m.manager, "worker": null,
            "token": self.token, "amount": U128(lead_fee), "spec_hash": m.spec_hash,
            "window_secs": m.window_secs, "signer": self.signer,
            "code_hash": Base58CryptoHash::from(self.code_hash),
            "checker": self.checker, "treasury": m.treasury, "judge": self.judge,
        });
        emit(
            "post_started",
            json!({ "n": n, "job": job, "client": sender_id, "budget": amount, "platform_fee": U128(fee) }),
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
        PromiseOrValue::Promise(
            create.then(register).then(fund).then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_ON_POSTED)
                    .on_posted(n, amount, Some(U128(fee))),
            ),
        )
    }

    /// After forwarding the budget. Returns what the token should refund to
    /// the client: 0 if the job kept everything.
    #[private]
    pub fn on_posted(&mut self, n: u32, amount: U128, fee: Option<U128>) -> U128 {
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
        self.jobs.insert(n, p);
        U128(amount.0 - used - keep)
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
    pub fn record(
        &mut self,
        worker: AccountId,
        delivered: bool,
        checked: bool,
        junk: bool,
        paid: bool,
        quality: Option<u8>,
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
        emit(
            "record",
            json!({ "worker": worker, "by": caller, "record": r }),
        );
        self.records.insert(worker, r);
    }

    pub fn get_record(&self, account_id: AccountId) -> Record {
        self.records.get(&account_id).cloned().unwrap_or_default()
    }

    pub fn get_job(&self, n: u32) -> Option<Posted> {
        self.jobs.get(&n).cloned()
    }

    pub fn get_settings(&self) -> serde_json::Value {
        json!({
            "owner": self.owner, "token": self.token, "fee_bps": self.fee_bps, "fees": U128(self.fees),
            "code_hash": Base58CryptoHash::from(self.code_hash),
            "signer": self.signer, "checker": self.checker, "judge": self.judge,
            "next": self.next,
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
        assert_eq!(f.on_posted(1, U128(BUDGET), None), U128(0));
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
        assert_eq!(f.on_posted(1, U128(BUDGET), None), U128(BUDGET));
        assert_eq!(f.get_job(1).unwrap().state, PostState::Failed);
        // A partial answer refunds the rest.
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Successful(b"\"0\"".to_vec())]);
        assert_eq!(f.on_posted(2, U128(BUDGET), None), U128(BUDGET));
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
        f.on_posted(1, U128(BUDGET), None);
        assert_eq!(f.get_credit(acc("client.test")).0, POST_NEAR.as_yoctonear());
    }

    #[test]
    fn unreadable_token_answer_counts_as_all_used() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Successful(b"garbage".to_vec())]);
        assert_eq!(f.on_posted(1, U128(BUDGET), None), U128(0));
    }

    #[test]
    #[should_panic(expected = "post is not pending")]
    fn on_posted_cannot_run_twice() {
        let mut f = factory();
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Failed]);
        f.on_posted(1, U128(BUDGET), None);
        f.on_posted(1, U128(BUDGET), None);
    }

    #[test]
    #[should_panic(expected = "owner only")]
    fn only_owner_changes_settings() {
        let mut f = factory();
        ctx("eve.test", vec![]);
        f.set_settings(None, Some("22".repeat(20)), None, None, None, None, None);
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
        f.record(acc("w.test"), true, true, false, true, Some(10));
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
        );
    }

    #[test]
    #[should_panic(expected = "callback: caller must be this contract")]
    fn on_posted_is_private() {
        let mut f = factory();
        ctx("eve.test", vec![PromiseResult::Failed]);
        f.on_posted(1, U128(BUDGET), None);
    }

    #[test]
    fn the_platform_fee_comes_off_the_top_and_is_kept_only_when_posted() {
        assert_eq!(platform_fee(70_000_000, 50), 350_000);
        assert_eq!(platform_fee(1, 100), 0);
        let mut f = factory();
        ctx("owner.test", vec![]);
        f.set_settings(None, None, None, None, None, None, Some(50));
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
        assert_eq!(f.on_posted(1, U128(BUDGET), Some(U128(fee))), U128(0));
        assert_eq!(f.get_settings()["fees"], fee.to_string());
        // A failed post refunds everything, fee included.
        ctx(TOKEN, vec![]);
        prepay(&mut f, "client.test", 1);
        ctx(TOKEN, vec![]);
        f.ft_on_transfer(acc("client.test"), U128(BUDGET), msg());
        ctx(F, vec![PromiseResult::Failed]);
        assert_eq!(f.on_posted(2, U128(BUDGET), Some(U128(fee))), U128(BUDGET));
        assert_eq!(f.get_settings()["fees"], fee.to_string());
    }

    #[test]
    #[should_panic(expected = "fee_bps is at most 100 (1%)")]
    fn the_fee_is_capped_at_one_percent() {
        let mut f = factory();
        ctx("owner.test", vec![]);
        f.set_settings(None, None, None, None, None, None, Some(101));
    }

    #[test]
    #[should_panic(expected = "owner only")]
    fn only_the_owner_withdraws_fees() {
        let mut f = factory();
        ctx("client.test", vec![]);
        f.withdraw_fees();
    }
}
