# Next: designs not (fully) built for the hackathon

Both were on the plan. Each is written down here, with what is built and
the reason the rest is not in the code yet.

## 1. A judge panel of several models

**Today.** The job pins one NEAR AI model and its signing key. Anyone may
submit signed answers to the contract's own check question; the lowest
score counts, and answers 3 or more apart leave the delivery unscored (the
manager decides). A worker who disagrees can dispute: two more panels
answer, and two junk votes of three uphold it (`finalize_dispute`).

**Design.** Pin up to three `(model, signing address)` pairs instead of
one. `deliver` opens one check round per model; `submit_check` verifies
each answer against its own model's key (the same
`model:sha256(request):sha256(answer)` receipt); `finalize_check` takes the
median of the models that answered, and leaves the delivery unscored when
fewer than two answered. Disputes keep the same three-vote rule, one vote
per model.

**Why not now.** NEAR AI Cloud attests each model separately and may sign
with a different key per model, so the panel needs one attested key per model pinned at post time. No live
NEAR AI call can be made from the build machine, and the hard cap on NEAR
AI spend is $0.10, so the panel could only be tested with fake keys. The
signed-receipt path is the part of the contract a judge trusts most;
changing it without a real receipt to test against adds risk for a small
gain.

## 2. A NEAR Intents payout leg (built)

Built on the local node with the Verifier's own release wasm (defuse 0.2.10
from github.com/near/intents, vendored in `vendor/near-intents/`). The
worker calls `set_payout_via(verifier)` on its job; at settle the payee leg
is `ft_transfer_call(verifier, pay, msg = worker)`, so the pay lands as the
worker's balance in the Verifier. The worker and a solver then swap with
signed NEP-413 `token_diff` intents and the worker withdraws. See
`agents/src/verifier.rs` and `agents/tests/intents.rs`.

Still not built: the cross-chain leg (the 1Click API, for example USDC on
Base). NEAR Intents' live solvers and bridges run on mainnet only (contest
FAQ Q-007), and this project stays on testnet and a local node.
