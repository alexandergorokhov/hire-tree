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
    fn checked_helpers() {
        assert_eq!(add(u128::MAX, 1), Err(MoneyError::Overflow));
        assert_eq!(sub(0, 1), Err(MoneyError::Underflow));
        assert_eq!(add(2, 3), Ok(5));
        assert_eq!(sub(3, 2), Ok(1));
    }
}
