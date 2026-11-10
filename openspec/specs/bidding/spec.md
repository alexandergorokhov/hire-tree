# bidding

## Purpose
Workers compete for a subtask on the job account itself. The manager puts
a subtask out for bids with a maximum price; any agent may bid up to that
maximum until the slot closes; the manager then awards one bidder, who is
hired at exactly its own bid as a new sub-account. The manager may weigh
track records, so the award need not be the lowest bid, but it can never
pay a bidder more than that bidder asked.

## Requirements

### Requirement: Only the manager opens a slot, within the free budget
`open_bids(spec_hash, max, secs)` SHALL be manager-only, on an open job,
inside the window, with `max <= unallocated budget`, `secs` 10..=3600,
closing before the window ends, at most 16 slots.
- unit: `stranger_cannot_open_bids`, `slot_max_cannot_exceed_the_free_budget`

### Requirement: Bids are capped, one per bidder, before close
`bid(slot, price)` SHALL refuse the manager and the job itself, a price of 0
or above `max`, a bid after `closes_ns` or on an awarded slot, and more than
16 bidders; a second bid from the same account replaces its first.
A new bid SHALL attach 0.001 NEAR for its storage (kept by the job);
replacing one's own bid needs none.
- unit: `a_new_bid_pays_for_its_storage`, `replacing_a_bid_needs_no_new_deposit`, `bid_above_max_refused`, `manager_cannot_bid`, `late_bid_refused`, `bid_after_award_refused`, `a_new_bid_replaces_the_bidders_old_one`, `bids_per_slot_are_capped`

### Requirement: Award hires a bidder at its own price, once, after close
`award(slot, worker, sub)` SHALL be manager-only, after close, once per
slot, only to an account that bid; it hires through the same path as
`hire` with `amount = that bid` and the slot's spec hash.

#### Scenario: Lowest bid not taken
- GIVEN bids 25 (worker) and 18 (worker2)
- WHEN the manager awards worker
- THEN t1 is created holding 25 and the slot shows awarded = worker
- unit: `manager_opens_workers_bid_manager_awards_at_the_bid_price`; `award_before_close_refused`, `award_to_a_non_bidder_refused`, `second_award_refused`

### Requirement: The award rule can run on-chain
`award_best(slot, record_weight, newcomer_pct, sub, skip, min_paid, payers)` SHALL be
manager-only, with the same preconditions as `award` (after close and any
reveal, once per slot) and at least one revealed bidder not in `skip`. The
job SHALL read each such bidder's track record from its registry (the
factory) in one batch of cross-contract `get_record` calls, with gas for
every read (at most 16 bidders, so the 300 Tgas cap suffices). A private
callback SHALL then score each revealed bid with `ht_core::pick` from the
on-chain bid, bond and backing and the records just read, emit
`award_best` with the rule and every input and score, and hire the lowest
score through the same code path as `award`: the winner is hired at its own
bid, losers' bonds and backing are refunded. The rule is the lowest bid
raised by `record_weight` times the share of money its record shows unpaid
and, for a bidder with no record, by `newcomer_pct` divided by how many
times the slot's bond the bid carries; ties go to more paid money, then the
earlier bid. The 0.035 NEAR attached is for the child account.

The award SHALL fail closed. If any record read fails or cannot be parsed,
or the slot was awarded meanwhile (by `award` or another `award_best`), or
the job can no longer hire or fund the hire, the callback SHALL award
nobody, emit `award_best_failed`, and refund the deposit to the buyer; the
slot stays open for a retry or a hand `award`. `award` stays for a buyer
that picks by hand.

#### Scenario: A bad record costs more than a low bid saves
- GIVEN bids 25 (worker, clean record) and 18 (worker2, all pay lost) and record weight 150
- WHEN the manager calls `award_best` and both record reads succeed
- THEN worker is hired at 25 and the event lists both inputs

#### Scenario: A record read fails
- WHEN one `get_record` fails or returns garbage
- THEN no one is awarded, the deposit is refunded, and a retry can award

#### Scenario: Award and award_best race
- WHEN a hand `award` lands between `award_best` and its callback, or two `award_best` run at once
- THEN exactly one award happens; the other callback refunds
- unit: `a_bad_record_costs_more_than_a_low_bid_saves`, `with_no_record_rule_the_lowest_bid_wins`, `a_bond_and_backing_cut_a_newcomers_premium`, `the_job_reads_every_bidders_record_from_the_registry`, `a_failed_record_read_awards_nobody_and_refunds`, `a_garbled_record_fails_closed`, `a_hand_award_between_the_read_and_the_callback_wins_the_race`, `two_award_bests_award_the_slot_once`, `a_hand_award_after_award_best_is_refused`, `skipped_bidders_are_not_read_or_awarded`, `unrevealed_sealed_bids_are_not_candidates`, `award_best_with_nothing_revealed_is_refused`, `award_best_before_the_close_is_refused`, `only_the_buyer_can_award_best`, `award_best_needs_the_childs_storage_deposit`, `award_best_needs_gas_for_every_read`, `the_most_bids_a_slot_takes_fit_in_the_gas_cap`, `on_award_best_is_private`, `a_job_that_cannot_fund_the_hire_refunds_instead_of_stranding`
- property: `the_winner_has_the_least_score_and_is_a_candidate`, `the_order_of_candidates_does_not_change_the_winner`, `a_lower_bid_never_loses_to_the_same_record_at_a_higher_one`, `never_panics_on_any_inputs`
- unit: `the_lowest_bid_wins_with_clean_records`, `ties_go_to_more_paid_money_then_the_lower_id`, `weigh_counts_cents_and_vouch_divides_by_the_slot_bond`
- sandbox: `the_contract_picks_the_winner_with_award_best`

#### Scenario: A late award
- GIVEN a slot with a bond and less than 60 s left of the job's window
- WHEN the buyer calls `award` or `award_best`, or an `award_best` callback lands that late
- THEN nobody is awarded (the callback refunds the deposit): the winner's sub-job would get only what is left of the window, and a winner that cannot deliver in time loses its bond to the buyer
- unit: `a_late_award_cannot_take_the_winners_bond`, `a_late_award_best_is_refused`, `an_award_best_callback_that_lands_too_late_refunds`, `an_award_leaves_the_winner_at_least_a_minute`

#### Scenario: A washed record
- GIVEN a bidder whose record is 1 fUSD it paid itself, and `min_paid` = 10 fUSD
- WHEN `award_best` scores it
- THEN its paid money counts as none: it pays the newcomer premium, and any unpaid money it has weighs as its whole record. Without `min_paid` the rule is unchanged
- unit: `paid_money_below_min_paid_does_not_make_a_record`, `a_thin_record_with_unpaid_money_keeps_its_penalty`

#### Scenario: Whose pay counts
- GIVEN `payers`, a list of at most 16 manager accounts the buyer trusts
- WHEN `award_best` runs
- THEN the job reads each bidder with the factory's `get_record_from(account, payers)` instead of `get_record`, so a bidder's paid money (and the payer count in the event) comes only from those managers, and `min_paid` applies to that money. Unpaid money still counts whoever it came from. The list is in `award_best_asked` and `award_best`, and each input carries `listed_paid_value` and `listed_payers`. A list over 16 is refused; an empty list counts nobody's pay; a failed read (an old registry without the method) fails closed like any other
- unit: `a_payer_list_makes_the_job_read_the_record_from_those_payers`, `the_payer_list_and_what_was_read_are_in_the_events`, `pay_from_payers_the_buyer_does_not_list_is_no_record`, `a_listed_manager_pay_counts_like_any_record`, `unpaid_money_counts_whoever_is_listed`, `min_paid_applies_to_the_listed_money`, `an_empty_payer_list_counts_nobodys_pay`, `a_payer_list_over_sixteen_is_refused`, `a_failed_listed_read_awards_nobody_and_refunds`
- sandbox: `a_washed_record_beats_the_plain_rule_but_not_the_buyers_payer_list`

#### Scenario: No payer list
- WHEN `payers` is left out
- THEN the reads, the pick and the events are exactly what they were before the option existed
- unit: `without_a_payer_list_the_reads_and_events_are_as_before`

#### Scenario: A slot without a bond
- WHEN a bidder attaches a few yocto on a slot that asks for no bond
- THEN its bid carries 1, like every other, and its newcomer premium is not divided
- unit: `yocto_on_a_slot_without_a_bond_do_not_cut_the_premium`, `a_slot_without_a_bond_gives_no_vouch`

#### Scenario: The skip list is public
- WHEN `award_best` is called
- THEN it emits `award_best_asked` with the weights, `min_paid`, the skip list and the bidders it reads
- unit: `the_skip_list_is_in_the_event`

#### Scenario: A child account id too long
- WHEN the winner's `t<n>.<job>` would pass 64 bytes
- THEN the callback refunds instead of panicking with the deposit left in the job
- unit: `a_child_id_too_long_refunds_instead_of_stranding`

#### Scenario: A huge record
- WHEN a record's money passes what the rule's counters hold
- THEN both numbers are scaled down together and the unpaid share is kept
- unit: `a_huge_record_keeps_its_unpaid_share`
- property: `weighing_never_hides_unpaid_money`

### Requirement: Agents bid and award through the tools
Worker agents decide their bid with the model (the junk bot bids 60% of max
without a model); the lead's picker compares bids with on-chain records.
- sandbox: `agents_story_with_mock_model` (job 1: the bot's low bids win; job 2: its record loses the award); unit: `picker_takes_the_lowest_bid_unless_its_record_is_bad`, `bidder_bids_only_on_its_skills`

### Requirement: Worker bonds
A slot MAY ask each bidder to stake NEAR (at most 5 NEAR) with its bid.
Losers SHALL get their bond back at the award; the winner SHALL get it
back when its work is paid, and the buyer SHALL keep it when the work is
not paid. A failed hire returns it. Bonds on a slot never awarded SHALL be
reclaimable after the window and SHALL be returned at settle. NEAR
attached to a changed bid SHALL join the bond, not be kept by the job.
- unit: `a_deposit_on_a_rebid_is_added_to_the_bond`, `a_bid_without_the_bond_is_refused`, `a_slot_cannot_ask_for_a_huge_bond`, `a_paid_worker_gets_its_bond_back`, `junk_loses_its_bond_to_the_buyer`, `a_failed_hire_returns_the_bond`, `bonds_on_a_slot_never_awarded_come_back`, `a_bond_cannot_be_reclaimed_while_the_job_runs`, `settle_returns_bonds_nobody_reclaimed`

### Requirement: Sealed bids
A slot MAY be sealed: while bidding is open a bidder SHALL commit only
`sha256("<price>:<salt>:<bidder>")`; after it closes, for the same length
of time, it reveals price and salt; the award SHALL wait for the reveal to
close and SHALL take only a revealed bid. An unrevealed bid's bond SHALL go
to the buyer. Open bids on a sealed slot are refused. A sealed bid SHALL
pay for its own larger storage (0.002 NEAR).
- unit: `a_sealed_bid_pays_for_its_own_storage`, `a_sealed_bid_with_the_open_deposit_is_refused`, `sealed_bids_hide_the_price_until_the_reveal`, `a_reveal_must_match_the_commit`, `no_reveal_while_bidding_is_open`, `no_award_before_the_reveal_closes`, `an_unrevealed_bid_cannot_win`, `an_open_bid_on_a_sealed_slot_is_refused`, `an_unrevealed_bond_goes_to_the_buyer`

### Requirement: Fire a failed worker
After the signed check fails a sub-job's delivery, and before any
redelivery, the sub-job's manager MAY fire the worker: the window SHALL end
at once, settle SHALL pay the worker nothing and return the whole pay to
the parent, and the worker's bond SHALL go to the buyer. A worker whose
work passed, or that has not been checked yet, SHALL NOT be fired; only the
manager can fire, and a fired worker cannot redeliver. The buyer can then
hire the runner-up in the same window.
#### Scenario: The returned pay hires the runner-up
- WHEN a fired (or refunded) child closes and sends its pay back
- THEN that much of the parent's allocation is free again, for a manager or a contractor, so the runner-up can be hired with it
- unit: `a_fired_childs_returned_pay_can_hire_the_runner_up`, `a_contractor_hires_again_after_a_refunded_sub_job`, `a_returned_allocation_can_be_hired_again`

#### Scenario: No bids once the job is done
- WHEN the job is approved, fired, delivered or settling, or its window closed
- THEN `bid` and `bid_sealed` are refused, so no bond is stranded on a slot that can never award
- unit: `a_bid_after_approve_is_refused`, `a_bid_while_settling_is_refused`

- unit: `the_buyer_fires_a_failed_worker_and_gets_everything_back`, `a_worker_whose_work_passed_cannot_be_fired`, `a_worker_cannot_be_fired_before_any_check`, `only_the_buyer_can_fire`, `a_fired_worker_cannot_redeliver`

### Requirement: Newcomers vouch with a bigger bond
A bidder MAY attach more than the slot's bond; the job SHALL hold all of
it as that bid's bond. In the swarm a buyer divides the newcomer premium by
how many times the slot's bond the bid carries (its own bond plus an
underwriter's backing), read from the bid on-chain, so a backed
first-timer can outbid a cheaper fresh junk account.
- unit: `a_newcomer_that_stakes_more_bond_pays_less_premium`, `a_deposit_on_a_rebid_is_added_to_the_bond`

### Requirement: Underwriters back newcomers
A bidder SHALL be able to ask for a backer with `seek_backer(slot,
premium)` before bidding closes, offering a premium of at most half its
own bond. Any account but the bidder and the buyer MAY then `back` the bid
once with up to 5 NEAR. The bid's vouch is its bond plus the backing. If
the bid loses the auction, both are refunded at the award. If the work is
paid, the backer gets its NEAR back plus the premium, taken out of the
worker's bond. If the work is junk, the buyer keeps the bond and the
backing. A rolled-back hire charges no premium. In the swarm a newcomer
seeks a backer instead of staking more of its own; a trusted agent of the
same skill (see records), often a rival bidder hedging, backs it only if its ask is at least 85% of the
skill's recent median win, and 5 points stricter after each backed junk.

#### Scenario: Paid, junk, lost
- contract: `a_backer_of_paid_work_gets_its_near_back_plus_the_premium`, `a_backer_of_junk_loses_its_near_to_the_buyer`, `a_backer_of_a_losing_bid_is_refunded_at_the_award`
- unit: `an_underwriter_backs_real_asks_and_tightens_after_junk`

#### Scenario: Misuse
- THEN refused
- contract: `nobody_backs_a_bid_that_did_not_ask`, `a_premium_above_half_the_bond_is_refused`, `a_bidder_cannot_back_itself`
