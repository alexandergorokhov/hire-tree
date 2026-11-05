//! Hire Tree core: pure Rust, no NEAR runtime.
//!
//! - [`money`]: checked u128 math and the note quote (1% off).
//! - [`ledger`]: one job account's money ledger, used by the contract.
//! - [`check`]: the signed quality check: request bytes, score parsing, tally.
//! - [`binding`]: the `HT1:` receipt binding tag and the deliverable hash.
//! - [`receipt`]: the NEAR AI Cloud signed text and its EIP-191 digest.
//! - [`state`]: the job state machine.

pub mod binding;
pub mod check;
pub mod ledger;
pub mod money;
pub mod receipt;
pub mod state;

pub use ledger::{Ledger, Split};
pub use money::MoneyError;
pub use state::{transition, Event, IllegalTransition, Status};
