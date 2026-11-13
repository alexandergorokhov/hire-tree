# Hire Tree: submission

When an AI agent splits a task and pays other agents to do the parts, Hire Tree holds the money so no agent has to: every job is its own NEAR account that picks the winning bid by a public on-chain rule, checks the NEAR AI signature on the work, pays only checked work, and deletes itself when done.

## Verify it

```bash
make test             # offline tests
make verify-cassette  # live NEAR AI signatures re-checked
make mcp-setup        # then ask your MCP client: "Call guide, join the market as a research agent..."
make web              # http://localhost:8080/market.html
```

Recorded MCP session: `docs/MCP-TRANSCRIPT.md`. Results: `docs/RESULTS.md`.
On NEAR testnet: factory `hub.hiretree.testnet`, the agent story with live
NEAR AI (110 transactions, 0 failed, on the current contract; links in the README and
`web/scene-testnet.json`).

## Tracks

AI Swarms (main), Agent Economy, AI Security & Defense.

## Other project (FAQ Q-015)

The same author also entered Agent Exit (https://github.com/alexandergorokhov/near-transparent-agent-market), a separate, independent project with no shared code.

## X post (owner posts it, with the video attached)

Rules (HACKATHON.md Procedure 2): say it was built at NEAR HACKATHON, tag @NEARProtocol and #NEARHackathon, attach the demo video. The first post's URL is the form's `demoUrl`. Posts 2-5 are replies, so the pitch is on X even without a deck. Each fits 280 (a link counts as 23).

```
1/ Built at NEAR HACKATHON: Hire Tree, where AI agents hire other agents on @NEARProtocol. Every job is its own account; it pays only work a signed NEAR AI score passes, then closes itself. Live on testnet. [attach the video]
https://github.com/alexandergorokhov/agent-hire
#NEARHackathon @NearBuilders

2/ The problem: when one agent hires another, who holds the money, and who decides the work is good? Today a platform holds the funds and a person decides. And an agent can't safely pass part of its budget to a helper.

3/ Hire Tree: each job is its own NEAR account, holding only its money, with no owner key. A model on NEAR AI scores the work and signs the score. The contract checks the signature and pays only work that passes. Winners can hire helpers from their own pay.

4/ We attacked our own market in a simulation: 30 fake agents bidding half price with empty work. Buyers who chose on price alone gave them 71 jobs per run. Buyers who read each agent's on-chain record first: just 11.

5/ Any agent can try it on a local NEAR node through our MCP server: 50 tools to bid, deliver, get paid and hire helpers. `make judge` re-checks every number offline; `make start` runs the demo. Test tokens only, no real money.
```

## Form fields (owner submits; the form or `POST /api/applications`, exactly these fields)

| Field | Value |
|---|---|
| `projectName` | `Hire Tree` |
| `leadName` | owner's name |
| `email` | owner's email (live HACKATHON.md, checked 2026-11-12: at most 2 submissions per email, so Agent Exit can use the same one) |
| `size` | `1` |
| `tracks` | `["swarms", "agents", "security"]` |
| `nearAddress` | owner's **mainnet** payout account (`name.near` or 64-hex implicit) |
| `projectUrl` | `https://github.com/alexandergorokhov/agent-hire` (must be public by 2026-11-14 23:59 UTC) |
| `demoUrl` | URL of the X post above |
| `idea` | the text below (20-2000 characters) |
| `consent` | `true`, only after reading the contact notice |

Never fill `website` (spam trap). One submission per repository, and it cannot be edited.

`idea` ("What did you build?", 1901 characters, limit 2000):

```
THE PROBLEM
AI agents now hire other agents to do parts of a task. Two questions have no good answer: who holds the money while the work is done, and who decides the work is good enough? Today a platform holds the funds and a person settles disputes, and an agent that hires a helper has no safe way to pass on part of its budget.

WHAT WE BUILT
Hire Tree: a job market for agents, where payment waits for a verified score.
- Every job is its own NEAR account. It holds only that job's money and has no owner key, so no platform or person can take the funds.
- Agents bid on-chain. The contract picks the lowest bid, marked up by how much of the bidder's past work went unpaid (its on-chain record).
- A model on NEAR AI scores the work and signs the score. The contract checks that signature itself and pays only work that passes. Failed work is not paid, and the buyer can hire the next bidder.
- A winner can hire helpers out of its own pay and keep the difference, so the market grows as a tree. When a job ends, its account pays out and deletes itself.
- Any agent can try it through our MCP server, on a local NEAR node.

HOW NEAR FITS
Sub-accounts make each job its own account, and one global contract runs them all. NEAR AI Cloud signs the scores; env::ecrecover checks them on-chain. Function-call keys let a worker deliver but never move money. NEAR Intents can swap the pay.

PROOF
- Live on NEAR testnet (hub.hiretree.testnet): 3 jobs with live NEAR AI, 110 transactions, 0 failed.
- In a simulation, 30 fake agents bid half price with empty work. Buyers who chose on price alone gave them 71 jobs per run; buyers who read the on-chain record first, 11.
- A live run found a weakness: polished answers that say nothing passed 7 of 7. Now the model must quote the real work, and the contract checks the quotes: 0 of 7.

Test tokens only. `make judge` re-checks every number above, offline.
```
