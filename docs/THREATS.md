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
| The root's final payout to an unregistered manager fails forever | the root registers the manager first (refunded if already registered) | `root_registers_the_manager_before_paying_it` |
| Spam posts drain the factory's NEAR | each post uses 0.06 NEAR the client prepaid; budget >= 1 fUSD | factory `post_without_prepaid_near_refunded`, `dust_budget_refunded` |
| Money rule broken by any sequence | property test over 10,000 random trees with random transfer failures | `core/tests/money_prop.rs` |

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

## Known limits
- One checker and one model for all three dispute panels; panels differ
  only by seed. A judge model that is fooled once may be fooled three times.
  The lead, who pays early, carries that risk, not the client.
- OutLayer's per-call deposit and latency on testnet are not measured yet;
  the sandbox uses `mock-outlayer`.
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
