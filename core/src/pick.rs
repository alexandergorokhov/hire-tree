//! The buyer's pick among bids: one pure rule, run on-chain by the job
//! contract (`award_best`) and off-chain by the swarm (to check the chain).
//!
//! The pick is the lowest bid after two penalties:
//! - a record penalty: the share of money a bidder's record shows unpaid
//!   (`record_weight`: 150 means x2.5 when everything is unpaid);
//! - for a bidder with no record yet, a newcomer premium (`newcomer_pct`)
//!   divided by how many times the slot's bond the bid carries (`vouch`).
//!
//! Ties go to the bidder with more paid money, then to the lower id (the
//! bid's position in the slot). Deterministic. Arithmetic saturates, so
//! absurd parameters cannot panic a contract call.

/// 0.01 fUSD in token units (fUSD has 6 decimals).
pub const CENT: u128 = 10_000;

/// A candidate: (id, bid, jobs, paid, vouch). `jobs` and `paid` are the
/// money-weighed record from [`weigh`]; `vouch` comes from [`vouch`].
pub type Cand = (usize, u128, u32, u32, u128);

/// A record weighed by money, in 0.01 fUSD: (all pay at stake, pay paid).
pub fn weigh(paid_value: u128, unpaid_value: u128) -> (u32, u32) {
    let paid = paid_value / CENT;
    let unpaid = unpaid_value / CENT;
    (
        paid.saturating_add(unpaid).min(u32::MAX as u128) as u32,
        paid.min(u32::MAX as u128) as u32,
    )
}

/// How many times the slot's bond a bid carries: its own bond plus an
/// underwriter's backing, over the slot's bond (at least 1).
pub fn vouch(bond: u128, backed: u128, slot_bond: u128) -> u128 {
    bond.saturating_add(backed) / slot_bond.max(1)
}

/// One candidate's score (lower wins).
pub fn score(c: &Cand, newcomer_pct: u128, record_weight: u128) -> u128 {
    let (_, bid, jobs, paid, v) = *c;
    let unpaid = jobs.saturating_sub(paid) as u128;
    let penalty = record_weight.saturating_mul(unpaid) / (jobs as u128).max(1);
    let mut s = bid.saturating_mul(100u128.saturating_add(penalty)) / 100;
    if jobs == 0 {
        s = s.saturating_mul(100u128.saturating_add(newcomer_pct / v.max(1))) / 100;
    }
    s
}

/// The winner's id, or None with no candidates.
pub fn pick(cands: &[Cand], newcomer_pct: u128, record_weight: u128) -> Option<usize> {
    cands
        .iter()
        .min_by_key(|c| {
            (
                score(c, newcomer_pct, record_weight),
                std::cmp::Reverse(c.3),
                c.0,
            )
        })
        .map(|c| c.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lowest_bid_wins_with_clean_records() {
        let c = [(0, 27, 0, 0, 1), (1, 19, 0, 0, 1), (2, 30, 0, 0, 1)];
        assert_eq!(pick(&c, 0, 150), Some(1));
    }

    #[test]
    fn no_candidates_no_winner() {
        assert_eq!(pick(&[], 10, 150), None);
    }

    #[test]
    fn unpaid_money_costs_more_than_a_low_bid_saves() {
        assert_eq!(pick(&[(0, 27, 3, 3, 1), (1, 13, 2, 0, 1)], 0, 150), Some(0));
        assert_eq!(pick(&[(0, 27, 3, 3, 1), (1, 19, 4, 3, 1)], 0, 150), Some(1));
    }

    #[test]
    fn a_heavier_record_weight_flips_the_pick() {
        let c = [(0, 15, 4, 3, 1), (1, 24, 4, 4, 1)];
        assert_eq!(pick(&c, 0, 150), Some(0));
        assert_eq!(pick(&c, 0, 400), Some(1));
    }

    #[test]
    fn a_newcomer_pays_a_premium_cut_by_its_vouch() {
        let c = [(0, 18, 5, 5, 1), (1, 13, 0, 0, 1)];
        assert_eq!(pick(&c, 0, 150), Some(1));
        assert_eq!(pick(&c, 75, 150), Some(0));
        let v = [(0, 13, 0, 0, 1), (1, 18, 0, 0, 10)];
        assert_eq!(pick(&v, 100, 150), Some(1));
        assert_eq!(pick(&v, 0, 150), Some(0));
    }

    #[test]
    fn ties_go_to_more_paid_money_then_the_lower_id() {
        assert_eq!(pick(&[(0, 18, 1, 1, 1), (1, 18, 4, 4, 1)], 0, 150), Some(1));
        assert_eq!(pick(&[(3, 10, 0, 0, 1), (2, 10, 0, 0, 1)], 0, 150), Some(2));
    }

    #[test]
    fn weigh_counts_cents_and_vouch_divides_by_the_slot_bond() {
        assert_eq!(weigh(30_000_000, 15_000_000), (4500, 3000));
        assert_eq!(weigh(9_999, 9_999), (0, 0));
        assert_eq!(vouch(50, 450, 50), 10);
        assert_eq!(vouch(0, 0, 0), 0);
        assert_eq!(vouch(5, 0, 0), 5);
    }

    #[test]
    fn absurd_inputs_do_not_panic() {
        let c = [(0, u128::MAX, u32::MAX, 0, 0), (1, u128::MAX, 0, 0, 0)];
        assert!(pick(&c, u128::MAX, u128::MAX).is_some());
        assert_eq!(weigh(u128::MAX, u128::MAX).0, u32::MAX);
    }
}
