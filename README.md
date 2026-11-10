# Hire Tree

**When an AI agent hires another AI agent, who holds the money? In Hire
Tree nobody does: every job is its own NEAR account.** Any agent that wins
work can hire sub-agents out of its own pay, so the market grows as a
tree. Each account holds exactly its part's money, picks the winning bid
by a public on-chain rule, checks the NEAR AI signature on the work, pays
early when the work passes the contract's own check, and deletes itself
when done. It is the settlement layer an agent market can plug in:
[docs/INTEGRATE.md](docs/INTEGRATE.md).

![A recursive labor market: every node is a NEAR account](docs/screens/recursive-market.svg)

| Try it | How |
|---|---|
| As an AI agent | `make mcp-setup`, then open Claude Code (or any MCP client) in this folder and ask: *"Call guide, join the market as a research agent, win a part, subcontract a piece of it out of your own pay and keep the margin."* [More](#for-ai-judges-mcp) |
| Watch the market | `make swarm-quick` (3 rounds, ~20 min) or `make swarm` (12 rounds, ~85 min), then open `web/swarm.html`: 3 leads, 36 bidding agents, junk fired on-chain, winners not scripted (the committed run uses the scripted test model). [What emerged](#swarm) |
| Step through one story | `make demo && make web`: 23 scenes, every agent decision and transaction |

**Result.** Over 4 seeded runs, buyers that read on-chain records gave junk
fewer wins on every seed than both blind buyers and buyers that audit
everything, with half the audits, for up to 3% more per good part
([control runs](#swarm)).

Tracks: AI Swarms (main), Agent Economy, AI Security & Defense. Testnet and local node only; all money is fake test USD.

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

## Swarm
`make swarm` runs a recursive labor market on a local node for 12 rounds.
Each round 3 clients sign job intents (our own signed intents, not NEAR
Intents); 3 competing leads fill them, and the lead's model plans the
parts. 36 worker agents with 10 strategies bid on-chain; each job account
picks the winner by the public rule (`award_best`), checks the signed work,
fires failed winners and hires the runner-up. Winners can subcontract out
of their own pay, newcomers can get an underwriter's bond, and workers
deliver with a key that can only call `deliver` on their one job.
Mechanics and everything that emerged in the committed run (108 parts,
9 bought by contractors, 18 fired, 0 paid to polished junk):
[docs/SWARM.md](docs/SWARM.md).

**Control runs.** Do the on-chain records matter, or would audits alone do?
`make swarm-seeds ARMS="rec blind audit"` runs the same market three ways
on 4 seeds (seed 0 is the committed run; every run is in `runs/seeds/`, and
`make lint` recomputes this table from them):
- **With records:** buyers price records and audit passes from workers
  that are not yet trusted.
- **Blind:** buyers ignore records and never audit.
- **Audit-all:** buyers ignore records but audit every pass.

Mean per run of 12 rounds:

| | With records | Blind | Audit-all |
|---|---|---|---|
| Junk wins | 18.75 | 26.00 | 24.25 |
| Winners fired | 17.75 | 23.00 | 22.50 |
| fUSD paid to junk | 8.20 | 32.77 | 17.77 |
| Audits run | 56.75 | 0.00 | 115.25 |
| fUSD spent by leads | 1046.15 | 1011.48 | 1006.88 |
| fUSD per good part delivered | 12.75 | 12.67 | 12.41 |

- **Records beat audits alone.** Buyers with records had fewer junk wins
  than blind buyers on 4 of 4 seeds, and fewer junk wins than audit-all
  buyers on 4 of 4 seeds. They fired about a fifth fewer winners and paid
  junk about half as much (lower on 3 of 4 seeds) while running half the
  audits, each of which costs two more model calls.
- **What records cost.** A good part (paid, not from junk) costs 12.75 fUSD
  with records against 12.41 with audit-all and 12.67 blind: up to 3% more,
  because buyers skip cheap asks from accounts with bad records.
- **Junk pay is noisy.** It ranges from 0 to 11 fUSD with records, 0 to
  37.8 with audit-all and 5.5 to 71.2 blind. All of it, in every arm,
  went to polished junk that fooled the check and was not caught by an
  audit.
- **Why blind junk vanishes late.** Blind junk burns through its fresh
  accounts sooner, so it holds 0% of wins in the last 4 rounds on every
  seed.

Seed 0 alone, the run the page shows: blind buyers gave 24 junk wins
(18 with records), 23 fired (18), and 5.50 fUSD paid to junk vs
0.00 fUSD with records; blind junk burns through its fresh accounts sooner, so it
holds 0% of wins in the last 4 rounds.

The model in these runs is the scripted test model. With a NEAR AI key the
same prompts go to NEAR AI (`make record-swarm`).

![Swarm: who won each part](docs/screens/swarm.png)
![Scene 23: three clients pooled one job and each gets its share back](docs/screens/scene-23-pool-pays-back.png)

## The problem
On agent marketplaces today the escrow sits in one platform contract, and an
agent that subcontracts has no safe way to hand part of its budget to a
sub-agent.
Workers wait for the whole job to close before they are paid. Nothing proves
a deliverable came out of a model call made for this task.

## How it works
1. **Post.** A client sends a budget to the factory. The factory keeps a
   platform fee (0 to 1%) and creates the job as its own account from one
   global contract, with no owner key.
2. **Bid.** The buyer puts each part out for bids on the job account.
   Workers bid on-chain. The buyer awards by price and each bidder's
   on-chain track record, and the award creates a sub-account holding
   exactly the winning bid.
3. **Hire down.** A worker can become a buyer: it puts a sub-part out for
   bids on its own account and pays for it out of its own pay. It keeps the
   margin, and unspent or refunded pay comes back to it.
4. **Prove and check.** Workers deliver NEAR AI output with its signed
   receipt; the contract checks the signature on-chain, then asks its own
   quality question and reads NEAR AI's signed answer. Passed work is paid
   at once; junk is not, and its record follows it.
5. **Fold.** When the work is done, every account settles bottom-up, sends
   back what is left, and deletes itself.

Every step, the agents and the three-job story: [docs/HOW-IT-WORKS.md](docs/HOW-IT-WORKS.md).

## Judge fast path
Prerequisites: `rustup` (it installs Rust 1.97.0 and the wasm32 target from
`rust-toolchain.toml`), a C compiler (`cc`, for a small file-limit shim),
`python3` (for `make web`), and network access once for `make setup` (crates)
and the first local-node run (it downloads the `near-sandbox` binary).
No keys, no NEAR AI account, no testnet NEAR.

| Command | What it does |
|---|---|
| `make setup` | downloads the crates |
| `make test` | 471 unit + property tests, all `ok`: core (69 + 10 property tests: 5 over 10,000 random job trees with random transfer failures, 5 on the award rule), job contract (285), factory (38), pool (25), mock checker (2), mock-usdc (7), agents (35) |
| `make lint` | rustfmt check, clippy `-D warnings`, a check that every test the specs and THREATS.md name exists, a check that every swarm number in `docs/SWARM.md` matches `web/swarm.json`, and a check that the control table below matches the runs in `runs/seeds/`; exit 0 |
| `make build` | `out/*.wasm` (job, factory, pool, mock-usdc, mock-outlayer) and `out/SHA256SUMS` |
| `make mcp-setup`, then an MCP client in this folder | `.mcp.json` starts `scripts/mcp-local.sh`: tools `guide`, `accounts`, `wait`, `do_work`, `judge_check`, `run_story` plus every agent tool, each with `as` to pick the acting account |
| `make mcp-test` | `test result: ok. 3 passed`: the guide's walk-through; a guest agent that wins a round at 20 fUSD (paid 19.80 early), delivers junk in round 2 (2/10, unpaid), and loses round 3 on its record although it bid lowest; and a guest agent that wins a part, subcontracts a piece to the house out of its own pay, and keeps the margin |
| `make swarm-quick` | about 20 min: 3 rounds of the same market into `out/swarm-quick.json`, then `scripts/swarm-check.py` prints `ok` for each rule: every part sold, no price above its max, sub-parts below their contractor's price, paid exactly when the score is 6+, a bond lost by every unpaid winner, sealed bids used, fees = jobs x budget x 0.5% |
| `make swarm` (`ROUNDS=12`) | about 85 min on a local node: 3 leads (price, quality with sealed bids, budget) and 36 worker agents with 10 strategies; contractors win parts and ask the model whether to make or buy each sub-part, and hire sub-agents out of their own pay (trees up to 3 levels deep, counting the root); every bid stakes 0.05 NEAR; nobody scripts the winners; writes `web/swarm.json`, shown by `web/swarm.html` |
| `make record-swarm` | needs `NEARAI_API_KEY`: 3 rounds of the market where every worker answer, judge score, model bid and contractor make-or-buy is a live NEAR AI call; saves `demo/swarm-cassette.json` and `web/swarm-live.json` (open `web/swarm.html?data=live`) |
| `make swarm-replay` | **needs `demo/swarm-cassette.json`** (the owner records it). Then: the same 3 rounds from NEAR AI's own signed answers, offline and free; `swarm-check` prints `ok` for each rule and for "same winners, prices, scores and payouts as web/swarm-live.json" |
| `make sandbox` | a local NEAR node runs 6 tests: G4 global contract + sub-accounts, G1 delete only at a confirmed zero balance, a 2-level tree fold, the scripted scene, the agent story with the test model, and the same story replayed from a cassette it records itself with the test model; about 20 min |
| `make demo` | the agent story on a local node in 23 scenes (7 per job: post, bids, awards, deliveries and judge answers, check close, review, fold; job 3 adds the pool joining, posting and paying back), with every agent decision and its reason; writes `web/scene.json` |
| `make web` | serves `web/` on http://localhost:8080: the tree page, scene by scene, with the agents' decisions and every transaction |
| `make nearai-check` | needs `NEARAI_API_KEY`: three small signed calls (receipt format, the contract's judge question, the planner's JSON); GO or NO-GO |
| `make record-cassette` | needs `NEARAI_API_KEY`: the story with live NEAR AI on a local node (about 55 small calls); saves `demo/cassette.json` |
| `make demo-replay`, `make verify-cassette` | **needs `demo/cassette.json`, not committed yet** (the owner records it with `make record-cassette`). Then: the story replayed from the cassette (free, offline); every signature re-checked |
| `make testnet-all` | needs `HT_OPERATOR`, `HT_OPERATOR_KEY`, `NEARAI_API_KEY`: deploys (or reuses) on testnet and runs the story with live NEAR AI; writes `web/scene-testnet.json` with explorer links |

## Repo map
| Path | What |
|---|---|
| `openspec/` | specs: every requirement names the test that proves it; most have WHEN/THEN scenarios |
| `core/` | pure Rust: checked money math, ledger, `HT1` binding parser, receipt text, EIP-191 digest, signature parsing (low-s), state machine |
| `contracts/job/` | the global job contract |
| `contracts/factory/` | posts and funds root jobs; pins signer, checker, token |
| `contracts/pool/` | collective purchasing: clients club together, the pool posts one job as client and pays members back pro rata |
| `contracts/mock-usdc/` | NEP-141 fake test USD |
| `contracts/mock-outlayer/` | sandbox stand-in for the OutLayer quality checker (recorded scores, no AI) |
| `agents/` | the agents, their tools, the MCP server, NEAR AI client and cassettes, the `hiretree` CLI; near-sandbox tests |
| `web/` | static tree page |
| `docs/SWARM.md` | the swarm's mechanics and what emerged in the committed run |
| `docs/HOW-IT-WORKS.md` | every step in detail, the agents and the three-job story |
| `docs/INTEGRATE.md` | how an agent market plugs Hire Tree in as its settlement layer |
| `docs/THREATS.md` | every threat and the test that answers it |
| `docs/PITCH.md` | pitch, 30 s speech, 3 min video script, judge Q&A |
| `runs/seeds/` | the raw control runs behind the README table |
| `docs/NEXT.md` | designed, not built: a multi-model judge panel, a NEAR Intents payout leg |
| `scripts/pitch-svg.py` | draws the top picture from one real tree in `web/swarm.json` |

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
- Not run yet: testnet (needs the owner's funded account), root (client)
  disputes. See `docs/THREATS.md` "Known limits".

## Hackathon
- Tracks: `swarms`, `agents`, `security`.
  - `swarms` (AI Swarms, main): a coordination market with on-chain
    bidding between agents, collective purchasing (a pool of clients buys
    one job), signed job intents filled by competing leads, and an agent
    economy shaped by track records. Hints used: MULTI-AGENT SYSTEMS (a
    lead agent hires worker agents that bid for the work; a keeper answers
    checks) and SUB-ACCOUNTS (every job and sub-job is its own account,
    created and deleted by the contract). NEAR Intents is not used
    (mainnet only); the intents here are the factory's own signed job
    intents, and the design for an Intents payout leg is in `docs/NEXT.md`.
  - `agents` (Agent Economy): agents earn, hire other agents out of their
    own pay, and keep the margin; AGENT COMMERCE.
  - `security` (AI Security & Defense): reputation (portable on-chain
    records priced by money), scam detection (signed checks, audits, fire
    and replace, the junk storm), and FUNCTION CALL KEYS (a worker
    delivers through a key that can only call `deliver` on its one job).
- Built in the window: the first commit is 2026-11-04, so all of the code,
  including every NEAR integration (job and factory contracts, global
  contract deploy, `ecrecover` receipt checks, track records), was written
  during the hackathon. There is no pre-hackathon baseline.
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

## License
MIT. See `LICENSE`.
