# Threats and how Hire Tree answers them

All money is fake test USD on sandbox or testnet. This file is written as if
it were real. Each row names the test that shows the answer.

## Actors
| Actor | Wants |
|---|---|
| Client | good work, refund of unspent budget |
| Lead (manager) | its fee (a cap the client sets, earned pro rata to the parts whose work passed); hires workers from the client's budget |
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
| A settle callback runs out of gas and the job is stuck in Settling | settle needs 250 Tgas (every path of the chain, receipt fees included, reaches the delete at exactly that, checked end to end); one transfer in flight per leg; `resume_settle` after 10 min decides from the real balance; attempt numbers refuse the late callback | `resume_*`, `late_original_callback_cannot_act_after_resume`, `settle_at_minimum_gas_budget_reserves_enough_for_every_leg` |
| A payee routes its pay (`set_payout_via`) to a receiver that keeps only part of it: the token sends the rest back, the books expect more than the job holds, every later settle fails, and the sub-job (and the parent, root, client refund, lead fee and bond above it) stays locked until someone pays the ransom | the job books exactly what `ft_transfer_call` says the receiver kept as paid and the rest as balance still owed to the payee; the route is dropped for good and the next settle pays the rest plainly. A result above the pay, or junk, counts as the whole pay (books can then only be below the real balance, which the balance leg sweeps). A lost callback is resolved from the real balance, a half-done routed leg included. A note sale whose route keeps part still sells: the rest goes to the worker plainly in the same chain, or, if that fails, the sale is undone and the part the worker got counts as prepaid. The property test checks money with partly kept payee legs in every random tree | `a_route_that_keeps_part_cannot_freeze_the_tree`, `a_lead_route_that_keeps_part_is_owed_only_the_rest`, `a_dropped_route_cannot_be_set_again`, `a_route_reporting_more_than_it_was_sent_counts_as_paid_in_full`, `an_unreadable_route_result_counts_as_paid_in_full`, `resume_of_a_routed_leg_that_kept_part_owes_the_payee_only_the_rest`, `a_route_that_keeps_part_of_the_note_price_gets_the_rest_plainly`, `a_failed_plain_rest_of_the_note_price_undoes_the_sale`, `stuck_sale_worker_partly_paid_through_its_route_counts_as_prepaid`, `a_partly_used_payee_transfer_books_what_left_and_owes_the_rest`, `money_is_conserved_in_any_tree` |
| A phantom transfer during `resume_settle`'s read makes a paid leg look unpaid (payee paid twice) | surplus never booked; only the manager may resume in the first day, so the payee cannot pick the moment | `resume_never_books_extra_tokens`, `stranger_cannot_resume_in_the_first_day` |
| A hire's callbacks are lost or the fund result is ambiguous: tree stuck in "children pending" | `resume_hire` after 10 min reads the child's own books and its real balance: funded opens it (even if it already paid out), never funded and empty rolls back and closes it; a funded child that settles first is closed by its own `child_closed`, which says what it was funded with | `stuck_hire::*`, `fund_failure_waits_then_resume_rolls_back_and_closes_child`, `resume_hire_opens_a_funded_child_that_already_paid_out`, `a_funded_child_whose_fund_callback_was_lost_closes_on_its_notice` |
| A sub-job runs out of NEAR after ~10 hires and a hire callback aborts | each hire must attach 0.035 NEAR; the job keeps 5 mNEAR for the child's token registration and its own record | `hire_creates_child_from_global_contract` |
| The root's final payout to an unregistered manager fails forever | the root registers the manager first (refunded if already registered) | `root_fee_and_bond_go_to_the_manager_the_rest_back_to_the_client` (asserts `storage_deposit` for the manager first) |
| Spam posts drain the factory's NEAR | each post uses 0.06 NEAR the client prepaid; budget >= 1 fUSD; a window under 60 s is refused, and a root that was never funded still settles (no stuck roots) | factory `post_without_prepaid_near_refunded`, `dust_budget_refunded`, `a_window_shorter_than_a_minute_is_refused`; job `a_root_that_was_never_funded_still_settles` |
| Bids after the job is done strand bonds | `bid` requires an open job with an open window | `a_bid_after_approve_is_refused`, `a_bid_while_settling_is_refused` |
| A contractor disputes honest sub-work and keeps the pay | signed panels that get no answer or disagree abstain; only two scored junk votes uphold | `a_dispute_with_empty_panels_is_rejected`, `a_buyer_cannot_win_a_dispute_with_one_low_answer_against_a_high_one` |
| A hostile MCP caller crashes the playground | stake capped at 5 NEAR with checked sums; a panic in one call becomes an error reply | agents `a_hostile_stake_is_refused_not_a_crash` |
| Money rule broken by any sequence | property test over 10,000 random trees with random transfer failures | `core/tests/money_prop.rs` |
| A lead hires junk (or nothing), every part is fired or refunded, and it still collects its whole fee; or it hires one tiny part (1 base unit) that passes and collects its whole fee | the root's fee is a cap: settle pays the lead `fee x paid / max(hired, floor)` (rounded down, the client keeps the unit). The floor is half the root's work budget (`funded - fee`, what the client funded for parts), rounded up: a lead must pass work worth at least half the work budget to earn the whole fee, so one tiny part that passes earns about 0 and a lead that spends 30% with everything passed earns 60% of the fee; a lead that spends half or more is not affected. The floor is set when the lead first hires: once the root has a child or anything allocated it refunds any further `fund`, so the client cannot top up the root after the parts passed (right before `approve`) to raise the floor and cut the fee to about 0. The lead_fee event names the floor. Every part the lead hired counts, at any depth, weighed by its price (its pay, not its sub-budget): a sub-job's `child_closed` reports the parts the lead hired under it, so failed parts cannot hide under a cheap wrapper part with a large sub-budget. A part counts as paid only when its work passed (a passed check, or any delivery with no check pinned); a failed part whose task another part passed (the runner-up) counts for nothing. No part passed, or none hired, pays no fee, only the bond. The unearned fee goes back to the client (or the pool). The property test checks the lead's take against the rule in every random tree | `a_lead_whose_every_part_failed_earns_no_fee`, `a_root_without_children_pays_no_lead_fee`, `the_lead_fee_is_earned_pro_rata_to_the_paid_parts`, `the_earned_fee_rounds_down_in_the_clients_favour`, `a_rolled_back_hire_does_not_count_for_the_fee`, `a_sub_budget_cannot_hide_failed_parts_from_the_lead_fee`, `a_sub_job_reports_the_parts_its_manager_hired_under_it`, `the_fee_weighs_each_part_by_its_price_not_its_sub_budget`, `a_failed_part_whose_task_passed_again_does_not_count`, `a_paid_part_that_did_not_pass_does_not_earn_the_fee`, `a_note_bought_part_counts_as_paid_for_the_lead_fee`, `a_root_with_no_paid_part_pays_no_fee`, `a_lead_that_hires_one_tiny_part_earns_almost_no_fee`, `a_lead_that_spends_over_half_the_work_budget_on_passed_parts_earns_the_whole_fee`, `a_lead_that_spends_under_half_the_work_budget_earns_the_fee_pro_rata_to_the_floor`, `a_client_top_up_after_the_parts_passed_does_not_cut_the_lead_fee`, `a_client_can_top_up_the_root_before_the_lead_hires`, `the_fee_floor_is_half_the_work_budget`, `earned_fee_divides_by_at_least_the_spend_floor`, `the_spend_floor_rounds_up_in_the_clients_favour`, `earned_fee_is_exact_and_capped` |
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
| The worker (or the buyer) samples the judge offline and fills the round with 8 picked answers in one transaction, so the other side's answer is refused as "round full" | a full round still takes any answer that sets a new lowest or highest score, and only those two decide; storage stays at 8 stored answers | `a_full_round_still_counts_a_new_low_answer` |
| The buyer splits the round on good work (one low answer makes it unclear) to keep the worker's bond and its backer's NEAR | unclear or unscored work is unpaid but keeps its bond; only no delivery, a failed check or an upheld dispute sends the bond to the buyer (the child tells the parent with `junk` in its close notice) | `an_unclear_check_leaves_good_work_unpaid_but_keeps_its_bond`, `unpaid_but_not_junk_work_keeps_its_bond`, `a_failed_check_forfeits_the_bond` |
| Prompt injection in the deliverable aimed at the AI checker | the checker sees the deliverable as data inside a fixed rubric; the dispute adds two more panels with fresh seeds, 2 of 3 decide; a missing vote counts as junk, and a stuck dispute resumes as upheld (never in the worker's favour). Limit: the AI judge can still be fooled; the lead (who pays early) carries that risk, not the client | `dispute_*` |

## Job Note
| Threat | Answer | Test |
|---|---|---|
| Sell the note, then dispute to get the pay back too (self-dealing) | only the lead's treasury may buy, and only the lead may dispute; an upheld dispute costs the treasury the face value and the pay goes up the tree (the lead at most breaks even; the worker already has its money) | `dispute_after_own_note_purchase_only_hurts_treasury` |
| Echo attack: a worker gets a NEAR AI receipt for a copy of someone's answer, and sells the note | notes need a pinned checker and a passing score on the delivered bytes, not only a receipt | `no_checker_no_note` |
| A contractor hires its own second account, the treasury buys that sub-job's note, then the contractor (that sub-job's disputer) disputes and gets the pay back | a contractor's sub-job names no treasury, so no note of it can be sold to the lead's treasury | `a_contractors_sub_job_names_no_treasury` |
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
| Bid spam fills a slot | 16 bidders per slot, 16 slots per job; a bidder's second bid replaces its first | `bids_per_slot_are_capped`, `a_new_bid_replaces_the_bidders_old_one` |
| Late bid or award before close | refused both ways | `late_bid_refused`, `award_before_close_refused` |
| Lead bids through another account and awards itself | possible, as with a direct `hire`: the lead already chooses who works. The client's protection is the judge's check and settle (unscored work is not paid unless approved). The fee rule does not protect here: a self-awarded part that passes the check earns the lead its share of the fee like any other part (see Known limits) | not preventable on-chain |
| Fake bidders fill the job's storage for free | a new bid attaches 0.001 NEAR for its own storage; at most 16 bids per slot, 16 slots | `a_new_bid_pays_for_its_storage` |
| A backer is drained by a premium it never agreed to | the premium is fixed by the bidder before anyone backs, at most half the bidder's own bond, and paid only when the work is paid | `a_premium_above_half_the_bond_is_refused`, `a_backer_of_paid_work_gets_its_near_back_plus_the_premium` |
| A junk bot backs itself to look vouched | the bidder and the buyer cannot back; a backing from a fresh account of the same operator is possible, but that account loses the NEAR on junk like a bigger bond would | `a_bidder_cannot_back_itself`, `a_backer_of_junk_loses_its_near_to_the_buyer` |
| A backer's NEAR is stuck after the bid loses | refunded at the award with every losing bond (past the first 4 refunds, by `refund_bids`, which anyone may call) | `a_backer_of_a_losing_bid_is_refunded_at_the_award` |
| A sniper backs a bid with 1 yocto: real underwriters are refused, and the sniper still takes the whole premium | a backing must be at least the bidder's own bond and twice the premium | `a_dust_backing_is_refused` |
| Bidders from implicit (64-hex) accounts make the award or settle run out of gas: each bond refund to such an account pays NEAR's account-creation fee (about 8.3 Tgas), so 15 losers plus backers pass 300 Tgas | an award refunds at most 4 losers itself; settle refunds at most 4 open bonds itself and otherwise asks for `refund_bids` (anyone, 16 per call, about 135 Tgas); the agents' `settle` tool does that on its own | `an_award_with_a_full_slot_of_implicit_bidders_fits_in_300_tgas`, `bids_cannot_keep_a_root_over_the_delete_size_limit` |
| Bids push a job's state past NEAR's 10 KB limit for deleting an account, so it never deletes (and a pool client never finishes) | settle drops the bids, check rounds and delivery hashes before the delete; a pool also treats a Settled job as gone | `bids_cannot_keep_a_root_over_the_delete_size_limit`, `a_settled_job_counts_as_gone` |
| In a sub-job, its worker awards the slot the lead opened (to its own second account) | a slot records whether the manager or the worker opened it; only that role can award it | `the_worker_cannot_award_the_managers_slot` |
| A flood of fresh accounts (a junk storm) takes every auction with half-price polished junk | each bid stakes the slot's bond and a junk winner loses it; no record means the newcomer premium; passes from untrusted workers are audited; at most 16 bids per slot, so a storm cannot grow a slot. `make swarm-seeds STORM=R` replays it in the records, blind and audit-all arms and charts the damage | `the_storm_floods_every_skill_without_passing_the_bid_cap`, `the_series_row_counts_junk_share_and_pay` |
| The buyer picks a winner by a secret or biased rule | `award_best` runs the rule in the contract: the buyer names only the weights and a skip list, and the event lists the rule, every bidder's inputs and score. A buyer that wants a friend can still use `award` by hand (as with `hire`) | `a_bad_record_costs_more_than_a_low_bid_saves`, `skipped_bidders_are_not_read_or_awarded` |
| A record read fails or returns junk during `award_best` | fails closed: nobody is awarded, the 0.035 NEAR deposit is refunded, the slot stays open for a retry or a hand `award` | `a_failed_record_read_awards_nobody_and_refunds`, `a_garbled_record_fails_closed` |
| Many bidders make `award_best` run out of gas | at most 16 bids per slot, each paid for by its bidder; 5 Tgas per read, so 16 reads plus the hire fit the 300 Tgas cap, and `award_best` refuses a call with too little gas before reading anything | `award_best_needs_gas_for_every_read`, `the_most_bids_a_slot_takes_fit_in_the_gas_cap` |
| `award` and `award_best` (or two `award_best`) race on one slot | the slot is marked awarded in one place; the late callback sees it, awards nothing and refunds | `a_hand_award_between_the_read_and_the_callback_wins_the_race`, `two_award_bests_award_the_slot_once` |
| A callback hire fails after the deposit was taken | the callback checks money, slot state and the child's account id before it marks anything and refunds when it cannot hire | `a_job_that_cannot_fund_the_hire_refunds_instead_of_stranding`, `a_child_id_too_long_refunds_instead_of_stranding` |
| The buyer awards in the last second of its window: the winner's sub-job gets what is left of the window, cannot deliver in time, and the buyer keeps its bond and its backer's NEAR (up to 5 + 5 NEAR per slot) and the winner's record shows the pay unpaid | `award` and `award_best` refuse unless at least 60 s of the window are left (`MIN_WORK_SECS`); the callback checks again and refunds; the bonds of a slot that can no longer be awarded go back at settle | `a_late_award_cannot_take_the_winners_bond`, `a_late_award_best_is_refused`, `an_award_best_callback_that_lands_too_late_refunds`, `an_award_leaves_the_winner_at_least_a_minute` |
| On a slot that asks for no bond, a bidder attaches a few yocto, counts as "1000x the bond" and its newcomer premium is divided away | with no slot bond every bid carries 1: nothing is at stake to multiply | `yocto_on_a_slot_without_a_bond_do_not_cut_the_premium`, `a_slot_without_a_bond_gives_no_vouch` |
| A fresh junk account washes a cent through a job it posts and pays itself, so `award_best` no longer sees a newcomer | the buyer passes `min_paid`: paid money below it is no record (a clean thin record is a newcomer again; a thin record with unpaid money weighs all unpaid). Optional, so a buyer that leaves it out keeps the old rule. Limit: washing `min_paid` costs the platform fee and 0.06 NEAR, not the money itself | `paid_money_below_min_paid_does_not_make_a_record`, `a_thin_record_with_unpaid_money_keeps_its_penalty` |
| A sybil washes a real record: its own client posts a job through the factory, its own manager hires it and pays it, twice from two accounts, so it crosses `min_paid` and the audit bar (30 fUSD from 2 payers) for the platform fee and some NEAR. "Distrust newcomers" then fails: the sybil is no newcomer | the buyer says whose pay counts: `award_best` takes a `payers` list (at most 16 managers) and the job reads each bidder through the factory's `get_record_from`, so paid money and the payer count come only from listed managers; the swarm's buyers list the three leads plus every account the leads' own pay made trusted, read on-chain. The washed sybil's record is then empty for that buyer: it pays the newcomer premium and gets audited. The inputs (the list, and each bidder's listed paid money and payer count) are in the `award_best` event, so the pick is re-checkable. Optional: left out, the rule is unchanged. Limits: it moves the trust to the buyer's list (a listed manager that pays a sybil still builds its record: list managers, not whoever paid); unpaid money counts whoever it came from; a washer pays the fee and NEAR for the wash again for every fresh account | `a_payer_list_makes_the_job_read_the_record_from_those_payers`, `pay_from_payers_the_buyer_does_not_list_is_no_record`, `a_listed_manager_pay_counts_like_any_record`, `the_payer_list_and_what_was_read_are_in_the_events`, `without_a_payer_list_the_reads_and_events_are_as_before`, `a_wash_crosses_the_trusted_rule_but_only_through_unlisted_payers`, `a_washed_record_beats_the_plain_rule_but_not_the_buyers_payer_list` |
| A sybil sprays payments from many accounts at a worker to push an honest payer out of the per-payer breakdown (to hurt that worker's record for buyers who list the honest payer), or to fill the factory's storage | the breakdown is bounded: 16 entries per worker (about 1.4 KB, 0.014 NEAR at most, kept by the factory), and a payment under 1 fUSD never gets an entry. With the list full a new payer must pay strictly more than the smallest entry to take its place, so small payers displace nobody; displacing an entry costs more than the entry, paid to the worker, and an entry only grows. Money without an entry is kept in `other`, so the entries and `other` always add up to `paid_value`. A displaced payer reads as nothing to a buyer who lists it (distrust, never trust). The `payers` count in the plain record is unchanged (a set of all payers, as before). Limit: a spender who pays more than a worker's smallest entry sixteen times over can push honest entries out; it pays the worker that money | `spraying_small_payers_cannot_evict_an_honest_one`, `a_full_list_is_displaced_only_by_paying_more_than_its_smallest`, `dust_never_takes_a_place`, `the_breakdown_is_bounded`, `the_payer_breakdown_adds_up_to_paid_value`, `an_entry_only_grows_and_is_never_the_one_displaced_by_its_own_top_up` |
| A buyer passes a long payer list, or the record reads cost too much gas | the list is at most 16 accounts, in the job and in the factory; the factory filters, so each read returns one small record whatever the breakdown holds, and the read stays at 5 Tgas (a 16-entry list is one storage read of about 1.4 KB and a loop over 16 accounts, far under that); 16 reads plus the hire still fit the 300 Tgas cap. A registry without `get_record_from` fails the read, so the award fails closed | `a_payer_list_over_sixteen_is_refused`, `listing_more_than_sixteen_payers_is_refused`, `a_failed_listed_read_awards_nobody_and_refunds`, `the_most_bids_a_slot_takes_fit_in_the_gas_cap`, `an_empty_payer_list_counts_nobodys_pay` |
| The buyer skips every bidder but a friend and the event still reads like a fair pick | `award_best` emits `award_best_asked` with the skip list and the bidders it reads, before any read | `the_skip_list_is_in_the_event` |


## Intents
| Threat | Answer | Test |
|---|---|---|
| A lead replays a filled intent to spend the client's deposit twice | each nonce fills once | `an_intent_fills_only_once` |
| A lead raises the budget or its fee in the intent | the signature covers the whole intent JSON | `a_tampered_intent_is_refused` |
| An intent signed for another factory is filled here | the intent names its factory | `an_intent_for_another_factory_is_refused` |
| A stale intent is filled long after the client moved on | refused after `ends_ns`; the client can cancel a nonce at any time | `an_expired_intent_is_refused`, `a_cancelled_intent_is_refused` |
| The client fills its own intent to collect the lead fee | refused | `the_client_cannot_fill_its_own_intent` |
| A failed post loses the budget | it returns to the deposit | `a_failed_intent_post_goes_back_to_the_deposit` |
| A stranger fills an open intent, becomes the job's manager, and pays the budget to its own second account (a manager may approve unscored sub-work) | an intent may name its `leads`: only they can fill it, the same trust as naming a manager in `post_job`. An intent without `leads` trusts whoever fills first; then the client's protection is the judge: answer every check within its 300 s | `an_intent_that_names_its_leads_is_filled_only_by_them`, `a_named_lead_fills_the_intent`, agents `an_intent_can_name_its_leads` |
| Free calls fill the factory's storage until it cannot take a post (cancelled nonces; 1-yocto credits for made-up account ids) | a new cancelled nonce attaches 0.001 NEAR and a prepay is at least 0.001 NEAR, each more than the entry costs | `cancelling_an_intent_pays_for_its_storage`, `cancelling_twice_pays_once`, `a_dust_prepay_is_refused` |

## Records
| Threat | Answer | Test |
|---|---|---|
| A worker pads its record with many tiny paid jobs to hide one big junk job | records weigh money: buyers price the share of fUSD lost, not of jobs | `a_record_counts_money_and_distinct_payers`, `records_are_weighed_by_money_and_payers` |
| A lead pays itself to pump a worker's record past the audit | audits are skipped only for 30 fUSD paid by at least 2 distinct managers; paying yourself counts no payer | `a_record_counts_money_and_distinct_payers` |
| A lead poisons a rival's record: it `hire`s the rival without asking (a hire needs no consent), lets the job lapse, and the rival's record shows the whole pay unpaid, which `award_best` prices at up to x2.5 | unpaid money marks a record only if the worker took the job on: it won it with its own bid, delivered, or hired with the pay. A hire nobody answered reports 0 | `a_hire_nobody_bid_for_leaves_no_unpaid_mark`, `a_bid_winner_that_never_delivers_is_marked_unpaid`, `an_award_tells_the_child_it_was_won_by_a_bid` |
| A record above 42.9 million fUSD overflows the award rule's counters and its unpaid half disappears | a record too large for the counters is scaled down as a whole, so the unpaid share survives at any size | `a_huge_record_keeps_its_unpaid_share`, property `weighing_never_hides_unpaid_money` |

## Pool (collective purchasing)
| Threat | Answer |
|---|---|
| The operator who deployed the pool takes the money | The deploy key is deleted right after `new`; only the pool's methods move tokens |
| A member is diluted or overpays | Joins keep at most what the target still needs; the excess is refunded in the same transfer |
| Someone binds the pool to another job | `bind` asks the factory: the job's client must be this pool, with its budget, Posted |
| Finish counts the refund too early | the job reports what it sent back (`job_refund`) only after the transfer succeeded; without a report `finish` reads the balance only once the job is gone or Settled |
| A member times a refused `ft_transfer_call` to the pool (the token credits it, then takes it back a block later) so the count reads more than is there, then claims the others' share | members share exactly the job's reported refund, with no balance read (`a_reported_refund_is_shared_exactly_with_no_read`); a read, used only when no report came, never counts more than members put in (`a_read_never_counts_more_than_members_put_in`). Limit: if the report itself is lost, a read can still be inflated up to what was raised |
| A minority approves bad work early | Approval needs voters holding more than half the stake |
| The launch fails half way | Any refusal or missing answer is `LaunchFailed`; members claim back what the pool holds |
| A claim transfer fails | The member's claimed amount is restored; claim again (`a_failed_claim_can_be_retried`) |
| Tokens arrive after the count (late refund, gift) | Anyone may `finish` again to recount, up to the reported refund (or what was raised); members claim the difference (`tokens_arriving_after_finish_are_shared_by_a_recount`) |

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

## Agent keys
| Threat | Answer | Test |
|---|---|---|
| An agent's key is stolen from the runtime that holds it | the worker delivers with a **function-call access key**, added by its owner for one job: it names one receiver (that job's account) and one method (`deliver`), and NEAR refuses a deposit on it. A thief can deliver (junk is caught by the check; the bond is at risk) but cannot transfer NEAR, call `ft_transfer`, delete the account, add a key, call another method or another job | `a_stolen_function_call_key_can_only_deliver_to_its_one_job` |
| A function-call key could cover bids too | it cannot: bids and bonds attach NEAR, a function-call key may not attach a deposit, and the receiver of a key is one fixed account while every job is a new account. So the deposit-attaching calls (`bid`, `bid_sealed`, `back`) stay on the owner's full-access key, and `reveal` and `seek_backer` are not scoped yet (they would need a key per bid). A thief who steals the owner's key still owns the account | not preventable with keys; keep the owner key out of the agent's runtime |
| A stolen delivery key keeps working after the job | the owner deletes it as soon as the delivery is made; its gas allowance is 0.5 NEAR and it names one job | `a_stolen_function_call_key_can_only_deliver_to_its_one_job` |

In the swarm one process plays both the owner and the agent, so the split is
shown but not enforced by separate machines: a production agent would hold
only the function-call keys and ask its owner (or a small signing service)
for the bid calls.

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
- Track records weigh money, but money can be washed: a junk bot can
  start over with a new account, and a lead can post a job, hire its own
  second account and pay it, for the platform fee and 0.06 NEAR. `payers`
  counts distinct managers, which a second account also fakes. `min_paid`
  in `award_best` and the audit rule raise the price; they do not close
  it. The `payers` list of `award_best` closes that door for a buyer that
  names its managers: only listed managers' pay builds a record for it
  (the swarm's wash storm, `make swarm-seeds STORM=6 WASH=1`, compares
  the two). It does not make records reputation: a buyer with no list of
  its own, or one that lists a manager a sybil can get paid by, is washed
  as before. Records help a lead avoid repeat junk; they are not reputation.
- A bid is consent to the job's window as it stands: a worker that wins a
  slot in a job with 60 s left has 60 s. Agents should read `window_end_ns`
  before they bid.
- A buyer and a bidder that work together can take an underwriter's NEAR
  (the bidder seeks a backer, wins, delivers junk; the buyer keeps bond and
  backing). The underwriter prices that risk; the contract does not.
- The demo's intent keys are derived from the account id and a seed
  (`HT_INTENT_SEED`, default "local" on the local node only): anyone can
  sign for a demo client there. Off the local node the agents refuse to
  sign without a private `HT_INTENT_SEED`, and never replace a key an
  account registered itself. A real client registers its own key.
- A sub-job's worker and its manager share the job's 16 bid slots and 16
  children: a contractor that opens dust slots can crowd out the lead's
  own hiring with the sub-budget until the window ends.
- `resume_note` (only after a lost callback) reads the token balance like
  the pool's fallback read, so a refused transfer timed into that read can
  make a paid note look unpaid. It needs a lost callback first, which the
  gas floors make unlikely.
- In signed mode anyone may submit an answer and the lowest counts; a
  client or rival who samples NEAR AI until it gets a low score can only
  pull it within 2 points of the others (3 apart is unclear, not failed),
  and a check nobody answers leaves the work unpaid unless the lead approves.
- The demo agents run as one process taking turns; each role only uses its
  own key and the shared tool list. Outside agents join through `hiretree mcp`.
- `child_closed` is acknowledged: a settled child deletes itself only after
  the parent took the notice; if it failed, the child stays Settled with
  `close_pending` and anyone can `resume_close` it, and the resend carries
  the caller's spare gas, so an out-of-gas notice can succeed the second
  time; a repeat credits nothing
  (`a_failed_parent_notice_keeps_the_child_alive_until_resume_close`,
  `a_resent_notice_gives_child_closed_the_callers_spare_gas`,
  `a_repeated_child_closed_credits_nothing`). A notice from a child whose
  hire the parent still lists in flight is decided by the `funded` it
  carries. Limit: a child the parent has marked Failed is still refused for
  good. `resume_hire` now reads the child's books, so a funded child is
  marked Failed only if its `get_funded` read fails while the token reports
  0 (the case of a child that was never created). `close_unfunded` is still
  fire-and-forget: the parent's books are rolled back first, so if it
  failed only the empty child's few mNEAR of storage would stay behind.
- NEAR from a failed create goes back to the job, not the manager (a few
  mNEAR; it flows up the tree on delete).
- The lead's fee is a cap the client sets at post (or the Dutch auction of
  a signed job offer, or a pool's terms). At settle the lead earns
  `fee x paid / max(hired, floor)`, where `floor` is half the work budget
  (`funded - fee`), rounded up, so hiring one tiny part that passes earns
  about nothing (`a_lead_that_hires_one_tiny_part_earns_almost_no_fee`).
  `hired` sums the prices (pay, not sub-budget) of
  every part the lead hired, at any depth (a sub-job reports the parts the
  lead hired under it); `paid` sums those whose work passed (a passed
  check, or any delivery with no check pinned). A failed part whose task
  another part passed counts for nothing, so firing junk and hiring the
  runner-up, or disputing and rehiring, costs the lead nothing. A lead
  whose parts all failed, or that hired nothing, gets only its bond;
  everything else goes back to the client
  (`root_fee_and_bond_go_to_the_manager_the_rest_back_to_the_client`,
  `a_lead_whose_every_part_failed_earns_no_fee`). Root (client) disputes
  are still cut (plan cut order 4): the client cannot dispute a part
  itself. Its protection is the fee rule, the judge's check on every part,
  and the approval window. What the rule does not stop:
  - Self-dealing. The fee is earned on work that passed the check; the
    contract cannot tell who did it or whether it was worth its price. A
    part from the lead's own second account that passes earns the lead its
    share like any part, and the lead keeps the price too.
  - Spending to dilute a failure. A failed part that was not redone counts
    against the fee, so buying more passing work raises `paid / hired`:
    with fee 10, a failed part of 20 and then an easy part of 70 bought
    from an honest worker, the lead earns 7.77 instead of 0, and the client
    pays 70 for that work. Any weight the lead sets (a price) can be raised
    this way, and a count of parts can be padded with small parts, so
    there is no weight the lead cannot move. The client's limits are the
    budget it posts and the fee cap.
  - Spending up to the floor. A lead that could do the job with less than
    half the work budget earns less than the whole fee unless it spends up
    to the floor, so it may buy dearer parts than it needs. The client
    caps that with the budget it posts: a budget near the real cost keeps
    the floor near the real cost too.
  - Disputing without redoing. A dispute that is upheld moves a part from
    paid to unpaid, so a lead that disputes junk the judge passed and does
    not rehire that task earns less than one that lets the junk be paid
    (the local-node scene: 8.33 instead of 10). Rehiring the task after the
    dispute makes it neutral. An unscored delivery, approved or not, never
    earns the fee, so approving one does not raise the lead's pay.
  - "Same task" means the same spec hash, which the lead picks. Parts hired
    under one spec hash are one task to the contract: one that passes
    leaves the failed ones out. The one that passed was judged against
    that same task text, so the client got that task done.
- The checker is pinned per factory, not per job; changing it affects only
  new jobs.
- Testnet only; the lead's bond is recorded but not yet slashable.
- The AI judge can be fooled. The swarm shows it: a polished-junk agent
  passes the first check about a third of the time. Buyers audit passes
  from thin records (2 more panels, 2 of 3 decide), which catches most of
  it; the rest is paid, and the README states how often. A worker with 3
  or more paid jobs is not audited, so a patient fooler that builds a
  record first is the open risk. With live NEAR AI it is worse: in the
  3-round live run (`web/swarm-live.json`) the polished-filler agent passed
  7 of 7 checks (scores 6 to 8, 66.90 of 207.90 fUSD paid), audits
  included. Fixed by judge rubric 2: the contract's question asks the
  judge to quote two passages that do this specific task before scoring,
  and the contract counts the score only if both quotes are found word for
  word in the delivery (else at most 2). Re-graded live by the same model
  (`runs/rubric2.json`, `make rubric-check`): filler passed 0 of 7 (was
  7 of 7); 10 of the 14 other passes still pass, the 4 that dropped are
  generic answers to placeholder tasks. The new question did that: the
  model scored the filler 0 with no quotes, and the contract's quote check
  changed none of the 29 scores; it is the on-chain backstop for a judge
  that is still fooled. Left open: a filler written for
  one task (on-topic sentences with no real content) can still quote
  itself; the dispute panels and the payer-weighted record are the next
  line (`rubric_2_counts_a_score_without_quotes_from_the_work_as_at_most_2`,
  `a_score_without_two_quotes_counts_at_most_two`).
- Firing ends the worker's chance to redeliver after one failed check. A
  borderline worker (score 5) loses the job; the signed check decides, not
  the buyer. A rich junk operator can still stake 10x and win once per
  fresh account; each loss costs it 0.5 NEAR.
