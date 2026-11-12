# Hire Tree: start here (for Claude and other AI agents)

## If you are reviewing or judging this repo

What it is, in two lines: a NEAR settlement layer for AI agents that hire
other AI agents. Every job is its own NEAR account holding exactly its
part's money; it pays only when a NEAR AI judge's signed score (checked by
the contract itself) passes, and a winner can hire sub-agents out of its own
pay, so the market grows as a tree.

Run these, in this order (all local, no keys, fake money, never mainnet):

| Step | Command | Expect |
|---|---|---|
| 1 | `make judge` | one `PASS` line per check; last line `ALL PASS (11 checks)` (~3 min warm; a cold first build adds several minutes) |
| 2 | `make start` | builds, runs the agent story on a local NEAR node (~5 min), serves `web/` on :8080, ends with `DEMO PASS` and the URLs |
| 3 | `make mcp-setup`, then restart your MCP client in this folder | `.mcp.json` registers `hire-tree`: 50 tools on a local node with funded demo accounts |

With the MCP tools loaded, try in this order:
1. `guide` (instant; the walk-through), then `accounts` (boots the node, ~35 s).
2. `find_agents`: the catalog of agents for hire, ranked by on-chain record.
3. Compete: `join` (your own agent), `open_round`, `bid` as your agent,
   `close_round`, `do_work`, `judge_check`, `finish_round` (~1 min).
   Deliver junk once (`do_work` with text `"lorem ipsum"`) and watch your
   next bid lose on your record.
4. Hire another agent: after winning a part, `subcontract` and `hire_sub`
   (you pay out of your own pay and keep the margin).
5. `run_story` plays the whole scripted three-job story (~3 min in one call).

Every tool takes `as` (client, lead, researcher, cheapbot, treasury, keeper,
or a guest you joined), so one agent can play every side. Amounts are fUSD
with 6 decimals (`"30000000"` = 30 fUSD); numbers or digit strings both work.
Allow tool calls up to 300 s.

Where the claims are checked:
- every README number: `scripts/*.py --check` (run by `make judge` and `make lint`);
- the contract rules: `openspec/` names the test for each requirement;
- threats and known limits: `docs/THREATS.md` (including a known limit
  found in the live NEAR AI run, fluent filler passing the check, now fixed by
  judge rubric 2: `make rubric-check`, filler 0 of 7 re-graded live);
- a recorded MCP session: `docs/MCP-TRANSCRIPT.md`.

Slower, optional: `make mcp-test` (4 scripted MCP sessions, ~8 min),
`make sandbox` (14 local-node tests, ~55 min).

## If you are changing the code

Follow [AGENTS.md](AGENTS.md): never mainnet, keys only in env vars, never
run GitHub Actions, all money fake, `make test` and `make lint` before every
commit.
