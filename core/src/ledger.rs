//! The money ledger of one job account.
//!
//! The contract keeps one `Ledger` per job and calls these methods, so the
//! property tests in `tests/money_prop.rs` exercise the same code the
//! contract runs.
//!
//! Terms:
//! - `amount`: the job's own pay, owed to its payee (worker or note holder).
//!   Zero on a root job: the lead is paid from the root's remainder.
//! - `funded`: tokens received as funding (client on the root, parent on a sub-job).
//! - `bond`: the lead's bond (root only). Never allocatable to children.
//! - `allocated`: tokens sent, or in flight, to children.
//! - `ft_balance`: the internal mirror of the account's real token balance.

use borsh::{BorshDeserialize, BorshSerialize};

use crate::money::{add, note_quote, sub, MoneyError};
use crate::state::Status;

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "wasm32"), derive(borsh::BorshSchema))]
pub struct Ledger {
    pub amount: u128,
    pub funded: u128,
    pub bond: u128,
    pub allocated: u128,
    pub ft_balance: u128,
    /// The payee was paid (by a settle attempt whose transfer succeeded).
    pub payee_paid: bool,
}

/// How one settle attempt splits the internal balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Split {
    pub to_payee: u128,
    pub to_rest: u128,
}

impl Ledger {
    pub fn new(amount: u128) -> Self {
        Ledger {
            amount,
            ..Default::default()
        }
    }

    /// Funds that may still go to new children:
    /// `funded - amount - allocated`, or 0 when underfunded.
    pub fn unallocated(&self) -> u128 {
        self.funded
            .checked_sub(self.amount)
            .and_then(|r| r.checked_sub(self.allocated))
            .unwrap_or(0)
    }

    pub fn fund(&mut self, x: u128) -> Result<(), MoneyError> {
        if x == 0 {
            return Err(MoneyError::ZeroAmount);
        }
        let funded = add(self.funded, x)?;
        let bal = add(self.ft_balance, x)?;
        self.funded = funded;
        self.ft_balance = bal;
        Ok(())
    }

    pub fn add_bond(&mut self, x: u128) -> Result<(), MoneyError> {
        if x == 0 {
            return Err(MoneyError::ZeroAmount);
        }
        let bond = add(self.bond, x)?;
        let bal = add(self.ft_balance, x)?;
        self.bond = bond;
        self.ft_balance = bal;
        Ok(())
    }

    /// Reserve `x` for a child and take it off the balance (state before promise).
    pub fn allocate(&mut self, x: u128) -> Result<(), MoneyError> {
        if x == 0 {
            return Err(MoneyError::ZeroAmount);
        }
        let available = self.unallocated();
        if x > available {
            return Err(MoneyError::OverAllocated {
                requested: x,
                available,
            });
        }
        if x > self.ft_balance {
            return Err(MoneyError::NotEnoughBalance {
                needed: x,
                balance: self.ft_balance,
            });
        }
        let allocated = add(self.allocated, x)?;
        let bal = sub(self.ft_balance, x)?;
        self.allocated = allocated;
        self.ft_balance = bal;
        Ok(())
    }

    /// Undo `allocate(x)` after a failed hire (the tokens never left).
    pub fn release(&mut self, x: u128) -> Result<(), MoneyError> {
        let allocated = sub(self.allocated, x)?;
        let bal = add(self.ft_balance, x)?;
        self.allocated = allocated;
        self.ft_balance = bal;
        Ok(())
    }

    /// The worker was already paid `x` out of this job's own tokens (a note
    /// sale whose callback was lost): it is owed `x` less, and the job holds
    /// `x` less. The money rule is unchanged.
    pub fn prepay(&mut self, x: u128) -> Result<(), MoneyError> {
        let amount = sub(self.amount, x)?;
        let bal = sub(self.ft_balance, x)?;
        self.amount = amount;
        self.ft_balance = bal;
        Ok(())
    }

    /// A closed child sent back `x` of the `allocation` it was given: the
    /// tokens are back, and so much of the allocation is free to hire with
    /// again (a fired worker's runner-up is paid from it). Anything the
    /// child returns above its allocation is balance only.
    pub fn credit(&mut self, x: u128, allocation: u128) -> Result<(), MoneyError> {
        let freed = x.min(allocation).min(self.allocated);
        let bal = add(self.ft_balance, x)?;
        self.allocated = sub(self.allocated, freed)?;
        self.ft_balance = bal;
        Ok(())
    }

    /// Contractor mode: the worker turns `x` of its own pay into a sub-hire
    /// budget (its pay drops by `x`, so `unallocated` rises by `x`). It must
    /// keep part of its pay: `x < amount`.
    pub fn carve(&mut self, x: u128) -> Result<(), MoneyError> {
        if x == 0 {
            return Err(MoneyError::ZeroAmount);
        }
        if x >= self.amount {
            return Err(MoneyError::OverAllocated {
                requested: x,
                available: self.amount.saturating_sub(1),
            });
        }
        self.amount = sub(self.amount, x)?;
        Ok(())
    }

    /// Undo `carve(x)` after a failed contractor hire (after `release(x)`).
    pub fn uncarve(&mut self, x: u128) -> Result<(), MoneyError> {
        self.amount = add(self.amount, x)?;
        Ok(())
    }

    /// A contractor's sub-job closed and sent back `x` (its unspent or
    /// refunded pay): the tokens are back, and they are the worker's pay again.
    /// The allocation is freed as in `credit`, and the freed part becomes
    /// pay again (undoing the carve), so the contractor can hire again.
    pub fn restore(&mut self, x: u128, allocation: u128) -> Result<(), MoneyError> {
        let freed = x.min(allocation).min(self.allocated);
        let amount = add(self.amount, x)?;
        let bal = add(self.ft_balance, x)?;
        self.allocated = sub(self.allocated, freed)?;
        self.amount = amount;
        self.ft_balance = bal;
        Ok(())
    }

    /// The price the lead's treasury pays for this job's note, if it may be
    /// sold now: the delivery passed the check (`Checked`), the receipt was
    /// verified, the note was not sold, and no settle attempt has already
    /// paid the payee (a bought note must still be worth its face value).
    pub fn sellable_quote(&self, status: Status, verified: bool, note_sold: bool) -> Option<u128> {
        if status != Status::Checked || !verified || note_sold || self.payee_paid {
            return None;
        }
        note_quote(self.amount).ok()
    }

    /// The split for a settle attempt. The payee gets `amount` only if the
    /// work was delivered (`pay_payee`) and it was not already paid. The rest
    /// of the balance goes to the parent (root: to the manager).
    pub fn settle_split(&self, pay_payee: bool) -> Result<Split, MoneyError> {
        let to_payee = if pay_payee && !self.payee_paid {
            self.amount
        } else {
            0
        };
        if to_payee > self.ft_balance {
            return Err(MoneyError::NotEnoughBalance {
                needed: to_payee,
                balance: self.ft_balance,
            });
        }
        Ok(Split {
            to_payee,
            to_rest: self.ft_balance - to_payee,
        })
    }

    /// The root's split: the manager (payee) gets its fee (`amount`, set by
    /// the client at post) plus its bond back; everything else goes back to
    /// the client.
    /// A root that was never funded owes no fee (nothing was posted), so it
    /// can still settle and close.
    pub fn settle_split_root(&self) -> Result<Split, MoneyError> {
        let fee = if self.funded == 0 { 0 } else { self.amount };
        let to_payee = if self.payee_paid {
            0
        } else {
            add(fee, self.bond)?
        };
        if to_payee > self.ft_balance {
            return Err(MoneyError::NotEnoughBalance {
                needed: to_payee,
                balance: self.ft_balance,
            });
        }
        Ok(Split {
            to_payee,
            to_rest: self.ft_balance - to_payee,
        })
    }

    /// Take a split off the balance before sending the transfers.
    pub fn begin_payout(&mut self, s: Split) -> Result<(), MoneyError> {
        let total = add(s.to_payee, s.to_rest)?;
        self.ft_balance = sub(self.ft_balance, total)?;
        Ok(())
    }

    /// Apply transfer outcomes: a failed transfer's tokens stayed with us.
    pub fn payout_result(
        &mut self,
        s: Split,
        payee_ok: bool,
        rest_ok: bool,
    ) -> Result<(), MoneyError> {
        let mut bal = self.ft_balance;
        if payee_ok {
            if s.to_payee > 0 {
                self.payee_paid = true;
            }
        } else {
            bal = add(bal, s.to_payee)?;
        }
        if !rest_ok {
            bal = add(bal, s.to_rest)?;
        }
        self.ft_balance = bal;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn funded(amount: u128, funding: u128) -> Ledger {
        let mut l = Ledger::new(amount);
        l.fund(funding).unwrap();
        l
    }

    #[test]
    fn prepay_reduces_pay_and_balance_together() {
        let mut l = funded(30, 30);
        l.prepay(29).unwrap();
        assert_eq!((l.amount, l.ft_balance), (1, 1));
        assert!(l.prepay(2).is_err());
        assert_eq!((l.amount, l.ft_balance), (1, 1), "no partial change");
    }

    #[test]
    fn unallocated_excludes_own_pay_and_bond() {
        let mut l = funded(30, 100);
        l.add_bond(50).unwrap();
        assert_eq!(l.unallocated(), 70);
        assert_eq!(l.ft_balance, 150);
    }

    #[test]
    fn a_contractor_carves_its_own_pay_and_gets_refunds_back() {
        let mut l = Ledger::new(20);
        l.fund(20).unwrap();
        assert_eq!(l.unallocated(), 0);
        // It must keep some pay.
        assert!(l.carve(20).is_err());
        l.carve(12).unwrap();
        assert_eq!((l.amount, l.unallocated()), (8, 12));
        l.allocate(12).unwrap();
        // The sub-job failed and sent its 12 back: the pay is whole again.
        l.restore(12, 12).unwrap();
        assert_eq!((l.amount, l.ft_balance), (20, 20));
        // ...and it can carve and hire again (a runner-up).
        assert_eq!(l.allocated, 0);
        l.carve(12).unwrap();
        l.allocate(12).unwrap();
        assert_eq!((l.amount, l.allocated, l.ft_balance), (8, 12, 8));
    }

    #[test]
    fn a_returned_allocation_can_be_hired_again() {
        let mut l = Ledger::new(0);
        l.fund(30).unwrap();
        l.allocate(30).unwrap();
        assert_eq!(l.unallocated(), 0);
        // A fired worker's job sent all 30 back: the runner-up can be hired.
        l.credit(30, 30).unwrap();
        assert_eq!((l.allocated, l.ft_balance, l.unallocated()), (0, 30, 30));
        l.allocate(25).unwrap();
        // A child that paid its worker 20 of 25 returns 5: only 5 is free.
        l.credit(5, 25).unwrap();
        assert_eq!((l.allocated, l.ft_balance, l.unallocated()), (20, 10, 10));
    }

    #[test]
    fn allocate_and_release_round_trip() {
        let mut l = funded(0, 100);
        l.allocate(60).unwrap();
        assert_eq!((l.allocated, l.ft_balance, l.unallocated()), (60, 40, 40));
        assert_eq!(
            l.allocate(41),
            Err(MoneyError::OverAllocated {
                requested: 41,
                available: 40
            })
        );
        l.release(60).unwrap();
        assert_eq!(l, funded(0, 100));
    }

    #[test]
    fn zero_amounts_refused() {
        let mut l = Ledger::new(0);
        assert_eq!(l.fund(0), Err(MoneyError::ZeroAmount));
        assert_eq!(l.add_bond(0), Err(MoneyError::ZeroAmount));
        assert_eq!(l.allocate(0), Err(MoneyError::ZeroAmount));
    }

    #[test]
    fn release_more_than_allocated_fails() {
        let mut l = funded(0, 10);
        assert_eq!(l.release(1), Err(MoneyError::Underflow));
    }

    #[test]
    fn underfunded_has_nothing_unallocated() {
        let l = funded(50, 10);
        assert_eq!(l.unallocated(), 0);
    }

    #[test]
    fn split_pays_payee_only_when_delivered() {
        let l = funded(30, 100);
        assert_eq!(
            l.settle_split(true).unwrap(),
            Split {
                to_payee: 30,
                to_rest: 70
            }
        );
        assert_eq!(
            l.settle_split(false).unwrap(),
            Split {
                to_payee: 0,
                to_rest: 100
            }
        );
    }

    #[test]
    fn failed_payee_transfer_restores_balance_and_is_retried() {
        let mut l = funded(30, 100);
        let s = l.settle_split(true).unwrap();
        l.begin_payout(s).unwrap();
        assert_eq!(l.ft_balance, 0);
        l.payout_result(s, false, true).unwrap();
        assert_eq!(l.ft_balance, 30);
        assert!(!l.payee_paid);
        let s2 = l.settle_split(true).unwrap();
        assert_eq!(
            s2,
            Split {
                to_payee: 30,
                to_rest: 0
            }
        );
    }

    #[test]
    fn paid_payee_is_never_paid_twice() {
        let mut l = funded(30, 100);
        let s = l.settle_split(true).unwrap();
        l.begin_payout(s).unwrap();
        l.payout_result(s, true, false).unwrap();
        assert!(l.payee_paid);
        assert_eq!(l.ft_balance, 70);
        assert_eq!(
            l.settle_split(true).unwrap(),
            Split {
                to_payee: 0,
                to_rest: 70
            }
        );
    }

    #[test]
    fn note_sellable_only_when_checked_verified_unsold_unpaid() {
        let mut l = funded(1_000_000, 1_000_000);
        assert_eq!(
            l.sellable_quote(Status::Checked, true, false),
            Some(990_000)
        );
        assert_eq!(l.sellable_quote(Status::Delivered, true, false), None);
        assert_eq!(l.sellable_quote(Status::Checked, false, false), None);
        assert_eq!(l.sellable_quote(Status::Checked, true, true), None);
        l.payee_paid = true;
        assert_eq!(l.sellable_quote(Status::Checked, true, false), None);
        assert_eq!(
            Ledger::new(0).sellable_quote(Status::Checked, true, false),
            None
        );
    }

    #[test]
    fn split_refuses_when_balance_below_pay() {
        let mut l = Ledger::new(30);
        l.fund(10).unwrap();
        assert!(matches!(
            l.settle_split(true),
            Err(MoneyError::NotEnoughBalance { .. })
        ));
    }

    #[test]
    fn a_root_never_funded_settles_without_its_fee() {
        let l = Ledger::new(10);
        assert_eq!(
            l.settle_split_root(),
            Ok(Split {
                to_payee: 0,
                to_rest: 0
            })
        );
    }

    #[test]
    fn root_split_pays_fee_and_bond_to_manager_rest_to_client() {
        let mut l = Ledger::new(10);
        l.fund(100).unwrap();
        l.add_bond(5).unwrap();
        assert_eq!(l.unallocated(), 90, "fee and bond are never hireable");
        l.allocate(60).unwrap();
        l.credit(20, 60).unwrap(); // a child returned 20 of its 60
        let s = l.settle_split_root().unwrap();
        assert_eq!(
            s,
            Split {
                to_payee: 15,
                to_rest: 50
            }
        );
        l.begin_payout(s).unwrap();
        l.payout_result(s, true, false).unwrap();
        // Retry after a failed rest leg: the manager is not paid twice.
        assert_eq!(
            l.settle_split_root().unwrap(),
            Split {
                to_payee: 0,
                to_rest: 50
            }
        );
    }
}
