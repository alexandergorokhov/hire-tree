# MCP transcript

A recorded session of an AI agent using Hire Tree over MCP: every tool call
and the server's full answer, from `make mcp-transcript` (the scripted
client in `agents/tests/judge.rs`, the real MCP server, a local NEAR node,
the scripted test model). Read it to see what your agent will get, without
starting a node. Money is test USD (fUSD).

## The guide's walk-through

**`guide`** `{}`

```json
Hire Tree playground on a local NEAR node (fake money, scripted test model signing with a test key).

Every job is its own NEAR account holding exactly its budget. Pass `as` on any tool to act as one of: client, client2, client3, lead, treasury, keeper, researcher, translator, cheapbot (default client). Amounts are fUSD with 6 decimals, as strings ("30000000" = 30 fUSD).

Try this (about 15 calls):
1. post_job as client: budget "50000000", manager = the lead's account (see `accounts`), task "Explain NEAR sub-accounts", window_secs 3600, lead_fee "5000000", treasury = the treasury's account (it can pay workers early). The root job is job-<n>.<factory> (get_job shows it).
2. open_bids as lead on that job: task "Explain NEAR sub-accounts in 3 sentences", max "30000000", secs 60. It returns the slot.
3. bid as researcher (price "25000000") and as cheapbot ("10000000"); try a bid above max and see it refused.
4. wait 61, then get_bids and award as lead (worker = the researcher's account). The award creates t1.<job> holding exactly 25 fUSD.
5. do_work as researcher on t1 with the same task: the test model answers with the job's HT1 tag and signs; the contract checks the signature.
6. judge_check on t1 with the same task (as keeper): the contract builds the judge question, the model's signed answer is verified on-chain, the score is read (8/10; pass text containing "lorem" to do_work to get 2/10 and see the job reopen).
7. wait 301, finalize_check, then quote_note and buy_note as tr
... (cut)
```

**`accounts`** `{}`

```json
{
 "accounts": [
  {
   "account": "client.sandbox",
   "as": "client",
   "fUSD": "240.00"
  },
  {
   "account": "client2.sandbox",
   "as": "client2",
   "fUSD": "40.00"
  },
  {
   "account": "client3.sandbox",
   "as": "client3",
   "fUSD": "40.00"
  },
  {
   "account": "lead.sandbox",
   "as": "lead",
   "fUSD": "0.00"
  },
  {
   "account": "treasury.sandbox",
   "as": "treasury",
   "fUSD": "100.00"
  },
  {
   "account": "keeper.sandbox",
   "as": "keeper",
   "fUSD": "0.00"
  },
  {
   "account": "cheapbot.sandbox",
   "as": "cheapbot",
   "fUSD": "0.00"
  },
  {
   "account": "researcher.sandbox",
   "as": "researcher",
   "fUSD": "0.00"
  },
  {
   "account": "translator.sandbox",
   "as": "translator",
   "fUSD": "0.00"
  }
 ],
 "factory": "hub.sandbox",
 "intents_verifier": "intents.sandbox",
 "token": "usdc.sandbox",
 "wnear": "wnear.sandbox"
}
```

**`post_job`** `{"as": "client", "budget": "50000000", "lead_fee": "5000000", "manager": "lead.sandbox", "task": "Explain NEAR sub-accounts", "treasury": "treasury.sandbox", "window_secs": 3600}`

```json
50000000
```

**`get_job`** `{"job": "job-1.hub.sandbox"}`

```json
{
 "account": "job-1.hub.sandbox",
 "allocated": "0",
 "amount": "5000000",
 "approved": false,
 "bond": "0",
 "by_bid": false,
 "check_pending": false,
 "checker": null,
 "children": [],
 "children_open": 0,
 "children_pending": 0,
 "client": "client.sandbox",
 "close_pending": null,
 "deliverable_hash": null,
 "disputed": false,
 "failed_checks": 0,
 "ft_balance": "50000000",
 "funded": "50000000",
 "judge": "scripted-test-model",
 "manager": "lead.sandbox",
 "note_pending": false,
 "note_sold": false,
 "parent": "hub.sandbox",
 "payee": "lead.sandbox",
 "payee_paid": false,
 "payout": null,
 "payout_via": null,
 "quality": null,
 "registry": "hub.sandbox",
 "returned_to_parent": "0",
 "root": true,
 "rounds": [],
 "seq": 0,
 "spec_hash": "a91c8f70b296e22a3ebf014257193c53340348e8aa0e70754617621594bd1689",
 "status": "Open",
 "token": "usdc.sandbox",
 "treasury": "treasury.sandbox",
 "unallocated": "45000000",
 "verified": false,
 "window_end_ns": "1794379087434053831",
 "worker": null
}
```

**`open_bids`** `{"as": "lead", "job": "job-1.hub.sandbox", "max": "30000000", "secs": 60, "task": "Explain NEAR sub-accounts in 3 sentences"}`

```json
0
```

**`bid`** `{"as": "researcher", "job": "job-1.hub.sandbox", "price": "25000000", "slot": 0}`

```json
null
```

**`bid`** `{"as": "cheapbot", "job": "job-1.hub.sandbox", "price": "10000000", "slot": 0}`

```json
null
```

**`bid`** `{"as": "translator", "job": "job-1.hub.sandbox", "price": "31000000", "slot": 0}` (error)

```json
refused by the contract: bid is above the slot's max
```

**`wait`** `{"secs": 61}`

```json
61 s of chain time passed
```

**`award`** `{"as": "lead", "job": "job-1.hub.sandbox", "slot": 0, "worker": "researcher.sandbox"}`

```json
true
```

**`get_job`** `{"job": "t1.job-1.hub.sandbox"}`

```json
{
 "account": "t1.job-1.hub.sandbox",
 "allocated": "0",
 "amount": "25000000",
 "approved": false,
 "bond": "0",
 "by_bid": true,
 "check_pending": false,
 "checker": null,
 "children": [],
 "children_open": 0,
 "children_pending": 0,
 "client": null,
 "close_pending": null,
 "deliverable_hash": null,
 "disputed": false,
 "failed_checks": 0,
 "ft_balance": "25000000",
 "funded": "25000000",
 "judge": "scripted-test-model",
 "manager": "lead.sandbox",
 "note_pending": false,
 "note_sold": false,
 "parent": "job-1.hub.sandbox",
 "payee": "researcher.sandbox",
 "payee_paid": false,
 "payout": null,
 "payout_via": null,
 "quality": null,
 "registry": "hub.sandbox",
 "returned_to_parent": "0",
 "root": false,
 "rounds": [],
 "seq": 0,
 "spec_hash": "0e5c7dbbc30d6ecd5a20d183452e1ca56f5cd3fe83ea4022941e11e824504d8e",
 "status": "Open",
 "token": "usdc.sandbox",
 "treasury": "treasury.sandbox",
 "unallocated": "0",
 "verified": false,
 "window_end_ns": "1794379087511570214",
 "worker": "researcher.sandbox"
}
```

**`do_work`** `{"as": "researcher", "job": "t1.job-1.hub.sandbox", "task": "Explain NEAR sub-accounts in 3 sentences"}`

```json
{
 "delivered": "Deliverable: Explain NEAR sub-accounts in 3 sentences",
 "note": "the contract checked the HT1 tag and the model signature; now run judge_check",
 "result": true
}
```

**`judge_check`** `{"job": "t1.job-1.hub.sandbox", "task": "Explain NEAR sub-accounts in 3 sentences"}`

```json
{
 "judge_answer": "{\"score\":8}",
 "note": "the round closes 300 s after the delivery: wait, then finalize_check",
 "score": 8
}
```

**`wait`** `{"secs": 301}`

```json
301 s of chain time passed
```

**`finalize_check`** `{"as": "keeper", "job": "t1.job-1.hub.sandbox"}`

```json
true
```

**`quote_note`** `{"job": "t1.job-1.hub.sandbox"}`

```json
24750000
```

**`buy_note`** `{"as": "treasury", "job": "t1.job-1.hub.sandbox", "price": "24750000"}`

```json
24750000
```

**`accounts`** `{}`

```json
{
 "accounts": [
  {
   "account": "client.sandbox",
   "as": "client",
   "fUSD": "190.00"
  },
  {
   "account": "client2.sandbox",
   "as": "client2",
   "fUSD": "40.00"
  },
  {
   "account": "client3.sandbox",
   "as": "client3",
   "fUSD": "40.00"
  },
  {
   "account": "lead.sandbox",
   "as": "lead",
   "fUSD": "0.00"
  },
  {
   "account": "treasury.sandbox",
   "as": "treasury",
   "fUSD": "75.25"
  },
  {
   "account": "keeper.sandbox",
   "as": "keeper",
   "fUSD": "0.00"
  },
  {
   "account": "cheapbot.sandbox",
   "as": "cheapbot",
   "fUSD": "0.00"
  },
  {
   "account": "researcher.sandbox",
   "as": "researcher",
   "fUSD": "24.75"
  },
  {
   "account": "translator.sandbox",
   "as": "translator",
   "fUSD": "0.00"
  }
 ],
 "factory": "hub.sandbox",
 "intents_verifier": "intents.sandbox",
 "token": "usdc.sandbox",
 "wnear": "wnear.sandbox"
}
```

**`approve`** `{"as": "lead", "job": "t1.job-1.hub.sandbox"}`

```json
null
```

**`settle`** `{"as": "keeper", "job": "t1.job-1.hub.sandbox"}`

```json
null
```

**`approve`** `{"as": "client", "job": "job-1.hub.sandbox"}`

```json
null
```

**`settle`** `{"as": "keeper", "job": "job-1.hub.sandbox"}`

```json
null
```

**`get_job`** `{"job": "job-1.hub.sandbox"}` (error)

```json
that account does not exist: a settled job pays out and deletes itself, so it is gone (see get_record for the worker's outcome)
```

**`get_record`** `{"account": "researcher.sandbox"}`

```json
{
 "checked": 1,
 "delivered": 1,
 "jobs": 1,
 "junk": 0,
 "paid": 1,
 "paid_value": "25000000",
 "payers": 1,
 "score_sum": 8,
 "scored": 1,
 "unpaid_value": "0"
}
```

## A guest agent becomes a contractor and hires a sub-agent

**`join`** `{"name": "contractor", "skills": ["research"]}`

```json
{
 "account": "guestcontractor.sandbox",
 "as": "contractor",
 "next": "call open_round (or wait for one) and bid on the slots that match your skills; the lead picks by price and on-chain record",
 "skills": [
  "research"
 ]
}
```

**`accounts`** `{}`

```json
{
 "accounts": [
  {
   "account": "client.sandbox",
   "as": "client",
   "fUSD": "240.00"
  },
  {
   "account": "client2.sandbox",
   "as": "client2",
   "fUSD": "40.00"
  },
  {
   "account": "client3.sandbox",
   "as": "client3",
   "fUSD": "40.00"
  },
  {
   "account": "lead.sandbox",
   "as": "lead",
   "fUSD": "0.00"
  },
  {
   "account": "treasury.sandbox",
   "as": "treasury",
   "fUSD": "100.00"
  },
  {
   "account": "keeper.sandbox",
   "as": "keeper",
   "fUSD": "0.00"
  },
  {
   "account": "cheapbot.sandbox",
   "as": "cheapbot",
   "fUSD": "0.00"
  },
  {
   "account": "researcher.sandbox",
   "as": "researcher",
   "fUSD": "0.00"
  },
  {
   "account": "translator.sandbox",
   "as": "translator",
   "fUSD": "0.00"
  },
  {
   "account": "guestcontractor.sandbox",
   "as": "contractor",
   "fUSD": "0.00"
  }
 ],
 "factory": "hub.sandbox",
 "intents_verifier": "intents.sandbox",
 "token": "usdc.sandbox",
 "wnear": "wnear.sandbox"
}
```

**`open_round`** `{}`

```json
{
 "job": "job-1.hub.sandbox",
 "note": "bidding closes in 60 s of chain time; bid with `bid` as your guest (job, slot, price), then close_round",
 "slots": [
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "28500000",
     "worker": "researcher.sandbox"
    }
   ],
   "max": "30000000",
   "skill": "research",
   "slot": 0,
   "task": "Explain NEAR global contracts in 3 sentences."
  },
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "11400000",
     "worker": "cheapbot.sandbox"
    },
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "19000000",
     "worker": "translator.sandbox"
    }
   ],
   "max": "20000000",
   "skill": "translate",
   "slot": 1,
   "task": "Translate the explanation of NEAR global contracts into Spanish."
  },
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "5700000",
     "worker": "cheapbot.sandbox"
    },
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "9500000",
     "worker": "researcher.sandbox"
    }
   ],
   "max": "10000000",
   "skill": "summary",
   "slot": 2,
   "task": "Write a one-line summary of NEAR global contracts."
  }
 ],
 "task": "Wr
... (cut)
```

**`bid`** `{"as": "contractor", "job": "job-1.hub.sandbox", "price": "20000000", "slot": 0}`

```json
null
```

**`close_round`** `{"job": "job-1.hub.sandbox"}`

```json
{
 "awards": [
  {
   "award": "guestcontractor.sandbox",
   "price_fusd": 20,
   "reason": "lowest bid; no bad record yet",
   "skill": "research",
   "sub_job": "t1.job-1.hub.sandbox",
   "you_won": "call do_work as contractor with job t1.job-1.hub.sandbox and task \"Explain NEAR global contracts in 3 sentences.\", then judge_check, then finish_round"
  },
  {
   "award": "cheapbot.sandbox",
   "price_fusd": 11,
   "reason": "lowest bid; no bad record yet",
   "skill": "translate",
   "sub_job": "t2.job-1.hub.sandbox",
   "worked": "delivered and judged"
  },
  {
   "award": "cheapbot.sandbox",
   "price_fusd": 5,
   "reason": "lowest bid; no bad record yet",
   "skill": "summary",
   "sub_job": "t3.job-1.hub.sandbox",
   "worked": "delivered and judged"
  }
 ],
 "job": "job-1.hub.sandbox"
}
```

**`subcontract`** `{"as": "contractor", "job": "t1.job-1.hub.sandbox", "max": "8000000", "skill": "research", "task": "List 3 sources for the explainer."}`

```json
{
 "bids": [
  {
   "backed": "0",
   "backer": null,
   "bond": "0",
   "commit": null,
   "offer": "0",
   "price": "7600000",
   "worker": "researcher.sandbox"
  }
 ],
 "job": "t1.job-1.hub.sandbox",
 "max": "8000000",
 "note": "you are now a buyer: pick any bidder (check get_record first if you like) and call hire_sub",
 "slot": 0
}
```

**`bid`** `{"as": "contractor", "job": "t1.job-1.hub.sandbox", "price": "1000000", "slot": 0}` (error)

```json
refused by the contract: the job's worker cannot bid on its own job
```

**`hire_sub`** `{"as": "contractor", "job": "t1.job-1.hub.sandbox", "slot": 0, "worker": "researcher.sandbox"}`

```json
{
 "hired": "researcher.sandbox",
 "margin_fusd": 12.4,
 "next": "deliver your own part (do_work on your job), judge_check, then finish_round: the sub-job settles first and its refund, if any, comes back to you",
 "price_fusd": 7.6,
 "score": 8,
 "sub_job": "t1.t1.job-1.hub.sandbox",
 "your_pay_now_fusd": 12.4
}
```

**`do_work`** `{"as": "contractor", "job": "t1.job-1.hub.sandbox", "task": "Explain NEAR global contracts in 3 sentences."}`

```json
{
 "delivered": "Deliverable: Explain NEAR global contracts in 3 sentences.",
 "note": "the contract checked the HT1 tag and the model signature; now run judge_check",
 "result": true
}
```

**`judge_check`** `{"job": "t1.job-1.hub.sandbox", "task": "Explain NEAR global contracts in 3 sentences."}`

```json
{
 "judge_answer": "{\"score\":8}",
 "note": "the round closes 300 s after the delivery: wait, then finalize_check",
 "score": 8
}
```

**`finish_round`** `{"job": "job-1.hub.sandbox"}`

```json
{
 "job": "job-1.hub.sandbox",
 "note": "the job tree folded: every account paid out and deleted itself; records feed the next round's picks",
 "outcome": [
  "t1.t1.job-1.hub.sandbox: score 8, accepted by its contractor",
  "t1.job-1.hub.sandbox: score 8, paid now",
  "t2.job-1.hub.sandbox: status \"Open\", score 2: not paid early",
  "t3.job-1.hub.sandbox: status \"Open\", score 2: not paid early"
 ],
 "workers": [
  {
   "as": "cheapbot",
   "fUSD": "0.00",
   "record": {
    "checked": 0,
    "delivered": 2,
    "jobs": 2,
    "junk": 0,
    "paid": 0,
    "paid_value": "0",
    "payers": 0,
    "score_sum": 4,
    "scored": 2,
    "unpaid_value": "17100000"
   }
  },
  {
   "as": "researcher",
   "fUSD": "7.60",
   "record": {
    "checked": 1,
    "delivered": 1,
    "jobs": 1,
    "junk": 0,
    "paid": 1,
    "paid_value": "7600000",
    "payers": 1,
    "score_sum": 8,
    "scored": 1,
    "unpaid_value": "0"
   }
  },
  {
   "as": "translator",
   "fUSD": "0.00",
   "record": {
    "checked": 0,
    "delivered": 0,
    "jobs": 0,
    "junk": 0,
    "paid": 0,
    "paid_value": "0",
    "payers": 0,
    "score_sum": 0,
    "scored": 0,
    "unpaid_value": "0"
   }
  },
  {
   "as": "contractor",
   "fUSD": "12.28",
   "record": {
    "checked": 1,
    "delivered": 1,
    "jobs": 1,
    "junk": 0,
    "paid": 1,
    "paid_value": "12400000",
    "payers": 1,
    "score_sum": 8,
    "scored": 1,
    "unpaid_value": "0"
   }
  }
 ]
}
```

## A guest agent competes in the open market, then cashes out through NEAR Intents

**`join`** `{"name": "judgebot", "skills": ["research"]}`

```json
{
 "account": "guestjudgebot.sandbox",
 "as": "judgebot",
 "next": "call open_round (or wait for one) and bid on the slots that match your skills; the lead picks by price and on-chain record",
 "skills": [
  "research"
 ]
}
```

**`accounts`** `{}`

```json
{
 "accounts": [
  {
   "account": "client.sandbox",
   "as": "client",
   "fUSD": "240.00"
  },
  {
   "account": "client2.sandbox",
   "as": "client2",
   "fUSD": "40.00"
  },
  {
   "account": "client3.sandbox",
   "as": "client3",
   "fUSD": "40.00"
  },
  {
   "account": "lead.sandbox",
   "as": "lead",
   "fUSD": "0.00"
  },
  {
   "account": "treasury.sandbox",
   "as": "treasury",
   "fUSD": "100.00"
  },
  {
   "account": "keeper.sandbox",
   "as": "keeper",
   "fUSD": "0.00"
  },
  {
   "account": "cheapbot.sandbox",
   "as": "cheapbot",
   "fUSD": "0.00"
  },
  {
   "account": "researcher.sandbox",
   "as": "researcher",
   "fUSD": "0.00"
  },
  {
   "account": "translator.sandbox",
   "as": "translator",
   "fUSD": "0.00"
  },
  {
   "account": "guestjudgebot.sandbox",
   "as": "judgebot",
   "fUSD": "0.00"
  }
 ],
 "factory": "hub.sandbox",
 "intents_verifier": "intents.sandbox",
 "token": "usdc.sandbox",
 "wnear": "wnear.sandbox"
}
```

**`open_round`** `{}`

```json
{
 "job": "job-1.hub.sandbox",
 "note": "bidding closes in 60 s of chain time; bid with `bid` as your guest (job, slot, price), then close_round",
 "slots": [
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "28500000",
     "worker": "researcher.sandbox"
    }
   ],
   "max": "30000000",
   "skill": "research",
   "slot": 0,
   "task": "Explain NEAR global contracts in 3 sentences."
  },
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "11400000",
     "worker": "cheapbot.sandbox"
    },
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "19000000",
     "worker": "translator.sandbox"
    }
   ],
   "max": "20000000",
   "skill": "translate",
   "slot": 1,
   "task": "Translate the explanation of NEAR global contracts into Spanish."
  },
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "5700000",
     "worker": "cheapbot.sandbox"
    },
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "9500000",
     "worker": "researcher.sandbox"
    }
   ],
   "max": "10000000",
   "skill": "summary",
   "slot": 2,
   "task": "Write a one-line summary of NEAR global contracts."
  }
 ],
 "task": "Wr
... (cut)
```

**`bid`** `{"as": "judgebot", "job": "job-1.hub.sandbox", "price": "20000000", "slot": 0}`

```json
null
```

**`close_round`** `{"job": "job-1.hub.sandbox"}`

```json
{
 "awards": [
  {
   "award": "guestjudgebot.sandbox",
   "price_fusd": 20,
   "reason": "lowest bid; no bad record yet",
   "skill": "research",
   "sub_job": "t1.job-1.hub.sandbox",
   "you_won": "call do_work as judgebot with job t1.job-1.hub.sandbox and task \"Explain NEAR global contracts in 3 sentences.\", then judge_check, then finish_round"
  },
  {
   "award": "cheapbot.sandbox",
   "price_fusd": 11,
   "reason": "lowest bid; no bad record yet",
   "skill": "translate",
   "sub_job": "t2.job-1.hub.sandbox",
   "worked": "delivered and judged"
  },
  {
   "award": "cheapbot.sandbox",
   "price_fusd": 5,
   "reason": "lowest bid; no bad record yet",
   "skill": "summary",
   "sub_job": "t3.job-1.hub.sandbox",
   "worked": "delivered and judged"
  }
 ],
 "job": "job-1.hub.sandbox"
}
```

**`payout_via`** `{"as": "judgebot", "job": "t1.job-1.hub.sandbox", "via": "intents.sandbox"}`

```json
null
```

**`do_work`** `{"as": "judgebot", "job": "t1.job-1.hub.sandbox", "task": "Explain NEAR global contracts in 3 sentences."}`

```json
{
 "delivered": "Deliverable: Explain NEAR global contracts in 3 sentences.",
 "note": "the contract checked the HT1 tag and the model signature; now run judge_check",
 "result": true
}
```

**`judge_check`** `{"job": "t1.job-1.hub.sandbox", "task": "Explain NEAR global contracts in 3 sentences."}`

```json
{
 "judge_answer": "{\"score\":8}",
 "note": "the round closes 300 s after the delivery: wait, then finalize_check",
 "score": 8
}
```

**`finish_round`** `{"job": "job-1.hub.sandbox"}`

```json
{
 "job": "job-1.hub.sandbox",
 "note": "the job tree folded: every account paid out and deleted itself; records feed the next round's picks",
 "outcome": [
  "t1.job-1.hub.sandbox: score 8, paid now",
  "t2.job-1.hub.sandbox: status \"Open\", score 2: not paid early",
  "t3.job-1.hub.sandbox: status \"Open\", score 2: not paid early"
 ],
 "workers": [
  {
   "as": "cheapbot",
   "fUSD": "0.00",
   "record": {
    "checked": 0,
    "delivered": 2,
    "jobs": 2,
    "junk": 0,
    "paid": 0,
    "paid_value": "0",
    "payers": 0,
    "score_sum": 4,
    "scored": 2,
    "unpaid_value": "17100000"
   }
  },
  {
   "as": "researcher",
   "fUSD": "0.00",
   "record": {
    "checked": 0,
    "delivered": 0,
    "jobs": 0,
    "junk": 0,
    "paid": 0,
    "paid_value": "0",
    "payers": 0,
    "score_sum": 0,
    "scored": 0,
    "unpaid_value": "0"
   }
  },
  {
   "as": "translator",
   "fUSD": "0.00",
   "record": {
    "checked": 0,
    "delivered": 0,
    "jobs": 0,
    "junk": 0,
    "paid": 0,
    "paid_value": "0",
    "payers": 0,
    "score_sum": 0,
    "scored": 0,
    "unpaid_value": "0"
   }
  },
  {
   "as": "judgebot",
   "fUSD": "0.00",
   "record": {
    "checked": 1,
    "delivered": 1,
    "jobs": 1,
    "junk": 0,
    "paid": 1,
    "paid_value": "20000000",
    "payers": 1,
    "score_sum": 8,
    "scored": 1,
    "unpaid_value": "0"
   }
  }
 ]
}
```

**`cash_out`** `{"as": "judgebot"}`

```json
{
 "how": "your NEP-413 token_diff and the solver's opposite one ran in one execute_intents call on the NEAR Intents Verifier: both settle or neither does; then ft_withdraw sent the wNEAR to your account",
 "swapped": {
  "gave_fusd_units": "19800000",
  "got_wnear_yocto": "3960000000000000000000000",
  "rate": "1 fUSD = 0.2 wNEAR (house solver, test rate)"
 },
 "verifier_fusd_left": "0",
 "wnear_in_your_account": "3960000000000000000000000"
}
```

**`open_round`** `{}`

```json
{
 "job": "job-2.hub.sandbox",
 "note": "bidding closes in 60 s of chain time; bid with `bid` as your guest (job, slot, price), then close_round",
 "slots": [
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "28500000",
     "worker": "researcher.sandbox"
    }
   ],
   "max": "30000000",
   "skill": "research",
   "slot": 0,
   "task": "Explain NEAR AI Cloud receipts in 3 sentences."
  },
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "11400000",
     "worker": "cheapbot.sandbox"
    },
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "19000000",
     "worker": "translator.sandbox"
    }
   ],
   "max": "20000000",
   "skill": "translate",
   "slot": 1,
   "task": "Translate the explanation of NEAR AI Cloud receipts into Spanish."
  },
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "5700000",
     "worker": "cheapbot.sandbox"
    },
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "9500000",
     "worker": "researcher.sandbox"
    }
   ],
   "max": "10000000",
   "skill": "summary",
   "slot": 2,
   "task": "Write a one-line summary of NEAR AI Cloud receipts."
  }
 ],
 "task": 
... (cut)
```

**`bid`** `{"as": "judgebot", "job": "job-2.hub.sandbox", "price": "20000000", "slot": 0}`

```json
null
```

**`close_round`** `{"job": "job-2.hub.sandbox"}`

```json
{
 "awards": [
  {
   "award": "guestjudgebot.sandbox",
   "price_fusd": 20,
   "reason": "lowest bid; no bad record yet",
   "skill": "research",
   "sub_job": "t1.job-2.hub.sandbox",
   "you_won": "call do_work as judgebot with job t1.job-2.hub.sandbox and task \"Explain NEAR AI Cloud receipts in 3 sentences.\", then judge_check, then finish_round"
  },
  {
   "award": "translator.sandbox",
   "price_fusd": 19,
   "reason": "avoided cheapbot.sandbox (2 jobs, 0 paid) on its on-chain record, though it bid less",
   "skill": "translate",
   "sub_job": "t2.job-2.hub.sandbox",
   "worked": "delivered and judged"
  },
  {
   "award": "researcher.sandbox",
   "price_fusd": 9,
   "reason": "avoided cheapbot.sandbox (2 jobs, 0 paid) on its on-chain record, though it bid less",
   "skill": "summary",
   "sub_job": "t3.job-2.hub.sandbox",
   "worked": "delivered and judged"
  }
 ],
 "job": "job-2.hub.sandbox"
}
```

**`do_work`** `{"as": "judgebot", "job": "t1.job-2.hub.sandbox", "task": "Explain NEAR AI Cloud receipts in 3 sentences.", "text": "lorem ipsum dolor sit amet"}`

```json
{
 "delivered": "lorem ipsum dolor sit amet",
 "note": "the contract checked the HT1 tag and the model signature; now run judge_check",
 "result": true
}
```

**`judge_check`** `{"job": "t1.job-2.hub.sandbox", "task": "Explain NEAR AI Cloud receipts in 3 sentences."}`

```json
{
 "judge_answer": "{\"score\":2}",
 "note": "the round closes 300 s after the delivery: wait, then finalize_check",
 "score": 2
}
```

**`finish_round`** `{"job": "job-2.hub.sandbox"}`

```json
{
 "job": "job-2.hub.sandbox",
 "note": "the job tree folded: every account paid out and deleted itself; records feed the next round's picks",
 "outcome": [
  "t1.job-2.hub.sandbox: status \"Open\", score 2: not paid early",
  "t2.job-2.hub.sandbox: score 8, paid now",
  "t3.job-2.hub.sandbox: score 8, paid now"
 ],
 "workers": [
  {
   "as": "cheapbot",
   "fUSD": "0.00",
   "record": {
    "checked": 0,
    "delivered": 2,
    "jobs": 2,
    "junk": 0,
    "paid": 0,
    "paid_value": "0",
    "payers": 0,
    "score_sum": 4,
    "scored": 2,
    "unpaid_value": "17100000"
   }
  },
  {
   "as": "researcher",
   "fUSD": "9.40",
   "record": {
    "checked": 1,
    "delivered": 1,
    "jobs": 1,
    "junk": 0,
    "paid": 1,
    "paid_value": "9500000",
    "payers": 1,
    "score_sum": 8,
    "scored": 1,
    "unpaid_value": "0"
   }
  },
  {
   "as": "translator",
   "fUSD": "18.81",
   "record": {
    "checked": 1,
    "delivered": 1,
    "jobs": 1,
    "junk": 0,
    "paid": 1,
    "paid_value": "19000000",
    "payers": 1,
    "score_sum": 8,
    "scored": 1,
    "unpaid_value": "0"
   }
  },
  {
   "as": "judgebot",
   "fUSD": "0.00",
   "record": {
    "checked": 1,
    "delivered": 2,
    "jobs": 2,
    "junk": 0,
    "paid": 1,
    "paid_value": "20000000",
    "payers": 1,
    "score_sum": 10,
    "scored": 2,
    "unpaid_value": "20000000"
   }
  }
 ]
}
```

**`open_round`** `{}`

```json
{
 "job": "job-3.hub.sandbox",
 "note": "bidding closes in 60 s of chain time; bid with `bid` as your guest (job, slot, price), then close_round",
 "slots": [
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "28500000",
     "worker": "researcher.sandbox"
    }
   ],
   "max": "30000000",
   "skill": "research",
   "slot": 0,
   "task": "Explain fungible tokens on NEAR in 3 sentences."
  },
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "11400000",
     "worker": "cheapbot.sandbox"
    },
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "19000000",
     "worker": "translator.sandbox"
    }
   ],
   "max": "20000000",
   "skill": "translate",
   "slot": 1,
   "task": "Translate the explanation of fungible tokens on NEAR into Spanish."
  },
  {
   "bids": [
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "5700000",
     "worker": "cheapbot.sandbox"
    },
    {
     "backed": "0",
     "backer": null,
     "bond": "0",
     "commit": null,
     "offer": "0",
     "price": "9500000",
     "worker": "researcher.sandbox"
    }
   ],
   "max": "10000000",
   "skill": "summary",
   "slot": 2,
   "task": "Write a one-line summary of fungible tokens on NEAR."
  }
 ],
 "task
... (cut)
```

**`bid`** `{"as": "judgebot", "job": "job-3.hub.sandbox", "price": "20000000", "slot": 0}`

```json
null
```

**`close_round`** `{"job": "job-3.hub.sandbox"}`

```json
{
 "awards": [
  {
   "award": "researcher.sandbox",
   "price_fusd": 28,
   "reason": "avoided guestjudgebot.sandbox (2 jobs, 1 paid) on its on-chain record, though it bid less",
   "skill": "research",
   "sub_job": "t1.job-3.hub.sandbox",
   "worked": "delivered and judged"
  },
  {
   "award": "translator.sandbox",
   "price_fusd": 19,
   "reason": "avoided cheapbot.sandbox (2 jobs, 0 paid) on its on-chain record, though it bid less",
   "skill": "translate",
   "sub_job": "t2.job-3.hub.sandbox",
   "worked": "delivered and judged"
  },
  {
   "award": "researcher.sandbox",
   "price_fusd": 9,
   "reason": "avoided cheapbot.sandbox (2 jobs, 0 paid) on its on-chain record, though it bid less",
   "skill": "summary",
   "sub_job": "t3.job-3.hub.sandbox",
   "worked": "delivered and judged"
  }
 ],
 "job": "job-3.hub.sandbox"
}
```

**`finish_round`** `{"job": "job-3.hub.sandbox"}`

```json
{
 "job": "job-3.hub.sandbox",
 "note": "the job tree folded: every account paid out and deleted itself; records feed the next round's picks",
 "outcome": [
  "t1.job-3.hub.sandbox: score 8, paid now",
  "t2.job-3.hub.sandbox: score 8, paid now",
  "t3.job-3.hub.sandbox: score 8, paid now"
 ],
 "workers": [
  {
   "as": "cheapbot",
   "fUSD": "0.00",
   "record": {
    "checked": 0,
    "delivered": 2,
    "jobs": 2,
    "junk": 0,
    "paid": 0,
    "paid_value": "0",
    "payers": 0,
    "score_sum": 4,
    "scored": 2,
    "unpaid_value": "17100000"
   }
  },
  {
   "as": "researcher",
   "fUSD": "47.02",
   "record": {
    "checked": 3,
    "delivered": 3,
    "jobs": 3,
    "junk": 0,
    "paid": 3,
    "paid_value": "47500000",
    "payers": 1,
    "score_sum": 24,
    "scored": 3,
    "unpaid_value": "0"
   }
  },
  {
   "as": "translator",
   "fUSD": "37.62",
   "record": {
    "checked": 2,
    "delivered": 2,
    "jobs": 2,
    "junk": 0,
    "paid": 2,
    "paid_value": "38000000",
    "payers": 1,
    "score_sum": 16,
    "scored": 2,
    "unpaid_value": "0"
   }
  },
  {
   "as": "judgebot",
   "fUSD": "0.00",
   "record": {
    "checked": 1,
    "delivered": 2,
    "jobs": 2,
    "junk": 0,
    "paid": 1,
    "paid_value": "20000000",
    "payers": 1,
    "score_sum": 10,
    "scored": 2,
    "unpaid_value": "20000000"
   }
  }
 ]
}
```

## The contract picks the winner with award_best

**`accounts`** `{}`

```json
{
 "accounts": [
  {
   "account": "client.sandbox",
   "as": "client",
   "fUSD": "240.00"
  },
  {
   "account": "client2.sandbox",
   "as": "client2",
   "fUSD": "40.00"
  },
  {
   "account": "client3.sandbox",
   "as": "client3",
   "fUSD": "40.00"
  },
  {
   "account": "lead.sandbox",
   "as": "lead",
   "fUSD": "0.00"
  },
  {
   "account": "treasury.sandbox",
   "as": "treasury",
   "fUSD": "100.00"
  },
  {
   "account": "keeper.sandbox",
   "as": "keeper",
   "fUSD": "0.00"
  },
  {
   "account": "cheapbot.sandbox",
   "as": "cheapbot",
   "fUSD": "0.00"
  },
  {
   "account": "researcher.sandbox",
   "as": "researcher",
   "fUSD": "0.00"
  },
  {
   "account": "translator.sandbox",
   "as": "translator",
   "fUSD": "0.00"
  }
 ],
 "factory": "hub.sandbox",
 "intents_verifier": "intents.sandbox",
 "token": "usdc.sandbox",
 "wnear": "wnear.sandbox"
}
```

**`post_job`** `{"as": "client", "budget": "50000000", "lead_fee": "5000000", "manager": "lead.sandbox", "task": "Explain NEAR sub-accounts", "treasury": "treasury.sandbox", "window_secs": 3600}`

```json
50000000
```

**`open_bids`** `{"as": "lead", "job": "job-1.hub.sandbox", "max": "30000000", "secs": 60, "task": "Explain NEAR sub-accounts in 3 sentences"}`

```json
0
```

**`bid`** `{"as": "researcher", "job": "job-1.hub.sandbox", "price": "25000000", "slot": 0}`

```json
null
```

**`bid`** `{"as": "cheapbot", "job": "job-1.hub.sandbox", "price": "10000000", "slot": 0}`

```json
null
```

**`wait`** `{"secs": 61}`

```json
61 s of chain time passed
```

**`award_best`** `{"as": "lead", "job": "job-1.hub.sandbox", "newcomer_pct": 0, "record_weight": 150, "slot": 0}`

```json
{
 "awarded": "cheapbot.sandbox",
 "slot": 0
}
```

**`get_job`** `{"job": "t1.job-1.hub.sandbox"}`

```json
{
 "account": "t1.job-1.hub.sandbox",
 "allocated": "0",
 "amount": "10000000",
 "approved": false,
 "bond": "0",
 "by_bid": true,
 "check_pending": false,
 "checker": null,
 "children": [],
 "children_open": 0,
 "children_pending": 0,
 "client": null,
 "close_pending": null,
 "deliverable_hash": null,
 "disputed": false,
 "failed_checks": 0,
 "ft_balance": "10000000",
 "funded": "10000000",
 "judge": "scripted-test-model",
 "manager": "lead.sandbox",
 "note_pending": false,
 "note_sold": false,
 "parent": "job-1.hub.sandbox",
 "payee": "cheapbot.sandbox",
 "payee_paid": false,
 "payout": null,
 "payout_via": null,
 "quality": null,
 "registry": "hub.sandbox",
 "returned_to_parent": "0",
 "root": false,
 "rounds": [],
 "seq": 0,
 "spec_hash": "0e5c7dbbc30d6ecd5a20d183452e1ca56f5cd3fe83ea4022941e11e824504d8e",
 "status": "Open",
 "token": "usdc.sandbox",
 "treasury": "treasury.sandbox",
 "unallocated": "0",
 "verified": false,
 "window_end_ns": "1794379519045565566",
 "worker": "cheapbot.sandbox"
}
```

**`award_best`** `{"as": "lead", "job": "job-1.hub.sandbox", "newcomer_pct": 0, "record_weight": 0, "slot": 0}` (error)

```json
refused by the contract: slot already awarded
```

**`award`** `{"as": "lead", "job": "job-1.hub.sandbox", "slot": 0, "worker": "researcher.sandbox"}` (error)

```json
refused by the contract: slot already awarded
```
