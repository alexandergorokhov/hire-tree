# Hire Tree + Job Note

AI agents hire other AI agents on NEAR, and get paid per job.

A client funds a job. A lead agent manages it and hires worker agents. Every
job and sub-job is its own NEAR sub-account (`job-1.hiretree.testnet`,
`t1.job-1.hiretree.testnet`, ...), all running one global contract. Each
account holds that job's fake USDC. When the 72 h window ends, or the client
approves, `settle` runs bottom-up: each account pays its worker, sends the
rest to its parent, and deletes itself.

Later: a worker that delivers through NEAR AI Cloud submits the signed
receipt, the contract checks it on-chain, and the worker's payout right (the
Job Note) can be sold to the lead's treasury for 1% off, so the worker is
paid at once.

**Status: work in progress, day 3 of 8.** Testnet only. All money is fake.

## Do X, see Y (works today)

| Do | See |
|---|---|
| `make setup` | toolchain versions; crates fetched (the only networked step) |
| `make test` | `core` unit + property tests (10,000 random job trees, money rule holds), job contract and mock-usdc unit tests; all `ok`, about 40 s after the first build |
| `PROP_CASES=500 make test` | the same, with fewer property cases |
| `make lint` | rustfmt check and clippy with `-D warnings`; exit 0 |
| `make build` | `out/hire_tree_job.wasm`, `out/mock_usdc.wasm`, `out/SHA256SUMS` (non-reproducible build for now) |
| `make sandbox` | a local NEAR node (near-sandbox; its binary is downloaded on the first build) runs 3 tests: G4 (global contract by code hash, `hire` creates `t1.job-1...` from it), G1 (no delete while a transfer failed or stray tokens remain; delete returns the storage NEAR to the parent), and a 2-level tree that settles bottom-up with every account deleted and `30 + 20 + 8 + 52 == 100 funded + 10 bond`; `test result: ok. 3 passed`, about 1 min |
| `make demo`, `make web`, `make testnet-*` | `not built yet: ...` and a non-zero exit |

## What is built

- `core/`: pure Rust. Checked money math and the note quote
  (`amount - amount*100/10000`), the per-job ledger, the `HT1:` receipt
  binding parser, the signed text and EIP-191 digest, and the job state
  machine. Property test: `sum(balances in tree) + paid_out == client funding + lead bond`
  over random hire / deliver / check / buy / settle sequences with random
  transfer failures.
- `contracts/job/`: the job contract. Init, funding and bond
  (`ft_on_transfer`), `hire` (sub-account from the global contract by code
  hash, token registration, funding, rollback on failure), `deliver`
  (without a receipt), `approve`, `settle` (pay, return the rest, check the
  real token balance is zero, then delete), `child_closed`, `get_job`.
- `contracts/mock-usdc/`: NEP-141 fake test USD, 6 decimals, owner-only mint.
- `tests/sandbox/`: near-sandbox + near-api integration tests. Measured
  there: a job account uses 463-583 bytes (about 0.005 NEAR locked; each
  child record adds about 60 bytes); `hire` burns about 25 Tgas, `settle`
  11-16 Tgas. The root job is created by hand in the tests until the
  factory exists.

## Not built yet

Receipt verification (`ecrecover`), the quality check (OutLayer), note sale,
disputes, the factory, attack tests, agents, MCP server, web page, demo.
Where a later feature is missing, the contract refuses (for example a note is
never sellable yet) instead of faking success.

## Also by me

[Agent Exit](https://github.com/alexandergorokhov/near-transparent-agent-market)
sells AI agents. Hire Tree is about agents hiring agents and getting paid per
job. They are two independent projects (hackathon FAQ Q-015): separate repos,
no shared code.

## AI tools used

Claude Code wrote code and tests under the owner's direction.

## License

MIT. See `LICENSE`.
