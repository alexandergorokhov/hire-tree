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
The rule is `ht_core::pick`, run by the job contract (`award_best`), not by
the buyer: a lead or contractor names the rule (a blind buyer names 0 and 0)
and the contract reads the bids, bonds and records itself. The swarm also
runs the rule off-chain and logs an error, counted in `award_mismatches`, if
the contract's winner differs. A fired winner's runner-up is still hired by
hand.
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
seek an underwriter like an honest newcomer, and deliver fluent, empty text that
the scripted judge passes about a third of the time (each panel's
question rolls again). Before paying a pass from a worker that is not trusted
(at least 30 fUSD paid by at least 2 distinct managers, read from the
money-weighted record), the buyer SHALL dispute it on-chain; two more judge panels
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

### Requirement: Clients sign intents, leads fill them
Each round each client SHALL sign a job intent for one of three briefs;
leads fill them in order of their reserve fee (the lowest fills first and
picks workers first). The first to fill asks 0.5 fUSD more next round, the
last 0.5 less. The run logs each fill with its lead fee.

### Requirement: The lead's model plans the tree
The lead that fills an intent SHALL ask its model which catalog parts the
brief needs and the most it pays for each. The answer is read safely:
unknown or repeated skills are dropped, each max is clamped to the lead's
cap and to at least the cost floor, parts over the budget are dropped, and
an unreadable answer buys the whole catalog. The run logs each plan.
- unit: `the_planner_buys_only_what_the_brief_needs`, `a_bad_plan_is_clamped_or_falls_back`

### Requirement: A junk storm can be replayed in every arm
`hiretree swarm --storm R` (Makefile `STORM=R` with `make swarm-seeds`) SHALL
add 30 fresh sybil accounts at the start of round R, in the records, blind
and audit-all arms alike. Each has no record, bids about half the usual price
(50% of the part's max), stakes only the slot's bond, delivers polished junk
and, when caught, comes back under a fresh account like a junk bot. They are
spread over the six skills so that no slot passes the contract's 16-bid cap.
The run's JSON SHALL carry a per-round series: share of won parts that went
to junk, fUSD paid to junk and to the storm, the storm's total earnings, and
the fUSD a storm key holds beyond what settled jobs paid it (read from
on-chain balances and the factory record; expected 0, because pay stays in
job accounts until the check passes). Storm runs go to `runs/storm/` and are
summarised in `web/swarm-storm.json`, which the page charts; the normal seed
summary is unchanged, and `scripts/swarm-check.py` applies all its rules to
storm runs plus the series rules.

#### Scenario: The storm fits the market
- WHEN the storm roster is built
- THEN it has 30 uniquely named accounts and every skill, with them, stays within 16 bidders
- unit: `the_storm_floods_every_skill_without_passing_the_bid_cap`

#### Scenario: The series counts junk
- GIVEN four won parts, three to junk (two of them storm) and one honest
- THEN the round's row shows a junk share of 0.75 and the pay of passed junk and storm parts
- unit: `the_series_row_counts_junk_share_and_pay`

### Requirement: A storm can wash its records, and buyers can list whose pay counts
`hiretree swarm --storm R --wash` SHALL, before the storm sybils bid (at
round R, and again for any sybil that came back under a fresh account),
give each of them a record through real factory jobs among accounts the
attacker owns: for each of two attacker managers, an attacker client posts a
job naming that manager (up to 16 sybils per job), the manager hires the
sybils at 16 fUSD each, each delivers, the judge checks, the manager approves
and the sub-job settles and pays. The sybils then send the pay back to the
attacker's bank. The run's JSON SHALL carry a `wash` block read from the
chain: sybils washed, factory jobs and paid hires, the platform fee kept,
the fUSD put in less what came back, the NEAR the attacker's accounts lost,
and the parts the storm won and was paid for. `--wash` without `--storm` is
refused. `hiretree swarm --allow-list` (records arm only) SHALL make every
buyer pass `award_best` a `payers` list and read every record, audit and
runner-up pick through it: the three leads, plus each contractor whose record
from the leads' pay alone is trusted (30 fUSD from two payers). Without the
flag the calls, picks and run JSON are what they were. `make swarm-seeds
STORM=R WASH=1` writes `runs/wash/` (arms `rec` and `allow`, also `blind` and
`audit`) and `web/swarm-wash.json`.

#### Scenario: A wash crosses the plain rule but not the list
- WHEN a sybil is washed by two of its own managers
- THEN the plain rule trusts its record (32 fUSD, 2 payers), and a buyer who lists only the leads reads it as no paid money and no payers
- unit: `a_wash_crosses_the_trusted_rule_but_only_through_unlisted_payers`
- sandbox: `a_washed_record_beats_the_plain_rule_but_not_the_buyers_payer_list`

#### Scenario: The allowlist
- WHEN a records buyer builds its list
- THEN it holds the leads, then each contractor trusted on the leads' pay alone, once each, at most 16
- unit: `an_allowlist_names_the_leads_and_contractors_the_leads_trust`

#### Scenario: Without the flags nothing changes
- WHEN neither `--wash` nor `--allow-list` is given
- THEN `award_best` is asked exactly as before (no `payers` key), and options that make no sense together are refused before any chain work
- unit: `without_an_allowlist_the_award_asks_exactly_as_before`, `the_default_run_options_change_nothing`, `wash_needs_a_storm_and_the_allowlist_needs_records`

#### Scenario: A washing job pays and costs are reported
- THEN a washing job's budget covers its hires after the fee, and the cost block adds up and reports fUSD and NEAR
- unit: `a_washing_job_can_pay_its_hires_after_the_fee`, `wash_costs_add_up_and_report_in_fusd_and_near`
