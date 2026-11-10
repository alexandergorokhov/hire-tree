//! The pick rule as properties over random bids, records and parameters.
//! Cases: `PROP_CASES` (default 10,000).

use ht_core::pick::{pick, score, weigh, Cand, CENT};
use proptest::prelude::*;

fn cases() -> u32 {
    std::env::var("PROP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000)
}

fn cand() -> impl Strategy<Value = (u128, u32, u32, u128)> {
    (1u128..=1_000_000, 0u32..=5_000, 0u32..=5_000, 0u128..=20)
        .prop_map(|(bid, jobs, paid, vouch)| (bid, jobs, paid.min(jobs), vouch))
}

fn build(v: &[(u128, u32, u32, u128)]) -> Vec<Cand> {
    v.iter()
        .enumerate()
        .map(|(i, &(b, j, p, k))| (i, b, j, p, k))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    #[test]
    fn the_winner_has_the_least_score_and_is_a_candidate(
        v in prop::collection::vec(cand(), 1..=16),
        pct in 0u128..=500,
        w in 0u128..=1_000,
    ) {
        let c = build(&v);
        let win = pick(&c, pct, w).unwrap();
        let best = c.iter().map(|x| score(x, pct, w)).min().unwrap();
        let won = c.iter().find(|x| x.0 == win).unwrap();
        prop_assert_eq!(score(won, pct, w), best);
    }

    #[test]
    fn the_order_of_candidates_does_not_change_the_winner(
        v in prop::collection::vec(cand(), 1..=16),
        pct in 0u128..=500,
        w in 0u128..=1_000,
        rot in 0usize..16,
    ) {
        let c = build(&v);
        let mut r = c.clone();
        let n = r.len();
        r.rotate_left(rot % n);
        r.reverse();
        prop_assert_eq!(pick(&c, pct, w), pick(&r, pct, w));
    }

    #[test]
    fn a_lower_bid_never_loses_to_the_same_record_at_a_higher_one(
        rec in (0u32..=5_000, 0u32..=5_000, 0u128..=20),
        lo in 1u128..=500_000,
        gap in 1u128..=500_000,
        pct in 0u128..=500,
        w in 0u128..=1_000,
    ) {
        let (jobs, paid, vouch) = (rec.0, rec.1.min(rec.0), rec.2);
        let c = [(0, lo + gap, jobs, paid, vouch), (1, lo, jobs, paid, vouch)];
        prop_assert_eq!(pick(&c, pct, w), Some(1));
    }

    #[test]
    fn never_panics_on_any_inputs(
        v in prop::collection::vec((any::<u128>(), any::<u32>(), any::<u32>(), any::<u128>()), 0..=16),
        pct in any::<u128>(),
        w in any::<u128>(),
    ) {
        let c = build(&v);
        prop_assert_eq!(pick(&c, pct, w).is_some(), !c.is_empty());
    }

    #[test]
    fn weighing_never_hides_unpaid_money(
        paid in any::<u128>(),
        unpaid in any::<u128>(),
    ) {
        let (jobs, p) = weigh(paid, unpaid);
        prop_assert!(p <= jobs);
        // A record at least half unpaid (and worth a cent) weighs at least
        // about half unpaid, at any size.
        if unpaid >= paid && unpaid >= CENT {
            prop_assert!(jobs > 0);
            prop_assert!((jobs - p) as u64 + 1 >= p as u64, "{} {}", jobs, p);
        }
    }
}
