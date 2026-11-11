//! Money math. Every operation is checked: no silent wrap, no silent clamp.
//!
//! Amounts are token base units (`u128`). The demo token has 6 decimals.

/// Discount a lead's treasury takes when it buys a Job Note: 1%.
pub const NOTE_FEE_BPS: u128 = 100;
/// Basis points in one whole.
pub const BPS_DENOM: u128 = 10_000;

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

/// The part of a root's lead fee the lead has earned: `fee * paid / hired`,
/// rounded down (the client keeps the remainder unit). `hired` is the sum
/// of the prices (pay, not sub-budget) of the parts the lead hired that
/// closed, at any depth, leaving out a failed part whose task another part
/// passed; `paid` the sum of those whose work passed (the job contract's
/// `fee_parts`). No part hired earns nothing, and `paid` is capped at
/// `hired`, so the fee is never exceeded.
pub fn earned_fee(fee: u128, paid: u128, hired: u128) -> Result<u128, MoneyError> {
    if hired == 0 || paid == 0 || fee == 0 {
        return Ok(0);
    }
    mul_div_floor(fee, paid.min(hired), hired)
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
        assert_eq!(earned_fee(10, 30, 40), Ok(7));
        assert_eq!(earned_fee(10, 0, 0), Ok(0));
        assert_eq!(earned_fee(10, 40, 40), Ok(10));
        // a * b above u128: exact, rounded down.
        let m = u128::MAX;
        assert_eq!(earned_fee(m, m, m), Ok(m));
        assert_eq!(earned_fee(m, 3, 4), Ok(3 * (1u128 << 126) - 1));
        assert_eq!(earned_fee(m, m - 1, m), Ok(m - 1));
        assert_eq!(
            earned_fee(1 << 100, 1 << 100, (1 << 101) + 1),
            Ok((1 << 99) - 1)
        );
        assert_eq!(mul_div_floor(m, m, 0), Err(MoneyError::Overflow));
    }

    #[test]
    fn checked_helpers() {
        assert_eq!(add(u128::MAX, 1), Err(MoneyError::Overflow));
        assert_eq!(sub(0, 1), Err(MoneyError::Underflow));
        assert_eq!(add(2, 3), Ok(5));
        assert_eq!(sub(3, 2), Ok(1));
    }
}
