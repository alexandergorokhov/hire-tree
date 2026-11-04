//! The job state machine. Every status change in the contract goes through
//! `transition`, which rejects anything not listed here.
//!
//! ```text
//! Open --Deliver--> Delivered --CheckPassed--> Checked
//!                   Delivered --CheckFailed--> Open        (worker may redeliver)
//! Delivered|Checked --Dispute--> Disputed --DisputeUpheld--> Refunded
//!                                Disputed --DisputeRejected--> Checked
//! Open|Delivered|Checked|Refunded --StartSettle--> Settling --SettleOk--> Settled
//!                                  Settling --SettleFailed(prev)--> prev
//! ```

use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(not(target_arch = "wasm32"), derive(borsh::BorshSchema))]
pub enum Status {
    Open,
    Delivered,
    Checked,
    Settling,
    Settled,
    Refunded,
    Disputed,
}

impl Status {
    pub const ALL: [Status; 7] = [
        Status::Open,
        Status::Delivered,
        Status::Checked,
        Status::Settling,
        Status::Settled,
        Status::Refunded,
        Status::Disputed,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Open => "Open",
            Status::Delivered => "Delivered",
            Status::Checked => "Checked",
            Status::Settling => "Settling",
            Status::Settled => "Settled",
            Status::Refunded => "Refunded",
            Status::Disputed => "Disputed",
        }
    }

    /// Statuses a settle may start from (and fall back to on failure).
    pub fn can_settle_from(&self) -> bool {
        matches!(
            self,
            Status::Open | Status::Delivered | Status::Checked | Status::Refunded
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Deliver,
    CheckPassed,
    CheckFailed,
    Dispute,
    DisputeUpheld,
    DisputeRejected,
    StartSettle,
    SettleOk,
    /// A settle transfer failed: go back to the status before the settle.
    SettleFailed(Status),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllegalTransition {
    pub from: Status,
    pub event: Event,
}

impl core::fmt::Display for IllegalTransition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "illegal transition: {:?} from {}",
            self.event,
            self.from.as_str()
        )
    }
}

pub fn transition(from: Status, event: Event) -> Result<Status, IllegalTransition> {
    use Event as E;
    use Status as S;
    let to = match (from, event) {
        (S::Open, E::Deliver) => S::Delivered,
        (S::Delivered, E::CheckPassed) => S::Checked,
        (S::Delivered, E::CheckFailed) => S::Open,
        (S::Delivered | S::Checked, E::Dispute) => S::Disputed,
        (S::Disputed, E::DisputeUpheld) => S::Refunded,
        (S::Disputed, E::DisputeRejected) => S::Checked,
        (s, E::StartSettle) if s.can_settle_from() => S::Settling,
        (S::Settling, E::SettleOk) => S::Settled,
        (S::Settling, E::SettleFailed(prev)) if prev.can_settle_from() => prev,
        _ => return Err(IllegalTransition { from, event }),
    };
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Event as E;
    use Status as S;

    #[test]
    fn happy_path() {
        let s = transition(S::Open, E::Deliver).unwrap();
        let s = transition(s, E::CheckPassed).unwrap();
        let s = transition(s, E::StartSettle).unwrap();
        assert_eq!(transition(s, E::SettleOk).unwrap(), S::Settled);
    }

    #[test]
    fn failed_check_reopens() {
        assert_eq!(transition(S::Delivered, E::CheckFailed).unwrap(), S::Open);
    }

    #[test]
    fn settled_is_terminal() {
        for e in [
            E::Deliver,
            E::CheckPassed,
            E::CheckFailed,
            E::Dispute,
            E::DisputeUpheld,
            E::DisputeRejected,
            E::StartSettle,
            E::SettleOk,
            E::SettleFailed(S::Open),
        ] {
            assert!(transition(S::Settled, e).is_err(), "{e:?}");
        }
    }

    #[test]
    fn no_double_settle_or_deliver_while_settling() {
        assert!(transition(S::Settling, E::StartSettle).is_err());
        assert!(transition(S::Settling, E::Deliver).is_err());
        assert!(transition(S::Delivered, E::Deliver).is_err());
    }

    #[test]
    fn disputed_cannot_settle() {
        assert!(transition(S::Disputed, E::StartSettle).is_err());
    }

    #[test]
    fn settle_failure_only_returns_to_a_settleable_status() {
        assert_eq!(
            transition(S::Settling, E::SettleFailed(S::Checked)).unwrap(),
            S::Checked
        );
        assert!(transition(S::Settling, E::SettleFailed(S::Disputed)).is_err());
        assert!(transition(S::Settling, E::SettleFailed(S::Settled)).is_err());
        assert!(transition(S::Open, E::SettleFailed(S::Open)).is_err());
    }

    #[test]
    fn open_cannot_be_checked_or_disputed() {
        assert!(transition(S::Open, E::CheckPassed).is_err());
        assert!(transition(S::Open, E::Dispute).is_err());
    }
}
