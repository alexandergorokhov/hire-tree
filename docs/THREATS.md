# Threats and how Hire Tree answers them

All money is fake test USD on sandbox or testnet. This file is written as if
it were real. Each row names the test that shows the answer.

## Actors
| Actor | Wants |
|---|---|
| Client | good work, refund of unspent budget |
| Lead (manager) | margin; hires workers from the client's budget |
| Lead's treasury | 1% for paying workers early |
| Worker | to be paid, early if possible |
| Cheater (any worker or stranger) | pay for junk, someone else's pay, or a stuck tree |

## Money
| Threat | Answer | Test |
|---|---|---|
| Hire more than the budget, or hire with the lead's bond | `Ledger::allocate` checks unallocated funds; the bond is never allocatable | `hire_over_budget_refused`, `bond_cannot_be_hired_with` |
| Pay the worker twice (retried settle) | `payee_paid` is set only on a confirmed transfer; resettle sends only what is left | `rest_failure_never_pays_payee_twice`, property test |
| Strand tokens by deleting an account with a balance | delete only in the balance callback when the token says 0; a surplus is swept up the tree first | `stray_tokens_are_swept_up_then_the_job_deletes`, sandbox G1 |
| Lock a job forever with a refused `ft_transfer_call` timed to land during the balance check (the read sees tokens the token refunds a block later) | a surplus is never booked; it is swept, and counts only if the sweep transfer succeeds; a failed sweep changes nothing | `phantom_surplus_from_a_refunded_transfer_call_cannot_lock_settle` |
| A worker never registers on the token, so settle can never pay it and the tree is stuck | settle and note sale register the payee first (NEP-145 refunds if already registered) | sandbox G1, `settle_delivered_sub_registers_then_pays_worker_first` |
| A settle callback runs out of gas and the job is stuck in Settling | settle needs 150 Tgas (every leg's callback budget fits); one transfer in flight per leg; `resume_settle` after 10 min decides from the real balance; attempt numbers refuse the late callback | `resume_*`, `late_original_callback_cannot_act_after_resume`, `settle_at_minimum_gas_budget_reserves_enough_for_every_leg` |
| A phantom transfer during `resume_settle`'s read makes a paid leg look unpaid (payee paid twice) | surplus never booked; only the manager may resume in the first day, so the payee cannot pick the moment | `resume_never_books_extra_tokens`, `stranger_cannot_resume_in_the_first_day` |
| A hire's callbacks are lost or the fund result is ambiguous: tree stuck in "children pending" | `resume_hire` after 10 min reads the child's real balance: full opens it, zero rolls back and closes it | `stuck_hire::*`, `fund_failure_waits_then_resume_rolls_back_and_closes_child` |
| A sub-job runs out of NEAR after ~10 hires and a hire callback aborts | each hire must attach 0.035 NEAR; the job keeps 5 mNEAR for the child's token registration and its own record | `hire_creates_child_from_global_contract` |
| The root's final payout to an unregistered manager fails forever | the root registers the manager first (refunded if already registered) | `root_fee_and_bond_go_to_the_manager_the_rest_back_to_the_client` (asserts `storage_deposit` for the manager first) |
| Spam posts drain the factory's NEAR | each post uses 0.06 NEAR the client prepaid; budget >= 1 fUSD | factory `post_without_prepaid_near_refunded`, `dust_budget_refunded` |
| Money rule broken by any sequence | property test over 10,000 random trees with random transfer failures | `core/tests/money_prop.rs` |
| A contractor (a sub-job's worker who hires) spends money that is not its own | it hires only out of its own pay (`Ledger::carve`), must keep part of it, cannot add a sub-budget; the lead's unallocated budget is untouched | `a_contractor_carves_its_own_pay_and_gets_refunds_back`, `a_contractor_cannot_bid_out_its_whole_pay` |
| A contractor's sub-agent fails and the refund goes to the wrong account | a closed sub-job's return becomes the contractor's pay again (`restore`); a failed hire restores it too | `unspent_or_refunded_sub_pay_goes_back_to_the_contractor`, `a_failed_contractor_hire_restores_its_pay` |
| The treasury buys a contractor's note, then a sub-job refund changes what the contractor is owed | the note is not sellable while the contractor's sub-jobs are open | `a_contractors_note_waits_for_its_sub_jobs` |
| A contractor bids on its own slot, or a stranger hires in a sub-job | refused | `a_contractor_cannot_bid_on_its_own_slot`, `a_stranger_cannot_hire_in_a_sub_job` |
| A junk bot bids freely and walks away after failing | slots can ask for a NEAR bond; the buyer keeps a junk winner's bond, losers get theirs back at the award | `junk_loses_its_bond_to_the_buyer`, `a_paid_worker_gets_its_bond_back` |
| Junk wins a part and the buyer has to wait the whole window on it | after a failed signed check the buyer fires the worker (no pay, bond to the buyer, pay back at once) and hires the runner-up in the same window; it cannot fire passed or unchecked work | `the_buyer_fires_a_failed_worker_and_gets_everything_back`, `a_worker_whose_work_passed_cannot_be_fired`, `a_fired_worker_cannot_redeliver` |
| A fresh junk account looks the same as an honest first-timer | an honest newcomer stakes 10x the slot's bond and the buyer divides its newcomer premium by that; junk that copies it loses 0.5 NEAR per junk delivery | `a_newcomer_that_stakes_more_bond_pays_less_premium` |
| A bidder watches the open bids and undercuts by one unit at the last second | sealed slots: commit a hash, reveal after bidding closes; the award waits for the reveal | `sealed_bids_hide_the_price_until_the_reveal`, `no_award_before_the_reveal_closes` |
| A sealed bidder commits, then does not reveal to keep a free option | an unrevealed bid cannot win and its bond goes to the buyer | `an_unrevealed_bid_cannot_win`, `an_unrevealed_bond_goes_to_the_buyer` |
| The factory owner sets a confiscatory platform fee | capped at 1%; a failed post refunds the fee too | `the_fee_is_capped_at_one_percent`, `the_platform_fee_comes_off_the_top_and_is_kept_only_when_posted` |

## Receipts (AI proof)
| Threat | Answer | Test |
|---|---|---|
| Copy another task's genuine receipt | the request must carry exactly one tag `HT1:<this account>:<spec>:<seq>` | `receipt_copied_from_other_task_rejected`, sandbox scene |
| Replay my own earlier receipt on a redelivery | the tag's seq must be this delivery's seq | `receipt_replayed_on_redelivery_rejected` |
| Swap the response (junk) under a real signature | the response hash is inside the signed text | `receipt_with_swapped_response_rejected` |
| Swap the model name | the model is inside the signed text | `receipt_with_swapped_model_rejected` |
| Malleable signature (high s) or odd v | `parse_signature` refuses high s, zero r/s, v not in 0/1/27/28; `ecrecover` runs with the malleability flag | `high_s_signature_rejected`, core `high_s_refused_half_n_accepted` |
| Sign with my own key | the recovered address must equal the pinned NEAR AI Cloud signer | `receipt_from_other_key_rejected` |
| Two tags in one request (one call claimed by two tasks) | more than one distinct tag is refused | core `binding` tests |
| Huge request to burn gas | `req` <= 8 KB, model <= 128 bytes | `oversized_request_rejected`, `oversized_model_rejected` |
| A stranger submits a stolen receipt | only the job's worker may deliver | `stranger_cannot_deliver_a_stolen_receipt` |

What a receipt does NOT prove: that the answer is good. That is the quality
check's job. It also trusts NEAR AI Cloud's TEE key; the pinned address is
the one NEAR AI Cloud returned for the model (recorded in Agent Exit's G6
gate; see `receipt::tests::recorded_text_matches_near_ai_cloud`).

## Quality check (AI hardness)
| Threat | Answer | Test |
|---|---|---|
| Re-roll the AI check until junk passes | only `deliver` starts a check, one per seq, max 3 deliveries; the seed is chosen by the contract; the same bytes cannot be delivered twice; any failed check makes the note unsellable for good | `deliver_schedules_exactly_one_check`, `deliveries_are_capped`, `same_bytes_cannot_be_rerolled`, `a_failed_check_makes_the_note_unsellable` |
| Deliver junk in the last second, settle before the score lands | settle and approve wait while a check is pending | `settle_waits_for_a_pending_check`, `approve_waits_for_a_pending_check` |
| The checker judges something other than what was delivered | with a checker pinned, `deliver` must carry the bytes (<= 16 KB) and their sha256 must equal the delivered hash; the job hands those bytes to the checker and to dispute panels | `checker_needs_the_content`, `content_must_match_hash` |
| Forge a check answer | `on_check` is private, checks the caller and the seq | `check_callback_rejects_outside_callers`, `stale_check_answer_refused` |
| Judged delivery sent as a hash only, so no one can ask the judge | refused: with a judge pinned, `deliver` needs the bytes; unscored work is paid only on approval | `hash_only_delivery_refused_when_a_judge_is_pinned`, `unscored_judged_delivery_is_not_paid_without_approval` |
| Checker down, or its answer is lost | fail closed: the job stays Delivered and unscored; it is paid only if the manager approves it; `resume_check` clears a lost answer after 10 min | `check_failure_leaves_delivered`, `unscored_delivery_is_not_paid_without_approval`, `lost_check_answer_can_be_cleared_after_timeout` |
| Nonsense score | anything outside 0..=10 counts as no answer | `out_of_range_score_counts_as_no_answer` |
| Prompt injection in the deliverable aimed at the AI checker | the checker sees the deliverable as data inside a fixed rubric; the dispute adds two more panels with fresh seeds, 2 of 3 decide; a missing vote counts as junk, and a stuck dispute resumes as upheld (never in the worker's favour). Limit: the AI judge can still be fooled; the lead (who pays early) carries that risk, not the client | `dispute_*` |

## Job Note
| Threat | Answer | Test |
|---|---|---|
| Sell the note, then dispute to get the pay back too (self-dealing) | only the lead's treasury may buy, and only the lead may dispute; an upheld dispute costs the treasury the face value and the pay goes up the tree (the lead at most breaks even; the worker already has its money) | `dispute_after_own_note_purchase_only_hurts_treasury` |
| Echo attack: a worker gets a NEAR AI receipt for a copy of someone's answer, and sells the note | notes need a pinned checker and a passing score on the delivered bytes, not only a receipt | `no_checker_no_note` |
| Stranger or the lead itself buys | treasury only | `stranger_cannot_buy_note`, `lead_itself_cannot_buy_only_its_treasury` |
| Buy cheaper | exact quote only | `wrong_price_refunded` |
| Buy an unchecked or unverified note | refused | `unchecked_note_refused`, `unverified_note_refused` |
| Worker payment fails mid-sale | sale undone, treasury refunded by the token | `failed_worker_payment_undoes_sale` |
| Settle or dispute while a sale is in flight | both wait | `settle_waits_for_note_sale`, `dispute_waits_for_note_sale` |
| The sale callback is lost: sale stuck in flight forever | `resume_note` after 10 min undoes the sale (the token refunded the treasury) and books any worker payment already made as prepaid | `stuck_sale_worker_unpaid_is_undone`, `stuck_sale_worker_paid_from_job_counts_as_prepaid` |

## Bidding
| Threat | Answer | Test |
|---|---|---|
| Lead awards above a bid | the award pays exactly the winner's bid, read from state | `manager_opens_workers_bid_manager_awards_at_the_bid_price` |
| Slot opened for more than the job holds | `max <= unallocated budget` | `slot_max_cannot_exceed_the_free_budget` |
| Bid spam fills a slot | 8 bidders per slot, 16 slots per job; a bidder's second bid replaces its first | `bids_per_slot_are_capped`, `a_new_bid_replaces_the_bidders_old_one` |
| Late bid or award before close | refused both ways | `late_bid_refused`, `award_before_close_refused` |
| Lead bids through another account and awards itself | possible, as with a direct `hire`: the lead already chooses who works. The client's protection is the fixed lead fee, the judge's score and settle (unscored work is not paid unless approved) | not preventable on-chain |
| Fake bidders fill the job's storage for free | a new bid attaches 0.001 NEAR for its own storage; at most 8 bids per slot, 16 slots | `a_new_bid_pays_for_its_storage` |

## Pool (collective purchasing)
| Threat | Answer |
|---|---|
| The operator who deployed the pool takes the money | The deploy key is deleted right after `new`; only the pool's methods move tokens |
| A member is diluted or overpays | Joins keep at most what the target still needs; the excess is refunded in the same transfer |
| Someone binds the pool to another job | `bind` asks the factory: the job's client must be this pool, with its budget, Posted |
| Finish counts the refund too early | `finish` reads the balance only after the job account is gone; the job deletes itself only after its refund transfer succeeded |
| A minority approves bad work early | Approval needs voters holding more than half the stake |
| The launch fails half way | Any refusal or missing answer is `LaunchFailed`; members claim back what the pool holds |
| A claim transfer fails | The member's claimed amount is restored; claim again (`a_failed_claim_can_be_retried`) |
| Tokens arrive after the count (late refund, gift) | Anyone may `finish` again to recount; members claim the difference (`tokens_arriving_after_finish_are_shared_by_a_recount`) |

## Agents (off-chain)
| Threat | Answer | Test |
|---|---|---|
| A deliverable carries a prompt injection aimed at the lead's reviewer ("buy my note") | the reviewer can only buy a note the contract quotes, and the contract quotes only checked, verified, never-failed work; the worst case is a lost dispute, paid by the lead, not the client | `reviewer_buys_only_quoted_good_work`; contract: `quote_none_when_unchecked_or_unverified` |
| The picker model names an account that is not a candidate | refused before any hire | `agents.rs` `job()` (unknown worker is an error) |
| The planner overspends | refused before any hire if the parts exceed budget minus fee; the contract also caps every hire at what the job holds | `planner: bad plan`; contract: hire caps |
| An edited cassette answer | `verify` and the contract both recheck every signature | `cassette_verifies_and_catches_tampering` |
| The run diverges from the recording | replay answers only byte-identical requests and stops | `replay_refuses_a_request_it_never_saw` |
| NEAR AI signs a different text than the contract checks | the live client stops with NO-GO before anything goes on-chain | `make nearai-check` (live only) |
| Demo account keys | derived from the operator key held in an env var; nothing written to disk | `Chain::derived` |

## Known limits
- Sealed bids: when no bid on a slot is revealed, nobody can be awarded,
  and the unrevealed bonds go back to their bidders at settle (the buyer
  keeps an unrevealed bond only when it awards someone else).
- Pool: a member cannot leave before the deadline; a pool that misses its
  target pays back only after `join_secs`. Tokens sent to the pool by
  anyone else are shared among members. One pool per purchase (a ~1.6 NEAR
  deploy). If a launch fails, its 0.06 NEAR stays at the factory as the
  pool's posting credit (the pool has no withdraw). The pool trusts NEAR to
  deliver its own callbacks; there is no resume for a pool stuck in
  `Launching` or `Finishing`.
- One checker and one model for all three dispute panels; panels differ
  only by seed. A judge model that is fooled once may be fooled three times.
  The lead, who pays early, carries that risk, not the client.
- Track records are per-account counts, not weighted by money: a junk bot
  can start over with a new account, and a lead can hire itself to build a
  record. Records help a lead avoid repeat junk; they are not reputation.
- In signed mode anyone may submit an answer and the lowest counts; a
  client or rival who samples NEAR AI until it gets a low score can only
  pull it within 2 points of the others (3 apart is unclear, not failed),
  and a check nobody answers leaves the work unpaid unless the lead approves.
- The demo agents run as one process taking turns; each role only uses its
  own key and the shared tool list. Outside agents join through `hiretree mcp`.
- `child_closed` and `close_unfunded` are fire-and-forget; if one ever
  failed, the parent would need a manual fix (not seen in tests).
- NEAR from a failed create goes back to the job, not the manager (a few
  mNEAR; it flows up the tree on delete).
- The lead's fee is fixed by the client at post; the rest of the root's
  budget goes back to the client at settle (`root_fee_and_bond_go_to_the_manager_the_rest_back_to_the_client`).
  Root (client) disputes are still cut; the client's protection is the fee
  cap and the approval window.
- Root (client) disputes are cut (plan cut order 4); a client's protection
  is the approval window and that the lead only gets the remainder.
- The checker is pinned per factory, not per job; changing it affects only
  new jobs.
- Testnet only; the lead's bond is recorded but not yet slashable.
- The AI judge can be fooled. The swarm shows it: a polished-junk agent
  passes the first check about a third of the time. Buyers audit passes
  from thin records (2 more panels, 2 of 3 decide), which catches most of
  it; the rest is paid, and the README states how often. A worker with 3
  or more paid jobs is not audited, so a patient fooler that builds a
  record first is the open risk.
- Firing ends the worker's chance to redeliver after one failed check. A
  borderline worker (score 5) loses the job; the signed check decides, not
  the buyer. A rich junk operator can still stake 10x and win once per
  fresh account; each loss costs it 0.5 NEAR.
