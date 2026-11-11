# AGENTS.md: rules for coding agents in this repo

## Hard rules

1. **Never mainnet.** Sandbox and testnet only. No mainnet account ids, RPC
   URLs or deploy commands anywhere.
2. **Keys only in env vars.** Never write a private key, seed phrase or API
   key into any file, commit, log or fixture. `NEARAI_API_KEY` and testnet
   keys come from the environment of the owner's machine.
3. **Never run or trigger GitHub Actions.** No workflow files that run on
   push, no `gh workflow run`, no re-runs.
4. **All money is fake.** The only token is `mock-usdc` ("FAKE test USD", no
   value). Never point the contracts at a real stablecoin.
5. **No fake success.** A target or feature that is not built prints
   `not built yet` and exits non-zero. A contract path that is not built
   refuses instead of pretending.
6. **No code copied from Agent Exit.** That repo may be read for build
   conventions only.

## Commands

| Command | What it does |
|---|---|
| `make test` | all unit and property tests, offline |
| `make lint` | `cargo fmt --check` and clippy `-D warnings` |
| `make build` | wasm into `out/` with `SHA256SUMS` |
| `make sandbox` | near-sandbox tests (local node only, never testnet or mainnet) |

Run `make test` and `make lint` before every commit. Use
`CARGO_INCREMENTAL=0` (the Makefile sets it) to keep `target/` small.

## Contract rules (plan section 10)

- State changes before promises. Every callback handles failure and
  restores state. No `unwrap` on promise results.
- Callbacks are `#[private]` and also call `assert_self()`.
- Delete an account only in a callback, after every transfer succeeded and
  the token reports a zero balance, and (sub-jobs) after the parent's
  `child_closed` succeeded. The beneficiary is the parent.
- `child_closed` accepts only a known, open child, or one whose hire is
  still in flight when the notice says what it was funded with; a repeat
  from a child it already closed is a no-op.
- Checked math everywhere (`ht_core::money`, `ht_core::Ledger`).

## Where things are

- `core/src/`: money, ledger, binding, receipt text, state machine.
- `core/tests/money_prop.rs`: the money-rule property test.
- `contracts/job/src/lib.rs`: the job contract; `tests.rs` beside it.
- `contracts/mock-usdc/src/lib.rs`: the fake token.
- `agents/tests/tree.rs`: G1, G4 and the 2-level fold on near-sandbox.
  `agents/fdshim/` is only loaded when neard cannot raise its
  open-file limit.
