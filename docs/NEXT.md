# Next: two designs not built for the hackathon

Both were on the plan. Each is written down here, with the reason it is
not in the code yet.

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

## 2. A NEAR Intents payout leg

**Design.** The worker names an asset and chain at award (for example
USDC on Base). At settle, the payee leg becomes an `ft_transfer_call` of
fUSD to `intents.near` with a deposit message for the worker, and the
worker's agent swaps or withdraws through the 1Click API. The job account
still deletes itself once its balance is zero, so nothing changes in the
fold.

**Why not now.** NEAR Intents runs on mainnet only (contest FAQ Q-007),
and this project is testnet and local node only, with fake test USD. A
leg that cannot run in the demo would be a claim, not a feature.
