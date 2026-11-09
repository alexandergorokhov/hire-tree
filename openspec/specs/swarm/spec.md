# swarm

## Purpose
Show a recursive labor market of AI agents: `hiretree swarm` (`make swarm`)
runs 3 leads and 36 worker agents with 10 strategies for 12 rounds on a
local node and writes `web/swarm.json` for `web/swarm.html`. Each round
every lead posts a job; a contractor that wins a part buys a sub-part from
a specialist out of its own pay, so trees grow up to 3 levels below the
root. Nothing decides the winners in advance.

## Requirements

### Requirement: Buyers weigh price against the record
A buyer (lead or contractor) SHALL pick the lowest bid after a penalty for
the share of unpaid jobs on the bidder's on-chain record, scaled by its own
record weight (price and budget leads and contractors 150 = x2.5 when all
unpaid; the quality lead 400). Ties go to the bidder with more paid jobs.
- unit: `pick_takes_the_lowest_bid_with_a_clean_record`, `unpaid_jobs_cost_more_than_a_low_bid_saves`, `a_quality_lead_weighs_the_record_more`, `ties_go_to_the_longer_clean_record`

### Requirement: Buyers learn a newcomer premium
Each buyer SHALL charge a bidder with no record a premium it learns from
its own history: a failed newcomer adds 100% (cap 150%), a newcomer that
delivered halves it, a round with no newcomer hired lets it fade by a
fifth. A caught junk bot returns under a fresh account (a Sybil) at most
twice, then quits.
- unit: `burned_leads_charge_newcomers_a_premium`, `the_newcomer_premium_rises_on_failure_and_fades`

### Requirement: Leads compete for the same workers
Three leads with different rules (price; quality; budget, which offers 75%
of max) SHALL buy every round, picking in turns; a worker SHALL take at
most one part per round, so a lead that picks later or offers less may go
unfilled.

### Requirement: Contractors grow the tree
A contractor that wins research, translate or sources SHALL put the
sub-part (sources, glossary, fetch) out for bids on its own job account at
most 60% of its price, award it out of its own pay, approve passed work so
the sub-agent is paid at once, and keep the margin. Its floor is 45%.
- unit: `trees_go_three_levels_deep`

### Requirement: Every agent reacts to the market
Steady, lean and sloppy agents SHALL cut their ask by 5 points after every
3 straight losing rounds (never below cost, 55%); adaptive agents +5 after
a win, -10 after a loss; undercut agents bid 3 under the last win; greedy
agents leave after 6 straight losses.
- unit: `roster_has_every_strategy_for_each_skill`

### Requirement: A model-driven bidder
The model agent SHALL ask the model for its price each round, given the
recent winning prices, its own on-chain record and its losing streak, and
SHALL clamp the answer to 40..=100 (an unreadable answer keeps the last
ask); it may bid below cost only to win a first job. Its reason is shown
with its bid. With the scripted test model it follows the median win, cuts
after two losses, tests higher after a win, and with no record after three
losses bids 50% as a loss leader.
- unit: `the_model_bidders_answer_is_read_safely`, `the_scripted_market_bidder_follows_the_market`

### Requirement: Platform fee, bonds and sealed bids
The swarm SHALL run with a 0.5% platform fee and report the fees kept.
Every slot SHALL ask a 0.05 NEAR bond (a junk winner loses it to its
buyer), and the quality lead's slots SHALL be sealed (commit, then reveal).

### Requirement: A judge that can be fooled, and audits that catch it
A polished-junk agent (one per top-level skill) SHALL bid low, stake a
newcomer's bond like an honest agent, and deliver fluent, empty text that
the scripted judge passes about a third of the time (each panel's
question rolls again). Before paying a pass from a worker with fewer than 3
paid jobs, the buyer SHALL dispute it on-chain; two more judge panels
answer and 2 failing votes of 3 refund the buyer and keep the bond. A
caught worker is replaced by the runner-up like a failed one. The run
reports how often polished junk fooled the first check, was caught, and was
still paid.
- unit: `polished_junk_fools_the_scripted_judge_about_a_third_of_the_time`, `roster_has_every_strategy_for_each_skill`; contract: `manager_disputes_sub_job`, `dispute_upheld_refunds_parent_at_settle`, `dispute_rejected_back_to_checked`

### Requirement: A control run with buyers blind to records
`make swarm-blind` SHALL run the same market with buyers that ignore
on-chain records: no record penalty, no newcomer premium and no audits.
The contract still checks every delivery, fires failed winners and keeps
their bonds. `scripts/swarm-compare.py` SHALL print junk wins, junk share
in the first and last third of rounds, fired winners and fUSD paid to junk
for both runs side by side, so the effect of the records is measured, not
claimed.
- unit: `a_blind_buyer_ignores_records_and_premium`
