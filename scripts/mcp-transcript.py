#!/usr/bin/env python3
"""Write docs/MCP-TRANSCRIPT.md from a `make mcp-transcript` run.

    python3 scripts/mcp-transcript.py out/mcp-transcript.jsonl

Every line is one MCP tool call made by the scripted client in
agents/tests/judge.rs against the real MCP server and a local NEAR node.
"""
import json
import sys

TITLES = {
    "a_judge_walks_the_guide_over_mcp": "The guide's walk-through",
    "a_judges_agent_competes_in_the_open_market": "A guest agent competes in the open market, then cashes out through NEAR Intents",
    "a_judges_agent_becomes_a_contractor": "A guest agent becomes a contractor and hires a sub-agent",
    "the_contract_picks_the_winner_with_award_best": "The contract picks the winner with award_best",
}
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
out = ["# MCP transcript", "",
       "A recorded session of an AI agent using Hire Tree over MCP: every tool call",
       "and the server's full answer, from `make mcp-transcript` (the scripted",
       "client in `agents/tests/judge.rs`, the real MCP server, a local NEAR node,",
       "the scripted test model). Read it to see what your agent will get, without",
       "starting a node. Money is test USD (fUSD).", ""]
cur = None
for r in rows:
    t = r["test"].split("::")[-1]
    if t != cur:
        cur = t
        out += [f"## {TITLES.get(t, t)}", ""]
    res = r["result"]
    try:
        res = json.dumps(json.loads(res), indent=1)
    except Exception:
        pass
    if len(res) > 1500:
        res = res[:1500] + "\n... (cut)"
    out += [f"**`{r['tool']}`** `{json.dumps(r['args'])}`" + (" (error)" if r.get("error") else ""), "", "```json", res, "```", ""]
open("docs/MCP-TRANSCRIPT.md", "w").write("\n".join(out))
print(f"wrote docs/MCP-TRANSCRIPT.md: {len(rows)} calls")
