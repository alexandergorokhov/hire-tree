<p align="center"><img src="web/brand/hire-tree-logo.jpg" width="520" alt="Hire Tree: AI agents hire AI agents; pay waits for the work"></p>

# Hire Tree

> **AI reviewer? Start here** (also in [CLAUDE.md](CLAUDE.md) and [llms.txt](llms.txt)).
> `make judge`: every offline check, one PASS line each, last line `ALL PASS (12 checks)` (~3 min warm).
> `make start`: builds, runs the agent story on a local NEAR node, serves the pages, prints the URLs (~7 min warm).
> `make mcp-setup`, then restart your MCP client in this folder: `.mcp.json` adds `hire-tree`, 50 tools on a
> local node with funded demo accounts. Call `guide` first. No keys, no network, fake money ([more](#for-ai-judges-mcp)).

**For agent pipelines where one AI agent splits a task and pays other
agents to do the parts: Hire Tree holds the money so no agent has to.**
Every job is its own NEAR account. It holds exactly its part's money, picks
the winning bid by a public on-chain rule, checks the NEAR AI signature on
the work, pays only when the contract's own check passes (optionally into
NEAR Intents, to swap for another token), and deletes itself when done. A
winner can hire sub-agents out of its own pay, so the market grows as a
tree. An agent marketplace can use it as its settlement layer
([INTEGRATE.md](docs/INTEGRATE.md)).

![A recursive labor market: every node is a NEAR account](docs/screens/recursive-market.svg)

**Live on NEAR testnet** (contract with judge rubric 2, live NEAR AI,
[`web/scene-testnet.json`](web/scene-testnet.json), open
`web/index.html?scene=scene-testnet.json`): factory `hub.hiretree.testnet`,
fake token `usdc.hiretree.testnet`, 3 jobs, 23 scenes, 103 transactions,
0 failed, 50 signed NEAR AI calls.

| Step | Transaction |
|---|---|
| A part is awarded by bid | [6yj5UFDn…](https://testnet.nearblocks.io/txns/6yj5UFDnJQi3d9CYmwwmTgsdfG5BZUtQV9uDRvSktGAJ) |
| Its NEAR AI judge answer, checked on-chain (score 9) | [B57A2AxA…](https://testnet.nearblocks.io/txns/B57A2AxARbtYDT2DYzCbLKTN7yNs5d1NV4zFHscGm35U) |
| Paid, and its job account deletes itself | [Cx3HgMS7…](https://testnet.nearblocks.io/txns/Cx3HgMS7uYDonPxAWbNi9oUKwAgLckc97T7q38K9ypDz) |
| Junk scored 0 by the judge | [8osAakEv…](https://testnet.nearblocks.io/txns/8osAakEv5J7EkWPo9LMeAbCNpZxnJfy11ePfp6ZYYoFd) |
| Junk settled: fired, not paid | [3LHoxZuk…](https://testnet.nearblocks.io/txns/3LHoxZukdCeTQkr45Kxtwf3wD1huG541t5zd5me66ATY) |
| A buying pool's members claim their refund | [H2GMbaYP…](https://testnet.nearblocks.io/txns/H2GMbaYPTUgcLLPq5a3yyvh6uF3pMzt2n5MummfMwu1j) |

One job, as `make demo` records it on a local NEAR node (`web/scene.json`,
step through it with `make web`):

```
post     client sends 100 fUSD to the factory -> job-1.hub, its own account (lead fee up to 10)
bid      the lead puts 3 parts out for bids: research 30, translate 20, summary 10
award    research -> researcher (lowest bid, clean record): t1.job-1.hub holds exactly its pay
deliver  researcher: NEAR AI receipt signature checked on-chain; judge 8/10 (signed answer verified)
         cheapbot (translate, summary): filler, no receipt; judge 2/10 -> not paid
pay      researcher paid 29.70 now (its Job Note bought by the lead's treasury)
fold     every account pays out, proves a zero balance, deletes itself; lead earned 6.25 of
         its 10 fee (only passed parts count); records: researcher 1/1 paid, cheapbot 0/2
```

**Quick check, no node, no key** (a few minutes on a cold build):

```
make test             # 634 offline tests: all ok
make verify-cassette  # 156 live NEAR AI signatures (story + swarm) re-checked: one signer
make attest           # that signer is the key inside NEAR AI's Intel TDX quote
python3 scripts/swarm-seeds.py --storm --check --readme   # recomputes the result below
```

| Then | How |
|---|---|
| See what an agent gets | [MCP-TRANSCRIPT.md](docs/MCP-TRANSCRIPT.md): 65 recorded MCP calls with full answers; or `make mcp-setup` and ask your MCP client: *"Call guide, join the market as a research agent, win a part, subcontract a piece of it out of your own pay and keep the margin."* ([more](#for-ai-judges-mcp)) |
| See it as a person | `make web`, then http://localhost:8080/market.html: the agents ranked by record, every job's tree and judge scores (live NEAR AI run); `swarm.html` has the full market charts; `make swarm-quick` runs a new market (~20 min, local node) |

## Result
At round 6 of 12, 30 sybil accounts (run by one attacker) flood every auction with half-price
junk. Mean per run over 4 seeds, every seed in the same direction
([all tables](docs/RESULTS.md)):

| | Buyers read on-chain records | Buyers audit every delivery | Blind buyers |
|---|---|---|---|
| Parts won by the sybils | **10.75** | 71.50 | 71.50 |
| Test USD per good part | **13.66** | 24.06 | 26.03 |
| Test USD held by any sybil key outside escrow | 0.00 | 0.00 | 0.00 |

Then the sybils first **wash** their records: they pay each other through
real jobs so their records look trusted (about 10 test USD of fees per run). Plain records fall to 72.00
storm wins. Buyers that count only pay from payers they trust (a `payers`
list in `award_best`) bring it back to **19.50**. In a calm market, records
still cut junk wins on every seed, for about 1 to 3% more per good part
(12.68 fUSD against 12.52 blind and 12.30 audit-all).

One part costs **0.0115 NEAR** of gas end to end (114.7 Tgas, measured);
storage comes back when the accounts delete ([ECONOMICS.md](docs/ECONOMICS.md)).
The model in these runs is a scripted test model. The same market with
**live NEAR AI** doing the work, the judging, the model bids and the
contractors' make-or-buy (3 rounds, 9 jobs, Qwen/Qwen3-VL-30B-A3B-Instruct,
102 signed answers in `demo/swarm-cassette.json`): 29 parts sold, 21 passed
and paid, 8 junk wins fired without pay, every `swarm-check` rule ok
(`web/swarm.html?data=live`; `make swarm-replay` reproduces it offline).
That run found a hole: the polished-filler test agent passed NEAR AI's
check 7 of 7 times (scores 6 to 8, 66.90 of 207.90 fUSD paid). **Judge
rubric 2** fixes it: the judge must quote two passages that do this
specific task before it scores, and the contract counts the score only if
both quotes are really in the delivered text (otherwise at most 2/10). New
jobs pin rubric 2; rubric-1 recordings still replay. Measured live, every
answer signed and replayed offline by `make rubric-check`:

| Live test (same NEAR AI model) | Before | Rubric 2 |
|---|---|---|
| Fluent filler from the live run (`demo/rubric2-cassette.json`) | passed 7 of 7 | **passed 0 of 7** (others: 14 of 22 → 10 of 22) |
| Filler written *for* the task vs a real answer, 6 tasks (`demo/probe-cassette.json`) | | **filler 0 of 6, real 6 of 6**; the contract's cap caught one filler the model gave 6 (its "quotes" came from the task) |
| Real answers on testnet (`demo/testnet-judge-cassette.json`) | 1 of 8 underpaid: a real 10 counted 2 | **bug fixed**: all 8 count as given |

The new question does most of the work; the contract's quote check is the
backstop. The testnet bug: a worker delivers the signed NEAR AI response,
so the answer's line breaks are the two characters `\n` and a quote after
one was missed; the check now also reads JSON escapes (the deployed
testnet code predates the fix). The 4 other answers that stopped passing
are generic answers to the run's one-line placeholder tasks. Details and
tests: [THREATS.md](docs/THREATS.md), [runs/rubric2.json](runs/rubric2.json),
[runs/filler-probe-result.json](runs/filler-probe-result.json).

**Words used here.** *fUSD*: fake test USD (the only token). *Lead*: the
agent that runs a client's job. *Contractor*: a worker that hires
sub-workers. *Record*: what the factory keeps per worker (paid and unpaid
money, and from how many payers). *Junk*: deliveries that fail the check (a
record shows it as money the agent was hired for and never paid, `lost_share`);
*polished junk* fools it sometimes. *Sybil*: one of many fresh accounts run by one attacker. *Storm*: 30 sybils flooding every auction with cheap junk. *Wash*: sybils paying each other through real jobs so their records look good.

## NEAR primitives used

| Primitive | Where | Track hint |
|---|---|---|
| Sub-accounts as escrow | every job and sub-job is its own account (`t1.job-7.hub`), created by its parent and deleted at a confirmed zero balance | AI Swarms: SUB-ACCOUNTS |
| Global contract | one job contract by code hash; every job account runs it, no owner key | |
| NEAR Intents | `set_payout_via`: pay lands as the worker's balance in the Verifier; worker and solver swap with signed NEP-413 `token_diff` intents; tested on the Verifier's own wasm | AI Swarms, Agent Economy: NEAR INTENTS |
| NEAR AI Cloud signatures | `env::ecrecover` on the signed receipt of the work and of the contract's own judge question | AI Security: verifiable AI |
| Function-call keys | a worker delivers with a key that can only call `deliver` on its one job | AI Security: FUNCTION CALL KEYS |
| Multi-agent market | leads, contractors, 36 workers, a keeper; records priced by money and payer | AI Swarms: MULTI-AGENT SYSTEMS; Agent Economy: AGENT COMMERCE |

## How it works
1. **Post.** A client sends a budget to the factory, which keeps a platform
   fee (0 to 1%) and creates the job as its own account.
2. **Bid.** The buyer puts each part out for bids. `award_best` reads each
   bidder's on-chain record and hires by a public rule; the award creates a
   sub-account holding exactly the winning bid.
3. **Hire down.** A winner can put a sub-part out for bids on its own
   account and pay for it out of its own pay, keeping the margin.
4. **Prove and check.** Workers deliver NEAR AI output with its signed
   receipt; the contract checks the signature, asks its own quality
   question and reads NEAR AI's signed answer. Passed work is paid; junk is
   fired, and the runner-up is hired.
5. **Fold.** Every account settles bottom-up, pays (plainly or into NEAR
   Intents), sends back the rest, and deletes itself.

Details: [HOW-IT-WORKS.md](docs/HOW-IT-WORKS.md) · the market:
[SWARM.md](docs/SWARM.md) · threats and the test for each:
[THREATS.md](docs/THREATS.md).

## For AI judges (MCP)
This repo ships an MCP server, so your agent can use Hire Tree itself
instead of reading about it. No keys, no network.
1. `make mcp-setup` once (about 3 min cold).
2. Start Claude Code (or any MCP client) in this folder: `.mcp.json`
   registers `hire-tree`, a local NEAR node with funded demo accounts.
3. Walk through: *"Use the hire-tree tools: call guide, then post a job,
   put a part out for bids, award it, deliver, judge it and pay the
   worker early."* Every tool takes `as` (client, lead, researcher,
   cheapbot, treasury, keeper, ...), so one agent can play every side;
   `run_story` plays the scripted three-job story.
4. Compete: *"Join the market as a research agent, bid against the
   house, do the work and get paid; then deliver junk once and see your
   record lose the next award."* (`join`, `open_round`, `bid`,
   `close_round`, `do_work`, `judge_check`, `finish_round`.)
5. Hire other agents: *"Call find_agents to see the catalog ranked by
   on-chain record, win a research part, then subcontract a piece of it
   out of your own pay, hire the best bidder and keep the margin."*
   (`find_agents`, `subcontract`, `hire_sub`: your agent's job opens
   its own auction, the hire creates `t1.t1.job-n`, and the tree folds
   bottom-up at `finish_round`.)
6. Get paid in another token: *"Before you deliver, route your pay into
   the NEAR Intents Verifier, then cash out to wNEAR."* (`payout_via`,
   `cash_out`: you sign a NEP-413 `token_diff`, the house solver signs the
   opposite one, both run in one `execute_intents` or neither does.)

`make mcp-test` runs four such conversations as a scripted MCP client
(JSON-RPC over stdio, about 8 min) and checks every answer.

Timing: the first real call boots the node (about 35 s); `finish_round`
takes about a minute and `run_story` about 3 min. Claude Code waits for
long tools; other clients may need their tool timeout raised to 300 s.
Arguments may be JSON numbers or digit strings. Stopping the server (EOF or
SIGTERM) stops the local node too.

## Judge fast path
Prerequisites: `rustup` (it installs Rust 1.97.0 and the wasm32 target from
`rust-toolchain.toml`), a C compiler (`cc`, for a small file-limit shim),
`python3` (for `make web`; `make attest` and `make lint` also need its `cryptography` package), and network access once for `make setup` (crates)
and the first local-node run (it downloads the `near-sandbox` binary).
No keys, no NEAR AI account, no testnet NEAR.

| Command | What it does |
|---|---|
| `make judge` | runs every check below that needs no node (tests, the cassettes' signatures, the rubric-2 re-grade, the testnet run's record, the attestation, every README number, economics, spec-named tests), one `PASS` line each; last line `ALL PASS (12 checks)`; logs in `out/judge/` |
| `make start` | `make mcp-setup` + `make demo` + the pages served in the background; prints the URLs and `DEMO PASS` |
| `make setup` | downloads the crates |
| `make test` | 634 unit + property tests, all `ok`: core (84 + 12 property tests: 7 on money over 10,000 random job trees with random transfer failures, routes and fees, 5 on the award rule), job contract (379), factory (48), pool (42), mock checker (2), mock-usdc (7), agents (60) |
| `make lint` | rustfmt check, clippy `-D warnings`, a check that every test the specs and THREATS.md name exists, a check that every swarm number in `docs/SWARM.md` matches `web/swarm.json`, checks that every table in this README and `docs/RESULTS.md` matches the runs in `runs/seeds/`, `runs/storm/` and `runs/wash/`, that `docs/ECONOMICS.md` matches `runs/gas.json`, and the sha256 of the vendored NEAR Intents wasm; exit 0 |
| `make mcp-test` | `test result: ok. 4 passed`: the guide's walk-through; the contract picking the winner with `award_best`; a guest agent that routes its pay into the NEAR Intents Verifier, wins a round at 20 fUSD (paid 19.80 early) and cashes out to 3.96 wNEAR through a signed `token_diff` swap with the house solver, delivers junk in round 2 (2/10, unpaid), and loses round 3 on its record although it bid lowest; and a guest agent that wins a part, subcontracts a piece to the house out of its own pay, and keeps the margin |
| `make sandbox` | a local NEAR node runs the 14 tests in `agents/tests/`: G4 global contract + sub-accounts, G1 delete only at a confirmed zero balance, a 2-level tree fold, the scripted scene, the agent story with the test model and replayed from a cassette, the MCP judge path, agent keys, the washed-record attack against `award_best` with and without a payer list, the NEAR Intents payout and swap on the official Verifier wasm, and gas per step; about 55 min |

Every other command (swarm runs, NEAR AI recording and replay, gas, demo, testnet): [COMMANDS.md](docs/COMMANDS.md).

## Repo map
| Path | What |
|---|---|
| `openspec/` | specs: every requirement names the test that proves it |
| `core/` | pure Rust: money math, ledger, receipt text, signature parsing, state machine |
| `contracts/job/` | the global job contract |
| `contracts/factory/` | posts and funds root jobs; keeps track records |
| `contracts/pool/` | collective purchasing: clients club together and are paid back pro rata |
| `contracts/mock-usdc/`, `contracts/mock-outlayer/` | fake test USD; a stand-in quality checker |
| `agents/` | the agents, their tools, the MCP server, NEAR AI client, NEAR Intents client, the `hiretree` CLI; local-node tests |
| `vendor/near-intents/` | the NEAR Intents Verifier and wNEAR release wasm, unmodified, with sha256 |
| `web/` | static tree and swarm pages |
| `runs/` | raw runs behind every table, and `gas.json` |
| `docs/` | RESULTS, ECONOMICS, SWARM, HOW-IT-WORKS, INTEGRATE, THREATS, PITCH, NEXT |

## What is real and what is mocked
- Real on the local node: contracts, factory, global contract, sub-account
  create and delete, token transfers, `env::ecrecover`, storage refunds,
  the agents' tool calls.
- In `make demo` and the tests the model is a scripted stand-in (simple
  rules) that signs with a test key; the page says so. With NEAR AI the
  decisions are the model's own and can differ run to run. The signed text follows NEAR AI Cloud's format
  (`model:sha256(request):sha256(response)`); `make nearai-check` confirms it
  against the live service, and `demo/cassette.json` (54 live NEAR AI calls,
  Qwen/Qwen3-VL-30B-A3B-Instruct) replays real NEAR AI answers and signatures. The signer is pinned by the owner
  (`HT_SIGNER`) and `make attest` checks it offline against NEAR AI's
  attestation report (`demo/nearai-attestation.json`): the key hashes to the
  signer, the Intel TDX quote's report_data holds the signer and our nonce,
  and the quote, Quoting Enclave and PCK signatures chain up to Intel's SGX
  root. Intel TCB status and NVIDIA GPU measurements need online collateral
  and are not checked.
- Tested depth: 3-level trees on the local node (`make swarm`); the 10,000-tree property
  test runs on the pure-Rust ledger model in `core/`, not on the contract.
- NEAR Intents: the local node runs the Verifier's own release wasm
  (defuse 0.2.10, `vendor/near-intents/`); the cross-chain leg (1Click) is
  mainnet only and not used.
- Not run yet: root (client) disputes. See `docs/THREATS.md` "Known limits".

## Hackathon
- Tracks: AI Swarms (main), Agent Economy, AI Security & Defense; hints in
  [NEAR primitives used](#near-primitives-used). Also in the contracts:
  collective purchasing (pools), signed job offers filled by competing
  leads (a Dutch auction on the lead fee), sealed bids, underwriter bonds,
  and paying checked work early through the lead's treasury (Job Note).
- Built in the window: code written since the announcement on 2026-10-19
  counts, submissions open 2026-11-01, and this repository's first commit
  is 2026-11-04 (`git log --reverse`). Every NEAR
  integration was written here; no code is carried over from an earlier
  project. The vendored NEAR Intents wasm is NEAR Foundation's, unmodified.
- Network: testnet and a local NEAR node only; all money is fake.
- Submission text, form fields and X post draft: [SUBMISSION.md](docs/SUBMISSION.md).

## Also by me
[Agent Exit](https://github.com/alexandergorokhov/near-transparent-agent-market)
sells AI agent businesses. Hire Tree is about agents hiring agents and
getting paid per job. Two independent projects (hackathon FAQ Q-015):
separate repos, no shared code.

## AI tools and libraries used
Claude Code wrote the code, tests and docs under the owner's direction.
No templates. Libraries (all open source, from crates.io):

| Where | Crates |
|---|---|
| Contracts | `near-sdk` 5.30 (with `unstable` for `env::ecrecover`), `near-contract-standards` (mock token), `borsh`, `serde_json`, `hex`, `sha2`, `sha3` |
| Tests | `proptest` (10,000-tree property test), `secp256k1` (test signer) |
| Agents | `near-api` 0.8, `near-sandbox` 0.4 (local node), `tokio`, `reqwest`, `serde`, `base64`, `secp256k1` |
| Build | `cargo-near` when installed, else plain `cargo` + `wasm-opt` (binaryen) |
| Local node only | NEAR Intents Verifier `defuse` 0.2.10 and wNEAR release wasm from github.com/near/intents (MIT), unmodified |

## License
MIT. See `LICENSE`.
