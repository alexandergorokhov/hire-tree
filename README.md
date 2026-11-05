# Hire Tree + Job Note

**Escrow for AI agents that hire AI agents: every job is its own NEAR
account.** The job account holds that job's money, checks a NEAR AI Cloud
signature to prove the AI did the work for this exact task, pays the worker
early if the work passes a quality check, and deletes itself when the job is
done. (The agents themselves are in progress; today the demo's lead and
workers are scripted accounts.)

Testnet and local node only. All money is fake test USD (`mock-usdc`).

![Scene 5: the researcher is paid now](docs/screens/scene-5-paid-now.png)

## The problem
On agent marketplaces today the escrow sits in one platform contract, and an
agent that subcontracts has no safe way to hand part of its budget to a
sub-agent.
Workers wait for the whole job to close before they are paid. Nothing proves
a deliverable came out of a model call made for this task.

## How it works
1. **Post.** A client prepays 0.06 NEAR of storage, then sends the budget
   to the factory with one `ft_transfer_call`, naming the lead and the
   lead's fee. The factory creates `job-1.hiretree.<net>` from one global
   contract (by code hash) and funds it. The account has no owner key.
2. **Hire.** The lead agent (manager) calls `hire`: a sub-account
   `t1.job-1...` is created from the same code and receives exactly that
   job's pay. Sub-jobs can hire in turn (a tree).
3. **Deliver with proof.** A worker that used NEAR AI Cloud sends the raw
   request, the response hash and the model's ECDSA signature. The contract
   hashes the request itself, requires the tag `HT1:<this job>:<spec hash>:<delivery #>`
   inside it, rebuilds `model:sha256(req):sha256(resp)`, and recovers the
   signer with `env::ecrecover`; it must equal the signer the factory pinned
   (NEAR AI Cloud's address on testnet). The factory owner can change the
   pinned signer and checker for new jobs only; existing jobs keep theirs.
4. **Check.** The contract (never the worker) starts one quality check per
   delivery on the pinned checker contract and hands it the delivered bytes.
   6/10 or more passes; no answer fails closed. The sandbox uses
   `mock-outlayer` (fixed scores); a checker that is itself a signed NEAR AI
   call is in progress.
5. **Paid now.** A verified and checked worker's payout right (the Job Note)
   can be bought by the lead's own treasury for 1% off: the worker is paid in
   the same transaction, the treasury collects the full pay at settle.
6. **Dispute.** The lead may dispute a sub-job once: two more checks; 2 junk
   votes of 3 send the pay back up the tree.
7. **Fold.** After the window (or approval), `settle` runs bottom-up, one
   transfer at a time: pay the note holder, return the rest to the parent
   (at the root: the lead's fee to the lead, everything else back to the
   client), confirm the real token balance is zero, delete the account
   (storage NEAR goes to the parent). Every stuck step (hire, check, note sale, settle, dispute) has a resume path, decided from the real token balance.

## Do X, see Y

| Do | See |
|---|---|
| `make setup` | toolchain versions; crates fetched (the only networked step besides the near-sandbox binary) |
| `make test` | 278 unit + property tests, all `ok`: core (56 + 5 property tests over 10,000 random job trees with random transfer failures), job contract (192), factory (16), mock checker (2), mock-usdc (7) |
| `make lint` | rustfmt check and clippy `-D warnings`; exit 0 |
| `make build` | `out/*.wasm` (job, factory, mock-usdc, mock-outlayer) and `out/SHA256SUMS` |
| `make sandbox` | a local NEAR node runs 4 tests: G4 global contract + sub-accounts, G1 delete only at a confirmed zero balance, a 2-level tree fold, and the full scene; `test result: ok. 4 passed`, about 2 min |
| `make demo` | the full story on a local node, narrated in 9 scenes (post, hire, copied receipt refused, verified + checked, note sold and worker paid now, no-receipt worker, junk disputed, fold, `100 in == 100 out`); writes `out/scene.json` |
| `make web` | serves `web/` on http://localhost:8080: the tree page replaying the `make demo` run, scene by scene |
| `make testnet-*`, `make record-fixtures` | `not built yet: ...` and a non-zero exit |

## Repo map
| Path | What |
|---|---|
| `openspec/` | specs: every requirement names the test that proves it; most have WHEN/THEN scenarios |
| `core/` | pure Rust: checked money math, ledger, `HT1` binding parser, receipt text, EIP-191 digest, signature parsing (low-s), state machine |
| `contracts/job/` | the global job contract |
| `contracts/factory/` | posts and funds root jobs; pins signer, checker, token |
| `contracts/mock-usdc/` | NEP-141 fake test USD |
| `contracts/mock-outlayer/` | sandbox stand-in for the OutLayer quality checker (recorded scores, no AI) |
| `tests/sandbox/` | near-sandbox integration tests and the demo scene |
| `web/` | static tree page |
| `docs/THREATS.md` | every threat and the test that answers it |
| `docs/PITCH.md` | pitch, 30 s speech, 3 min video script, judge Q&A |

## What is real and what is mocked
- Real on the local node: contracts, factory, global contract, sub-account
  create and delete, token transfers, `env::ecrecover`, storage refunds.
- Mocked in the sandbox: the quality checker (`mock-outlayer`, fixed scores)
  and the NEAR AI signing key (a test key). The signed text follows NEAR AI
  Cloud's documented format (`model:sha256(request):sha256(response)`);
  `recorded_text_matches_near_ai_cloud` checks the text shape only. A test
  with a real NEAR AI signature is pending (it needs the owner's API key).
- Tested depth: a 2-level tree on the local node; the 10,000-tree property
  test runs on the pure-Rust ledger model in `core/`, not on the contract.
- Not built yet: testnet deploy and run, agents, root (client) disputes.
  See `docs/THREATS.md` "Known limits".

## Also by me
[Agent Exit](https://github.com/alexandergorokhov/near-transparent-agent-market)
sells AI agent businesses. Hire Tree is about agents hiring agents and
getting paid per job. Two independent projects (hackathon FAQ Q-015):
separate repos, no shared code.

## AI tools used
Claude Code wrote the code, tests and docs under the owner's direction.

## License
MIT. See `LICENSE`.
