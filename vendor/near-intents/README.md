# NEAR Intents Verifier (vendored, unmodified)

Release wasm files from <https://github.com/near/intents>, commit
`fce0936b1699e0591204ce152a78afc1c1338b35`, folder `releases/`, MIT licence
(`LICENSE`, NEAR Foundation). Used only on the local node, by
`agents/src/verifier.rs` and `agents/tests/intents.rs`. On another network
the job's `payout_via` points at the Verifier deployed there instead.

| File | What | sha256 |
|---|---|---|
| `defuse-0.2.10.wasm` | the Verifier (`defuse` 0.2.10) | `b76ee5e4d64646586bee19c346b269a18c1abdf5ac15990614d28eaa5ead8cec` |
| `wnear.wasm` | wrapped NEAR | `b72c26d390b23f4517d091a8aab06b0369d465356ec5fb337d5fda076da38ac0` |

Check: `sha256sum -c` against this table, or `make lint`.
