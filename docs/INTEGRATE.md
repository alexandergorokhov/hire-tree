# Plug Hire Tree into an agent market

Hire Tree is a settlement layer, not a rival marketplace. An agent market
keeps its own listings, matching and UI, and hands Hire Tree the parts it
does not cover in its public docs: who holds the money when an agent
subcontracts, what counts as done, and what happens to failed work.

| What the market needs | What Hire Tree does on-chain |
|---|---|
| Hold the budget with no operator key | Each job is its own account (`job-<n>.<factory>`), created from one global contract by code hash, with no owner key. |
| Let a hired agent subcontract safely | A worker can open bids below itself and hire out of its own pay (`t1.t1.job-<n>`). Its max must be less than its pay, so it keeps a margin. |
| Decide what counts as done | The contract checks NEAR AI's signature on the delivery, then writes its own judge question and checks the signed answer. |
| Handle failed work | Work that fails the check is not paid. `fire` ends the job at once, the buyer keeps the winner's bond and hires the runner-up. |
| Pay good work early | A checked worker can be paid now through the lead's treasury, not when the whole tree closes. |
| Share reputation across apps | Every job reports its worker's outcome to the factory. Any app can read `get_record(account)`: the fUSD the worker was paid or lost, and from how many managers. |

## 1. Post a job

The market's client sends one token transfer (NEP-141 `ft_transfer_call`)
to the factory:

```json
{ "receiver_id": "<factory>", "amount": "<budget>",
  "msg": "{\"manager\":\"<lead agent>\",\"spec_hash\":\"<64 hex>\",\"window_secs\":3600,\"lead_fee\":\"<optional>\"}" }
```

The factory creates the job account and funds it with the budget. If any
step fails, the whole budget goes back to the client.

**Or let agents compete to take it.** The client deposits once
(`msg: "deposit"`), registers an ed25519 key (`set_intent_key`), and signs
job intents off-chain: `factory`, `client`, `budget`, `spec_hash`,
`window_secs`, `max_lead_fee`, `starts_ns`, `ends_ns` and `nonce`. Any lead
agent calls `fill_intent(intent, signature)`. The lead fee rises from 0
like a Dutch auction, so the lead that asks least fills first. Each intent
fills once and only on this factory.

## 2. Hire, subcontract, award

On the job account:
1. `open_bids(spec_hash, max, secs, bond?, sealed?)` puts a part out for
   bids.
2. Workers call `bid(slot, price)`, or `bid_sealed` and then `reveal`. A
   newcomer can `seek_backer` and an established agent can `back` it.
3. `award_best(slot, record_weight, newcomer_pct, sub?, skip?, min_paid?, payers?)`
   makes the job read each bidder's record from the factory. It hires the
   best bid under the published rule (`ht_core::pick`), and any failed
   record read means no award. Pass `min_paid` (fUSD units) so that a
   record with less paid money than that still counts as a newcomer's.
   Pass `payers` (at most 16 manager accounts you trust) so that only their
   pay counts: a record a sybil washed through its own accounts is then no
   record for you (the factory's `get_record_from` filters; unpaid money
   counts whoever it came from). To choose by hand, use `award(slot, worker, sub?)`. Both refuse in the last
   60 s of the job's window: the winner needs time to deliver.

The award creates a child account that holds exactly the winning bid. The
winner can repeat these steps one level down.

## 3. Deliver, check, pay, fold

1. The worker calls `deliver` with NEAR AI's signed response.
2. A keeper submits the signed judge answer to the contract's own question
   (`submit_check`).
3. The buyer calls `approve` or `fire`.

When the tree folds, each account pays out, reports to the factory and
deletes itself.

## 4. From an AI agent

The repo ships an MCP server. To try the whole flow above from Claude Code
or any MCP client, run `make mcp-setup` (see [README](../README.md#for-ai-judges-mcp)).
The `hire-tree` tools take `as`, so one agent can play the market, the
client and every worker.
