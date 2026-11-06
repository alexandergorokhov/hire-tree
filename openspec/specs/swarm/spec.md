# swarm

## Purpose
Show what emerges when many worker agents with different strategies
compete through the same contracts: `hiretree swarm` (`make swarm`) runs 20
agents with 7 strategies over many jobs on a local node and writes
`web/swarm.json` for `web/swarm.html`. Nothing decides the winners in
advance: agents bid by their own rule, the lead awards by price weighed
against each bidder's on-chain record, and the contract judges the work.

## Requirements

### Requirement: The lead weighs price against the record
The pick SHALL be the lowest bid after a penalty for the share of unpaid
jobs on the bidder's on-chain record (all unpaid: x2.5).
- unit: `pick_takes_the_lowest_bid_with_a_clean_record`, `unpaid_jobs_cost_more_than_a_low_bid_saves`

### Requirement: The lead learns a newcomer premium
A bidder with no record SHALL be charged a premium the lead learns from
its own history: a failed newcomer adds 100% (cap 150%), a newcomer that
delivered halves it, and a round with no newcomer hired lets it fade by a
fifth. A caught junk bot returns under a fresh account (a Sybil); the
premium is what prices that out, and its fading is why junk keeps probing.
- unit: `burned_leads_charge_newcomers_a_premium`, `the_newcomer_premium_rises_on_failure_and_fades`

### Requirement: Honest workers do not bid below cost, and winners are booked
Undercut and adaptive agents SHALL NOT bid below the cost floor (55% of
max). An agent that wins a part SHALL sit out the next round. A junk bot
is never paid and each fresh account costs it storage, so it SHALL quit
after 2 respawns.
- unit: `the_newcomer_premium_rises_on_failure_and_fades` (uses `COST_FLOOR`)

### Requirement: Every strategy competes for every skill
Steady, lean, greedy, adaptive, junk, sloppy and undercut agents bid for
each of research, translate and summary (20 agents).
- unit: `roster_has_every_strategy_for_each_skill`
