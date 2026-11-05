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
| Strand tokens by deleting an account with a balance | delete only in the balance callback when the token says 0; stray tokens are booked and resettled | `stray_tokens_block_delete_and_are_settled_next`, sandbox G1 |
| A worker never registers on the token, so settle can never pay it and the tree is stuck | settle and note sale register the payee first (NEP-145 refunds if already registered) | sandbox G1, `settle_delivered_sub_registers_then_pays_worker_first` |
| A settle callback runs out of gas and the job is stuck in Settling | one transfer in flight per leg; `resume_settle` after 10 min decides from the real balance alone; attempt numbers refuse the late callback | `resume_*`, `late_original_callback_cannot_act_after_resume` |
| Stray tokens sent mid-settle to confuse `resume_settle` | strays only add; the worst case makes a paid leg look unpaid, and the extra payment is funded by the stray sender | `resume_counts_extra_tokens_as_stray` |
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
| Re-roll the AI check until junk passes | only `deliver` starts a check, one per seq, max 3 deliveries; the seed is chosen by the contract | `deliver_schedules_exactly_one_check`, `deliveries_are_capped` |
| Forge a check answer | `on_check` is private, checks the caller and the seq | `check_callback_rejects_outside_callers`, `stale_check_answer_refused` |
| Checker down | the job stays Delivered: the worker is still paid at settle, the note just is not sellable | `check_failure_leaves_delivered` |
| Nonsense score | anything outside 0..=10 counts as no answer | `out_of_range_score_counts_as_no_answer` |
| Prompt injection in the deliverable aimed at the AI checker | the checker sees the deliverable as data inside a fixed rubric; the dispute adds two more panels with fresh seeds, 2 of 3 decide. Limit: the AI judge can still be fooled; the lead (who pays early) carries that risk, not the client | `dispute_*` |

## Job Note
| Threat | Answer | Test |
|---|---|---|
| Sell the note, then dispute to get the pay back too (self-dealing) | only the lead's treasury may buy, and only the lead may dispute; an upheld dispute only costs the treasury | `dispute_after_own_note_purchase_only_hurts_treasury` |
| Stranger or the lead itself buys | treasury only | `stranger_cannot_buy_note`, `lead_itself_cannot_buy_only_its_treasury` |
| Buy cheaper | exact quote only | `wrong_price_refunded` |
| Buy an unchecked or unverified note | refused | `unchecked_note_refused`, `unverified_note_refused` |
| Worker payment fails mid-sale | sale undone, treasury refunded by the token | `failed_worker_payment_undoes_sale` |
| Settle or dispute while a sale is in flight | both wait | `settle_waits_for_note_sale`, `dispute_waits_for_note_sale` |

## Known limits
- If `on_note_paid` itself runs out of gas, the token refunds the treasury
  while the worker was paid from the job's balance. It has a 20 Tgas static
  budget and does three field writes; there is no resume path yet.
- Root (client) disputes are cut (plan cut order 4); a client's protection
  is the approval window and that the lead only gets the remainder.
- The checker is pinned per factory, not per job; changing it affects only
  new jobs.
- Testnet only; the lead's bond is recorded but not yet slashable.
