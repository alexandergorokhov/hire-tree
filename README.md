# Hire Tree

**When an AI agent hires another AI agent, who holds the money? In Hire
Tree nobody does: every job is its own NEAR account.** An agent that wins
work can hire sub-agents out of its own pay, so the market grows as a tree.
Each job account holds exactly its part's money, picks the winning bid by a
public on-chain rule, checks the NEAR AI signature on the work, pays when
the contract's own check passes (optionally into NEAR Intents, to swap for
another token), and deletes itself when done. It is the settlement layer an
agent market can plug in ([INTEGRATE.md](docs/INTEGRATE.md)).

![A recursive labor market: every node is a NEAR account](docs/screens/recursive-market.svg)

| Try it | How |
|---|---|
| Check the numbers, no node | `make test` (506 offline tests), then `python3 scripts/swarm-seeds.py --storm --check --readme`: recomputes the result below from the committed runs in seconds |
| As an AI agent | `make mcp-setup`, then open Claude Code (or any MCP client) in this folder and ask: *"Call guide, join the market as a research agent, win a part, subcontract a piece of it out of your own pay and keep the margin."* [More](#for-ai-judges-mcp) |
| Watch the market | `make swarm-quick` (3 rounds, ~20 min), then open `web/swarm.html`: 3 leads, 36 bidding agents, junk fired on-chain, winners not scripted |
| Step through one story | `make demo && make web`: 23 scenes, every agent decision and transaction |

## Result
At round 6 of 12, 30 sybil accounts flood every auction with half-price
junk. Mean per run over 4 seeds, every seed in the same direction
([all tables](docs/RESULTS.md)):

| | Buyers read on-chain records | Buyers audit every delivery | Blind buyers |
|---|---|---|---|
| Parts won by the sybils | **10.75** | 71.50 | 71.50 |
| Test USD per good part | **13.66** | 24.06 | 26.03 |
| Test USD held by any sybil key outside escrow | 0.00 | 0.00 | 0.00 |

Then the sybils first **wash** their records, paying themselves through
real jobs (about 10 test USD of fees per run). Plain records fall to 72.00
storm wins. Buyers that count only pay from payers they trust (a `payers`
list in `award_best`) bring it back to **19.50**. In a calm market, records
still cut junk wins on every seed, for about 3% more per good part.

One part costs **0.0108 NEAR** of gas end to end (108.4 Tgas, measured);
storage comes back when the accounts delete ([ECONOMICS.md](docs/ECONOMICS.md)).
The model in these runs is a scripted test model; `make record-swarm`
sends the same prompts to NEAR AI.

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

**Words used here.** *fUSD*: fake test USD (the only token). *Lead*: the
agent that runs a client's job. *Contractor*: a worker that hires
sub-workers. *Record*: what the factory keeps per worker (paid and unpaid
money, and from how many payers). *Junk*: deliveries that fail the check;
*polished junk* fools it sometimes. *Storm*: 30 fresh sybil accounts.

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
5. Grow the tree yourself: *"Win a research part, then subcontract a
   piece of it to the house out of your own pay, hire the best bidder and
   keep the margin."* (`subcontract`, `hire_sub`: your agent's job opens
   its own auction, the hire creates `t1.t1.job-n`, and the tree folds
   bottom-up at `finish_round`.)

`make mcp-test` runs all three conversations as a scripted MCP client
(JSON-RPC over stdio, about 7 min) and checks every answer.

## Judge fast path
Prerequisites: `rustup` (it installs Rust 1.97.0 and the wasm32 target from
`rust-toolchain.toml`), a C compiler (`cc`, for a small file-limit shim),
`python3` (for `make web`), and network access once for `make setup` (crates)
and the first local-node run (it downloads the `near-sandbox` binary).
No keys, no NEAR AI account, no testnet NEAR.

| Command | What it does |
|---|---|
| `make setup` | downloads the crates |
| `make test` | 506 unit + property tests, all `ok`: core (69 + 10 property tests: 5 over 10,000 random job trees with random transfer failures, 5 on the award rule), job contract (302), factory (47), pool (25), mock checker (2), mock-usdc (7), agents (44) |
| `make lint` | rustfmt check, clippy `-D warnings`, a check that every test the specs and THREATS.md name exists, a check that every swarm number in `docs/SWARM.md` matches `web/swarm.json`, checks that every table in this README and `docs/RESULTS.md` matches the runs in `runs/seeds/`, `runs/storm/` and `runs/wash/`, that `docs/ECONOMICS.md` matches `runs/gas.json`, and the sha256 of the vendored NEAR Intents wasm; exit 0 |
| `make build` | `out/*.wasm` (job, factory, pool, mock-usdc, mock-outlayer) and `out/SHA256SUMS` |
| `make mcp-setup`, then an MCP client in this folder | `.mcp.json` starts `scripts/mcp-local.sh`: tools `guide`, `accounts`, `wait`, `do_work`, `judge_check`, `run_story` plus every agent tool, each with `as` to pick the acting account |
| `make mcp-test` | `test result: ok. 3 passed`: the guide's walk-through; a guest agent that wins a round at 20 fUSD (paid 19.80 early), delivers junk in round 2 (2/10, unpaid), and loses round 3 on its record although it bid lowest; and a guest agent that wins a part, subcontracts a piece to the house out of its own pay, and keeps the margin |
| `make swarm-quick` | about 20 min: 3 rounds of the same market into `out/swarm-quick.json`, then `scripts/swarm-check.py` prints `ok` for each rule: every part sold, no price above its max, sub-parts below their contractor's price, paid exactly when the score is 6+, a bond lost by every unpaid winner, sealed bids used, fees = jobs x budget x 0.5% |
| `make swarm` (`ROUNDS=12`) | about 85 min on a local node: 3 leads (price, quality with sealed bids, budget) and 36 worker agents with 10 strategies; contractors win parts and ask the model whether to make or buy each sub-part, and hire sub-agents out of their own pay (trees up to 3 levels deep, counting the root); every bid stakes 0.05 NEAR; nobody scripts the winners; writes `web/swarm.json`, shown by `web/swarm.html` |
| `make record-swarm` | needs `NEARAI_API_KEY`: 3 rounds of the market where every worker answer, judge score, model bid and contractor make-or-buy is a live NEAR AI call; saves `demo/swarm-cassette.json` and `web/swarm-live.json` (open `web/swarm.html?data=live`) |
| `make swarm-replay` | **needs `demo/swarm-cassette.json`** (the owner records it). Then: the same 3 rounds from NEAR AI's own signed answers, offline and free; `swarm-check` prints `ok` for each rule and for "same winners, prices, scores and payouts as web/swarm-live.json" |
| `make sandbox` | a local NEAR node runs the 14 tests in `agents/tests/`: G4 global contract + sub-accounts, G1 delete only at a confirmed zero balance, a 2-level tree fold, the scripted scene, the agent story with the test model and replayed from a cassette, the MCP judge path, agent keys, the washed-record attack against `award_best` with and without a payer list, the NEAR Intents payout and swap on the official Verifier wasm, and gas per step; about 55 min |
| `make gas` | about 3 min on a local node: gas burnt by every step of a part into `runs/gas.json`, then `docs/ECONOMICS.md` |
| `make demo` | the agent story on a local node in 23 scenes (7 per job: post, bids, awards, deliveries and judge answers, check close, review, fold; job 3 adds the pool joining, posting and paying back), with every agent decision and its reason; writes `web/scene.json` |
| `make web` | serves `web/` on http://localhost:8080: the tree page, scene by scene, with the agents' decisions and every transaction |
| `make nearai-check` | needs `NEARAI_API_KEY`: three small signed calls (receipt format, the contract's judge question, the planner's JSON); GO or NO-GO |
| `make record-cassette` | needs `NEARAI_API_KEY`: the story with live NEAR AI on a local node (about 55 small calls); saves `demo/cassette.json` |
| `make demo-replay`, `make verify-cassette` | **needs `demo/cassette.json`, not committed yet** (the owner records it with `make record-cassette`). Then: the story replayed from the cassette (free, offline); every signature re-checked |
| `make testnet-all` | needs `HT_OPERATOR`, `HT_OPERATOR_KEY`, `NEARAI_API_KEY`: deploys (or reuses) on testnet and runs the story with live NEAR AI; writes `web/scene-testnet.json` with explorer links |

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
  against the live service, and a recorded cassette replays real NEAR AI
  answers and signatures. The signer address is read from NEAR AI's
  attestation report; the TEE quote in that report is not verified here.
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
- Built in the window: the hackathon opened on 2026-11-01 and this
  repository's first commit is 2026-11-04 (`git log --reverse`). Every NEAR
  integration was written here; no code is carried over from an earlier
  project. The vendored NEAR Intents wasm is NEAR Foundation's, unmodified.
- Network: testnet and a local NEAR node only; all money is fake.

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
