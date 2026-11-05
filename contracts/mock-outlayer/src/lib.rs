//! Sandbox stand-in for the quality checker. The real one (testnet) is an
//! OutLayer job that asks NEAR AI Cloud to score a deliverable against the
//! spec with a fixed rubric. This mock returns a score recorded per
//! deliverable hash (default 8), so sandbox runs are deterministic and free.
//! It never calls any AI.

use near_sdk::store::LookupMap;
use near_sdk::{env, near, require, AccountId, PanicOnDefault};

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MockOutlayer {
    owner: AccountId,
    scores: LookupMap<String, Vec<u8>>,
    default_score: u8,
    calls: u64,
}

#[near]
impl MockOutlayer {
    #[init]
    pub fn new(owner: AccountId, default_score: u8) -> Self {
        require!(default_score <= 10, "score is 0..=10");
        Self {
            owner,
            scores: LookupMap::new(b"s"),
            default_score,
            calls: 0,
        }
    }

    /// Owner only: the scores panels 0, 1, 2 return for a deliverable hash.
    pub fn set_scores(&mut self, deliverable_hash: String, scores: Vec<u8>) {
        require!(env::predecessor_account_id() == self.owner, "owner only");
        require!(scores.iter().all(|s| *s <= 10), "score is 0..=10");
        self.scores.insert(deliverable_hash, scores);
    }

    /// Same interface the job contract calls on the real checker.
    #[allow(unused_variables)]
    pub fn check(
        &mut self,
        job: AccountId,
        spec_hash: String,
        deliverable_hash: String,
        seq: u32,
        seed: String,
        panel: u8,
    ) -> u8 {
        self.calls += 1;
        let score = self
            .scores
            .get(&deliverable_hash)
            .and_then(|v| v.get(panel as usize).copied())
            .unwrap_or(self.default_score);
        env::log_str(&format!(
            "mock-outlayer: job {job} seq {seq} panel {panel} score {score}"
        ));
        score
    }

    pub fn calls(&self) -> u64 {
        self.calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::testing_env;

    fn ctx(pred: AccountId) {
        testing_env!(VMContextBuilder::new().predecessor_account_id(pred).build());
    }

    #[test]
    fn default_and_recorded_scores() {
        ctx(accounts(0));
        let mut m = MockOutlayer::new(accounts(0), 8);
        let c = |m: &mut MockOutlayer, h: &str, p| {
            m.check(accounts(1), "s".into(), h.into(), 1, "x".into(), p)
        };
        assert_eq!(c(&mut m, "aa", 0), 8);
        m.set_scores("bb".into(), vec![9, 2, 3]);
        assert_eq!(c(&mut m, "bb", 0), 9);
        assert_eq!(c(&mut m, "bb", 2), 3);
        assert_eq!(c(&mut m, "bb", 5), 8);
        assert_eq!(m.calls(), 4);
    }

    #[test]
    #[should_panic(expected = "owner only")]
    fn only_owner_sets_scores() {
        ctx(accounts(0));
        let mut m = MockOutlayer::new(accounts(0), 8);
        ctx(accounts(1));
        m.set_scores("aa".into(), vec![1]);
    }
}
