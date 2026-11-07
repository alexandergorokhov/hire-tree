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
8 bidders; a second bid from the same account replaces its first.
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
