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
- unit: `bid_above_max_refused`, `manager_cannot_bid`, `late_bid_refused`, `bid_after_award_refused`, `a_new_bid_replaces_the_bidders_old_one`, `bids_per_slot_are_capped`

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
