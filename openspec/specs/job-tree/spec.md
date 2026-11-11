# job-tree

## Purpose
A client funds a root job; the manager (lead agent) hires workers. Each hire
creates a sub-account `t{n}.<job>` running the same global contract by code
hash, registers it on the token and funds it.

## Requirements

### Requirement: Init
Exactly one of client (root) or worker (sub-job); sub-job amount > 0; window
1 s..30 days; spec hash and signer well-formed; the account must be a
sub-account of its creator.
- unit: `root_init_sets_fields`, `sub_init_sets_worker_as_payee`, `init_must_be_called_by_parent_account`, `root_funding_below_the_fee_refunded`, `sub_with_zero_amount_refused`, `client_and_worker_both_refused`, `bad_spec_hash_refused`, `bad_signer_refused`, `window_too_long_refused`

### Requirement: Lead fee and client refund
A root's `amount` SHALL be the lead's fee cap, set by the client at post (by
the factory from a signed job offer's Dutch auction, or by a pool from its
terms). It is never hireable. The lead SHALL earn it pro rata to the parts
whose work passed, over at least a spend floor: at settle the manager gets
`fee * paid / max(hired, floor)` (rounded down) plus its bond, where
`floor` is half the root's work budget (`funded - fee`, what the client
funded for parts; the bond is not in it), rounded up in the client's
favour. The floor SHALL be set when the lead first hires: once the root has
a child or anything allocated, it SHALL refund any further `fund`, so a
late top-up cannot raise the floor and cut a fee already earned. Each job sums over its
closed children that its manager hired (a worker's contractors are inside
that worker's price; a hire rolled back never counts), each weighed by its
price (the hire's `amount`, not its sub-budget):
- a child whose own work passed (it settled after a passed check, or
  delivered with no check pinned) counts in `paid` and `hired`; an unscored
  delivery the manager approved, or a note whose dispute was upheld, is
  paid to the worker but does not pass;
- a child that did not pass counts in `hired` only, unless another such
  child with the same spec hash passed (the runner-up, or a rehire after a
  dispute): then it counts for nothing;
- plus the `paid` and `hired` the child reported in its `child_closed`
  (`fee`: `passed`, `paid`, `hired`) for the parts its manager hired under
  it, so the root counts every part the lead hired at any depth.

A notice without `fee` (older code) counts the child's own price, as passed
when it reported its worker paid. Settle already requires every child
closed, so the ratio is final.
Everything else, the unearned part of the fee included, goes back to the
client (or the pool that posted it). A root with no children, or none paid,
pays no fee. The bond rule is unchanged: the bond always goes back to the
manager.

#### Scenario: Root settles
- WHEN the client approves (or the window ends) and all children are closed
- THEN the manager is registered and paid its earned fee + bond, then the client is registered and paid the rest
- unit: `root_fee_and_bond_go_to_the_manager_the_rest_back_to_the_client`, `settle_root_sends_remainder_back_to_client`, `root_registers_the_client_before_refunding_it`; core: `root_split_pays_fee_and_bond_to_manager_rest_to_client`

#### Scenario: Every part fired or refunded
- WHEN a lead hired parts and every one closed unpaid (fired, refunded by a dispute, or junk)
- THEN it gets its bond back and no fee; the whole fee goes back to the client
- unit: `a_lead_whose_every_part_failed_earns_no_fee`; core: `a_root_with_no_paid_part_pays_no_fee`

#### Scenario: Some parts paid
- WHEN parts worth 30 passed and parts worth 10 did not (fee 10, budget 50: floor 20)
- THEN the lead earns 10 * 30 / 40 = 7.5 and the client gets the other 2.5 back; with thirds the fee rounds down and the client gets the unit
- unit: `the_lead_fee_is_earned_pro_rata_to_the_paid_parts`, `the_earned_fee_rounds_down_in_the_clients_favour`; core: `root_fee_is_pro_rata_and_rounds_down`, `earned_fee_never_overflows`
- sandbox: `scene_full_tree_on_local_node` (fee 10; parts 30 and 20 paid, 10 refunded by a dispute: the lead gets 8.333333, the client 41.666667)

#### Scenario: A spend floor
- WHEN the client posts 100 with a fee cap of 10 (work budget 90, floor 45) and the lead hires one part of 1 base unit that passes, and nothing else
- THEN the lead earns 10 * 1 / 45 USD = 0 and gets only its bond; a lead that spent 54 (60%) with every part passed earns the whole 10; one that spent 27 (30%) with every part passed earns 10 * 27 / 45 = 6 (60%)
- unit: `a_lead_that_hires_one_tiny_part_earns_almost_no_fee`, `a_lead_that_spends_over_half_the_work_budget_on_passed_parts_earns_the_whole_fee`, `a_lead_that_spends_under_half_the_work_budget_earns_the_fee_pro_rata_to_the_floor`; core: `the_fee_floor_is_half_the_work_budget`, `earned_fee_divides_by_at_least_the_spend_floor`, `the_spend_floor_rounds_up_in_the_clients_favour`; property: `earned_fee_is_exact_and_capped`, `the_fee_floor_is_half_the_work_budget_rounded_up`

#### Scenario: A late top-up cannot cut the fee
- WHEN the client posts 100 with a fee cap of 10, the lead hires a part of 54 that passes, and the client then funds the root another 1000 before approving
- THEN the root refunds the 1000 (the lead has already hired), the floor stays 45 and the lead still gets the whole 10; before the first hire a top-up is taken
- unit: `a_client_top_up_after_the_parts_passed_does_not_cut_the_lead_fee`, `a_client_can_top_up_the_root_before_the_lead_hires`

#### Scenario: No parts hired
- WHEN the root closes without a child (or only with hires that were rolled back)
- THEN no fee is paid, only the bond
- unit: `a_root_without_children_pays_no_lead_fee`, `a_rolled_back_hire_does_not_count_for_the_fee`

#### Scenario: A part paid early through a Job Note
- WHEN the lead's treasury bought a part's note after a passed check (the worker was paid at once)
- THEN the child reports it paid and passed, and it counts as paid for the fee
- unit: `a_note_bought_part_counts_as_paid_for_the_lead_fee`

#### Scenario: Paid but not passed
- WHEN the manager approved an unscored delivery, so its worker was paid
- THEN the child reports it paid but not passed, and it earns no fee
- unit: `unscored_delivery_is_paid_when_the_manager_approves`, `a_paid_part_that_did_not_pass_does_not_earn_the_fee`

#### Scenario: A sub-budget cannot hide failed parts
- WHEN the lead hires a part of price 1 with a sub-budget of 89, and inside it three parts of 29 that all fail (fee 10)
- THEN the sub-job reports 0 paid of 87 hired, and the lead earns 10 x 1 / 88; when the small part failed and the parts under it passed, it earns 10 x 87 / 88
- unit: `a_sub_budget_cannot_hide_failed_parts_from_the_lead_fee`, `a_sub_job_reports_the_parts_its_manager_hired_under_it`

#### Scenario: Weighed by price, not by sub-budget
- WHEN a part of price 1 with an unused sub-budget of 89 passed and a part of 10 failed (fee 10, budget 100)
- THEN the lead earns 10 x 1 / 45 (the 11 hired are under the floor of 45), not 10 x 90 / 100
- unit: `the_fee_weighs_each_part_by_its_price_not_its_sub_budget`

#### Scenario: A failed part redone
- WHEN a part failed and was fired, and the runner-up hired for the same task passed
- THEN the failed part counts for nothing; a failed task that was never redone still counts
- unit: `a_failed_part_whose_task_passed_again_does_not_count`

#### Scenario: The fee earned so far is public
- WHEN anyone reads the root's `get_job`
- THEN `lead_fee_earned` shows the fee settle would pay now (null on a sub-job), and settle emits `lead_fee` with the fee, the earned part, both sums and the floor
- unit: `the_view_shows_the_lead_fee_earned_so_far`

#### Scenario: Money is conserved with the fee rule
- core property: `money_prop` (a random root fee; the lead's take equals the rule exactly, never more than the fee)

#### Scenario: Budget below the fee
- WHEN the root is funded with less than its fee
- THEN refunded in full
- unit: `root_funding_below_the_fee_refunded`

### Requirement: Funding and bond
Only the client funds the root, only the parent funds a sub-job, only the
manager bonds the root; wrong token, zero amount, unknown msg and late
funding are refunded in full with no state change.
- unit: `client_funds_root`, `wrong_token_is_refunded_without_state_change`, `stranger_funding_root_is_refunded`, `unknown_msg_is_refunded`, `zero_amount_is_refunded`, `manager_posts_bond_on_root`, `bond_by_stranger_or_on_sub_is_refunded`, `sub_accepts_funding_only_from_parent`, `funding_after_window_is_refunded`

### Requirement: Hire
Manager only, Open, window open, <= 16 children, amount > 0, within the
unallocated budget (never the bond), >= 150 Tgas, >= 0.035 NEAR attached.
The job keeps `HIRE_KEEP_NEAR` (5 mNEAR: the child's token registration and
its own child record) and sends the rest to the child, so a job never pays
for hires from its own storage. State before promises; a create failure
rolls back. A failed fund call is ambiguous, so the hire stays in flight
until `resume_hire` (anyone, after `RESUME_AFTER_SECS`, at least 100 Tgas)
reads the child's own books (`get_funded`) and its real balance: funded
with its total -> Open, even if it already paid out and holds nothing;
never funded and holding nothing (or no child to answer, and nothing) ->
rolled back and the child closed. A resume whose answer never came back can
run again after `RESUME_AFTER_SECS`.
- unit: `resume_hire_opens_a_funded_child_that_already_paid_out`, `resume_hire_still_rolls_back_a_child_that_was_never_funded`, `resume_hire_needs_gas`, `a_stuck_resume_hire_can_run_again`, `resume_hire_rollback_fits_its_static_gas`
- unit: `hire_creates_child_from_global_contract`, `non_manager_cannot_hire`, `hire_over_budget_refused`, `bond_cannot_be_hired_with`, `create_failure_rolls_back`, `fund_failure_waits_then_resume_rolls_back_and_closes_child`, `lost_hire_callback_with_funded_child_opens_it`, `lost_hire_callback_with_empty_child_rolls_back`, `unknown_child_balance_keeps_waiting`, `resume_hire_too_early`, `late_create_callback_after_resume_refused`, `partial_funding_keeps_books_right`, `funding_callback_cannot_run_twice`
- sandbox: `g4_global_contract_and_hire_create_sub_accounts`

### Requirement: A worker can become a buyer (contractor mode)
A sub-job's own worker SHALL be able to put parts out for bids, award and
hire inside its sub-job, paying for each sub-hire out of its own pay: its
pay drops by the sub-job's total, it must keep part of its pay, and it
cannot add a sub-budget. The sub-job's manager SHALL be the worker, so it
approves or disputes its sub-agents. Pay that a closed sub-job sends back
(unspent or refunded) SHALL become the worker's pay again; a failed hire
SHALL restore it. The worker cannot bid on its own job, and its note SHALL
not be sold while its sub-jobs are open. A stranger cannot hire.
- unit: `a_worker_hires_sub_agents_out_of_its_own_pay`, `unspent_or_refunded_sub_pay_goes_back_to_the_contractor`, `a_contractors_note_waits_for_its_sub_jobs`, `a_contractor_cannot_bid_out_its_whole_pay`, `a_contractor_cannot_bid_on_its_own_slot`, `a_failed_contractor_hire_restores_its_pay`, `a_stranger_cannot_hire_in_a_sub_job`, `a_contractor_carves_its_own_pay_and_gets_refunds_back`
