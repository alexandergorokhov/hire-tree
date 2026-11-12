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
NEAR AI (103 transactions, 0 failed; links in the README and
`web/scene-testnet.json`).

## Tracks

AI Swarms (main), Agent Economy, AI Security & Defense.

## Other project (FAQ Q-015)

The same author also entered Agent Exit (https://github.com/alexandergorokhov/near-transparent-agent-market), a separate, independent project with no shared code.

## X post (owner posts it, with the video attached)

Rules (HACKATHON.md Procedure 2): say it was built at NEAR HACKATHON, tag @NEARProtocol and #NEARHackathon, attach the video or screenshots. The demo is local (`make web`), so the video is required (FAQ Q-012: screenshots alone are not enough when judges cannot open the demo). The post's URL is the form's `demoUrl`.

> Built at NEAR HACKATHON: Hire Tree, where AI agents hire AI agents on @NEARProtocol.
>
> Every job is its own NEAR account: it holds exactly its part's money, picks the winning bid by a public rule, checks the NEAR AI signature on the work, pays only what passes, then deletes itself.
>
> Code: https://github.com/alexandergorokhov/agent-hire
> #NEARHackathon @NearBuilders

## Form fields (owner submits; the form or `POST /api/applications`, exactly these fields)

| Field | Value |
|---|---|
| `projectName` | `Hire Tree` |
| `leadName` | owner's name |
| `email` | owner's email (max 2 submissions per email; Agent Exit is the other) |
| `size` | `1` |
| `tracks` | `["swarms", "agents", "security"]` |
| `nearAddress` | owner's **mainnet** payout account (`name.near` or 64-hex implicit) |
| `projectUrl` | `https://github.com/alexandergorokhov/agent-hire` (must be public by 2026-11-14 23:59 UTC) |
| `demoUrl` | URL of the X post above |
| `idea` | the text below (20-2000 characters) |
| `consent` | `true`, only after reading the contact notice |

Never fill `website` (spam trap). One submission per repository, and it cannot be edited.

`idea`:

> Hire Tree is a settlement layer for AI agents that hire other AI agents, on NEAR. When one agent splits a task and pays others for the parts, someone has to hold the money, and a subcontracting agent has no safe way to hand part of its budget to a sub-agent. In Hire Tree every job is its own NEAR sub-account running one global contract with no owner key. It holds exactly its part's money, takes bids on-chain and hires by a public rule that reads each bidder's on-chain record, checks the NEAR AI Cloud signature on the delivered work and on the contract's own quality question (env::ecrecover), pays only work that passes (optionally into NEAR Intents to swap the pay), fires junk and hires the runner-up, then pays out and deletes itself. A winner can hire sub-agents out of its own pay, so the market grows as a tree. In a simulated market of 36 workers, 30 sybil accounts flooding every auction with junk won 10.75 parts (mean of 4 seeds) when buyers read on-chain records, against 71.5 when they did not. A live NEAR AI run found that fluent filler fooled the judge 7 of 7 times; the contract's question now makes the judge quote the work first, and on the same deliveries filler passed 0 of 7, and in two more live probes filler written for the task passed 1 of 22 (that one named the steps asked for) while real answers passed 14 of 14. Live on NEAR testnet (hub.hiretree.testnet). Agents use it through an MCP server. No mainnet; all money is a fake test token.
