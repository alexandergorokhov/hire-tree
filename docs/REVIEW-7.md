# Adversarial review 7 (2026-11-11): what was found and what changed

Three fresh reviews: the contracts (read as an attacker, each finding proven
by a unit test first), the agents, MCP server and page, and a cold
machine-to-machine run in which an AI agent cloned the repo and used it as a
judge's Claude would. Every high and medium finding is fixed; each fix has
a test that would fail without it. Earlier reviews: `docs/THREATS.md`.

## Contracts

| ID | Sev | Finding | Fix | Test |
|---|---|---|---|---|
| R7-1 | High | The worker samples the judge offline and fills the check round with 8 picked passing answers in one transaction; the buyer's low answer is refused ("round full") and junk is paid | a full round still takes any answer that sets a new lowest or highest score | `a_full_round_still_counts_a_new_low_answer` |
| R7-2 | High | A refused `ft_transfer_call` to a pool shows tokens for a block; a member timing `finish` into it inflates the payout and claims the others' share | the root job reports its refund to the client (`job_refund`) after the transfer settled; the pool shares exactly that; reads are capped at what members put in | `a_reported_refund_is_shared_exactly_with_no_read`, `a_read_never_counts_more_than_members_put_in`, `a_settled_root_tells_its_client_what_came_back` |
| R7-3 | High | Bids push a job's state past NEAR's 10 KB delete limit, so it can never delete and a pool client never finishes | settle drops bids, check rounds and delivery hashes before the delete; the pool treats Settled as gone | `bids_cannot_keep_a_root_over_the_delete_size_limit`, `a_settled_job_counts_as_gone` |
| R7-11 | High (found while fixing R7-3) | Each bond refund to an implicit account pays NEAR's account-creation fee (about 8.3 Tgas), so a slot full of implicit bidders pushes the award or settle past 300 Tgas | the award refunds at most 4 itself; `refund_bids` (anyone, 16 per call) does the rest; settle asks for it when more than 4 are open; the agents' `settle` tool does that on its own | `an_award_with_a_full_slot_of_implicit_bidders_fits_in_300_tgas` |
| R7-4 | Medium | One low answer makes good work "unclear"; unclear work was unpaid and lost its bond and its backer's NEAR to the buyer | unclear or unscored work is unpaid but keeps its bond; only no delivery, a failed check or an upheld dispute forfeits it | `an_unclear_check_leaves_good_work_unpaid_but_keeps_its_bond`, `unpaid_but_not_junk_work_keeps_its_bond`, `a_failed_check_forfeits_the_bond` |
| R7-5 | Medium | A 1-yocto backer locks a bid against real underwriters and takes the whole premium | a backing is at least the bidder's bond and twice the premium | `a_dust_backing_is_refused` |
| R7-6 | Medium | A contractor's sub-job named the lead's treasury, while the contractor is its disputer: buy the note, dispute, keep both | contractor sub-jobs name no treasury | `a_contractors_sub_job_names_no_treasury` |
| R7-7 | Medium-Low | A sub-job's worker could award the slot its lead opened | a slot records which role opened it; only that role awards it | `the_worker_cannot_award_the_managers_slot` |
| R7-9 | Low | `close_unfunded` deleted a child without refunding bids placed on it | bids are refunded first | covered by the bond tests |
| R7-10 | Low | `reclaim_bond` stopped at a backer's first, already-refunded backing | it finds the next backing that still holds NEAR | covered by the bond tests |
| R7-8 | Low | `resume_note` reads a balance after a lost callback | not changed; listed in THREATS.md known limits | - |

The contract state layout changed (new fields), so a testnet run needs a
fresh deploy of the job, pool and factory code.

## Agents, MCP server and page

- The page said bad work was paid 0.00; in the live NEAR AI run a test
  agent sending fluent filler passed the single check 7 of 7 times. The
  page, README and THREATS.md now say so, and docs/NEXT.md plans a sharper
  check question.
- `sign_intent` could sign for any demo client off the local node with the
  public demo seed; it now needs a private `HT_INTENT_SEED` there and never
  replaces a key an account registered itself.
- `find_agents` ranked by fired jobs only, so a 0%-pass bot sat above a
  clean newcomer; it now ranks by the share of hired money left unpaid,
  and leaves the client, lead, treasury and keeper out of the catalog.
- Views the contract refuses are no longer retried for 30 s; the node boots
  at `initialize`; a failed boot is an error, not a dead server.

## Machine-to-machine (a judge's Claude)

The cold run passed every check but found friction. Now:
- `CLAUDE.md` and `llms.txt` give the reviewer's path; the README opens
  with it.
- `make judge` runs every offline check and ends `ALL PASS (10 checks)`.
- `make start` builds, runs the story on a local node, serves the pages and
  prints the URLs.
- MCP: `guide`, `accounts` and `find_agents` come first; numbers and digit
  strings both work as arguments; a bare `null` result reads `ok`; SIGTERM
  stops the local node with the server; `CARGO_TARGET_DIR` is respected.
