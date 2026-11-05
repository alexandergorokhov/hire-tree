# Project: Hire Tree + Job Note

Spec-driven (OpenSpec layout). Each capability under `specs/<name>/spec.md`
lists requirements and WHEN/THEN scenarios. Every scenario names the test
that proves it (`unit:` = `contracts/job/src/tests.rs` or a core test,
`sandbox:` = `agents/tests/*.rs`). A scenario without a test is
marked `(planned)`.

Process: write or change the scenario first, then a failing test, then the
code (TDD). `make test` and `make lint` before every commit.

## Context
- A client funds a job. A lead agent (the manager) hires worker agents. Every
  job and sub-job is its own NEAR account running one global contract.
- Money is fake (`mock-usdc`, 6 decimals). Sandbox and testnet only.
- Worker deliveries can carry a NEAR AI Cloud receipt (an ECDSA signature by
  the model's TEE key) that the contract verifies with `env::ecrecover`.
- A paid-early note ("Job Note") may be sold only to the lead's own treasury.

## Capabilities
| Capability | What |
|---|---|
| job-tree | funding, bond, hire (sub-accounts from the global contract) |
| settle | bottom-up payout, delete, stuck-settle recovery |
| receipt | NEAR AI Cloud receipt verification and task binding |
| quality-check | one contract-started check per delivery (OutLayer; mock in sandbox) |
| job-note | sale of a worker's payout right to the lead's treasury |
| dispute | 2-of-3 check on a sub-job delivery |
| factory | registry: creates root jobs, pins signer, checker, token |

## Conventions
- Money rule: at every step, the sum of all balances in the tree plus
  everything paid out equals client funding plus the lead bond.
- State changes before promises; every callback handles failure; callbacks
  are `#[private]` and re-check the caller; checked math.
