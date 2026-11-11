//! Money math. Every operation is checked: no silent wrap, no silent clamp.
//!
//! Amounts are token base units (`u128`). The demo token has 6 decimals.

/// Discount a lead's treasury takes when it buys a Job Note: 1%.
pub const NOTE_FEE_BPS: u128 = 100;
/// Basis points in one whole.
pub const BPS_DENOM: u128 = 10_000;
/// The lead fee's spend floor: the fee is earned over at least this share
/// of the root's work budget (`fee_floor`), 50%.
pub const FEE_FLOOR_BPS: u128 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoneyError {
    /// A sum or product does not fit in u128.
    Overflow,
    /// A subtraction would go below zero.
    Underflow,
    /// Zero amounts are refused (they hide bugs and NEP-141 rejects them).
    ZeroAmount,
    /// Asked to allocate more than is free.
    OverAllocated { requested: u128, available: u128 },
    /// The internal token balance cannot cover a payment.
    NotEnoughBalance { needed: u128, balance: u128 },
}

impl core::fmt::Display for MoneyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MoneyError::Overflow => write!(f, "amount overflow"),
            MoneyError::Underflow => write!(f, "amount underflow"),
            MoneyError::ZeroAmount => write!(f, "amount must be greater than zero"),
            MoneyError::OverAllocated {
                requested,
                available,
            } => write!(
                f,
                "amount {requested} exceeds unallocated funds {available}"
            ),
            MoneyError::NotEnoughBalance { needed, balance } => {
                write!(f, "need {needed} but internal balance is {balance}")
            }
        }
    }
}

pub fn add(a: u128, b: u128) -> Result<u128, MoneyError> {
    a.checked_add(b).ok_or(MoneyError::Overflow)
}

pub fn sub(a: u128, b: u128) -> Result<u128, MoneyError> {
    a.checked_sub(b).ok_or(MoneyError::Underflow)
}

/// The treasury's discount on a note: `amount * 100 / 10_000`, rounded down.
pub fn note_fee(amount: u128) -> Result<u128, MoneyError> {
    Ok(amount
        .checked_mul(NOTE_FEE_BPS)
        .ok_or(MoneyError::Overflow)?
        / BPS_DENOM)
}

/// What the treasury pays the worker for a note: `amount - amount*100bps/10000`.
/// Rounding favours the worker (the fee rounds down).
pub fn note_quote(amount: u128) -> Result<u128, MoneyError> {
    if amount == 0 {
        return Err(MoneyError::ZeroAmount);
    }
    sub(amount, note_fee(amount)?)
}

/// `a * b / c`, rounded down, exact even when `a * b` does not fit in
/// u128 (a 256-bit product, then long division). `Overflow` when `c` is 0
/// or the quotient does not fit.
pub fn mul_div_floor(a: u128, b: u128, c: u128) -> Result<u128, MoneyError> {
    if c == 0 {
        return Err(MoneyError::Overflow);
    }
    if let Some(p) = a.checked_mul(b) {
        return Ok(p / c);
    }
    // 256-bit product (hi, lo) from 64-bit halves.
    const M: u128 = u64::MAX as u128;
    let (a1, a0, b1, b0) = (a >> 64, a & M, b >> 64, b & M);
    let (p00, p01, p10, p11) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
    let mid = (p00 >> 64) + (p01 & M) + (p10 & M);
    let lo = (p00 & M) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    // Shift-subtract division, one bit at a time from the top.
    let (mut q, mut r) = (0u128, 0u128);
    for i in (0..256).rev() {
        let bit = if i >= 128 {
            (hi >> (i - 128)) & 1
        } else {
            (lo >> i) & 1
        };
        let carry = r >> 127;
        r = (r << 1) | bit;
        if carry == 1 || r >= c {
            r = r.wrapping_sub(c);
            if i >= 128 {
                return Err(MoneyError::Overflow);
            }
            q |= 1 << i;
        }
    }
    Ok(q)
}

/// The least `hired` that a root's lead fee is divided by:
/// `work_budget * FEE_FLOOR_BPS / 10_000`, rounded up in the client's
/// favour. `work_budget` is what the client funded for parts (the posted
/// budget less the fee cap).
pub fn fee_floor(work_budget: u128) -> Result<u128, MoneyError> {
    // work_budget = q * 10_000 + r: q * bps cannot overflow (bps <= 10_000).
    let (q, r) = (work_budget / BPS_DENOM, work_budget % BPS_DENOM);
    add(q * FEE_FLOOR_BPS, (r * FEE_FLOOR_BPS).div_ceil(BPS_DENOM))
}

/// The part of a root's lead fee the lead has earned:
/// `fee * paid / max(hired, floor)`, rounded down (the client keeps the
/// remainder unit). `hired` is the sum of the prices (pay, not sub-budget)
/// of the parts the lead hired that closed, at any depth, leaving out a
/// failed part whose task another part passed; `paid` the sum of those
/// whose work passed (the job contract's `fee_parts`). `floor` is the spend
/// floor (`fee_floor`): a lead that hires one tiny part that passes earns
/// next to nothing; it must pass work worth at least the floor to earn the
/// whole fee. No part paid earns nothing, and `paid` is capped at `hired`,
/// so the fee is never exceeded.
pub fn earned_fee(fee: u128, paid: u128, hired: u128, floor: u128) -> Result<u128, MoneyError> {
    if hired == 0 || paid == 0 || fee == 0 {
        return Ok(0);
    }
    mul_div_floor(fee, paid.min(hired), hired.max(floor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_is_one_percent_off() {
        assert_eq!(note_quote(10_000_000).unwrap(), 9_900_000);
        assert_eq!(note_fee(10_000_000).unwrap(), 100_000);
    }

    #[test]
    fn fee_rounds_down_in_favour_of_worker() {
        // 99 * 100 / 10_000 = 0.99 -> 0
        assert_eq!(note_quote(99).unwrap(), 99);
        assert_eq!(note_quote(100).unwrap(), 99);
        assert_eq!(note_quote(1).unwrap(), 1);
    }

    #[test]
    fn zero_note_is_refused() {
        assert_eq!(note_quote(0), Err(MoneyError::ZeroAmount));
    }

    #[test]
    fn huge_amount_overflows_instead_of_wrapping() {
        assert_eq!(note_quote(u128::MAX), Err(MoneyError::Overflow));
        assert!(note_quote(u128::MAX / NOTE_FEE_BPS).is_ok());
    }

    #[test]
    fn earned_fee_never_overflows() {
        assert_eq!(earned_fee(10, 30, 40, 0), Ok(7));
        assert_eq!(earned_fee(10, 0, 0, 0), Ok(0));
        assert_eq!(earned_fee(10, 40, 40, 0), Ok(10));
        // a * b above u128: exact, rounded down.
        let m = u128::MAX;
        assert_eq!(earned_fee(m, m, m, m), Ok(m));
        assert_eq!(earned_fee(m, 3, 4, 0), Ok(3 * (1u128 << 126) - 1));
        assert_eq!(earned_fee(m, m - 1, m, 0), Ok(m - 1));
        assert_eq!(earned_fee(m, m - 1, m - 1, m), Ok(m - 1));
        assert_eq!(earned_fee(m, m - 1, 1, m), Ok(1));
        assert_eq!(
            earned_fee(1 << 100, 1 << 100, (1 << 101) + 1, 0),
            Ok((1 << 99) - 1)
        );
        assert_eq!(mul_div_floor(m, m, 0), Err(MoneyError::Overflow));
    }

    #[test]
    fn earned_fee_divides_by_at_least_the_spend_floor() {
        // Work budget 90: floor 45. One tiny part of 1 that passed earns
        // 10 * 1 / 45 = 0, not the whole fee.
        let floor = fee_floor(90).unwrap();
        assert_eq!(floor, 45);
        assert_eq!(earned_fee(10, 1, 1, floor), Ok(0));
        // 60 hired, all passed: above the floor, the whole fee.
        assert_eq!(earned_fee(10, 60, 60, floor), Ok(10));
        // 30% of the budget (27), all passed: 10 * 27 / 45 = 6 (60%).
        assert_eq!(earned_fee(10, 27, 27, floor), Ok(6));
        // Exactly at the floor: the whole fee.
        assert_eq!(earned_fee(10, 45, 45, floor), Ok(10));
        // Nothing paid, or nothing hired: nothing, floor or not.
        assert_eq!(earned_fee(10, 0, 60, floor), Ok(0));
        assert_eq!(earned_fee(10, 0, 0, floor), Ok(0));
    }

    #[test]
    fn the_spend_floor_rounds_up_in_the_clients_favour() {
        assert_eq!(fee_floor(0), Ok(0));
        assert_eq!(fee_floor(1), Ok(1));
        assert_eq!(fee_floor(91), Ok(46));
        assert_eq!(fee_floor(u128::MAX), Ok(u128::MAX / 2 + 1));
        assert_eq!(FEE_FLOOR_BPS, 5_000);
    }

    #[test]
    fn checked_helpers() {
        assert_eq!(add(u128::MAX, 1), Err(MoneyError::Overflow));
        assert_eq!(sub(0, 1), Err(MoneyError::Underflow));
        assert_eq!(add(2, 3), Ok(5));
        assert_eq!(sub(3, 2), Ok(1));
    }
}
