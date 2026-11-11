# Commands

Every `make` target, what it needs and what it shows. The short list is in the README's judge fast path.

| Command | What it does |
|---|---|
| `make setup` | downloads the crates |
| `make test` | 509 unit + property tests, all `ok`: core (69 + 10 property tests: 5 over 10,000 random job trees with random transfer failures, 5 on the award rule), job contract (305), factory (47), pool (25), mock checker (2), mock-usdc (7), agents (44) |
| `make lint` | rustfmt check, clippy `-D warnings`, a check that every test the specs and THREATS.md name exists, a check that every swarm number in `docs/SWARM.md` matches `web/swarm.json`, checks that every table in this README and `docs/RESULTS.md` matches the runs in `runs/seeds/`, `runs/storm/` and `runs/wash/`, that `docs/ECONOMICS.md` matches `runs/gas.json`, and the sha256 of the vendored NEAR Intents wasm; exit 0 |
| `make build` | `out/*.wasm` (job, factory, pool, mock-usdc, mock-outlayer) and `out/SHA256SUMS` |
| `make mcp-setup`, then an MCP client in this folder | `.mcp.json` starts `scripts/mcp-local.sh`: tools `guide`, `accounts`, `wait`, `do_work`, `judge_check`, `cash_out`, `run_story` plus every agent tool, each with `as` to pick the acting account |
| `make mcp-test` | `test result: ok. 4 passed`: the guide's walk-through; the contract picking the winner with `award_best`; a guest agent that routes its pay into the NEAR Intents Verifier, wins a round at 20 fUSD (paid 19.80 early) and cashes out to 3.96 wNEAR through a signed `token_diff` swap with the house solver, delivers junk in round 2 (2/10, unpaid), and loses round 3 on its record although it bid lowest; and a guest agent that wins a part, subcontracts a piece to the house out of its own pay, and keeps the margin |
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
| `make demo-replay`, `make verify-cassette` | uses the committed `demo/cassette.json` (54 live NEAR AI calls). The story replayed from the cassette (free, offline); every signature re-checked |
| `make testnet-all` | needs `HT_OPERATOR`, `HT_OPERATOR_KEY`, `NEARAI_API_KEY`: deploys (or reuses) on testnet and runs the story with live NEAR AI; writes `web/scene-testnet.json` with explorer links |
