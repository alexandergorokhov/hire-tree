# settle

## Purpose
Close a job bottom-up: pay the payee, return the rest, prove the real token
balance is zero, delete the account to the parent. Never pay anyone twice,
never strand money, never get stuck for good.

## Requirements

### Requirement: Settle only when the job is finished
Settle SHALL run only after the window ended or the job was approved, with no
open or in-flight child, and not while disputed.

#### Scenario: Window still open
- WHEN anyone calls `settle` before `window_end` and without approval
- THEN it panics "window is still open"
- unit: `settle_before_window_refused`

#### Scenario: Open children
- WHEN a child is open or a hire is in flight
- THEN settle panics "children still open"
- unit: `settle_with_open_children_refused`, `settle_with_hire_in_flight_refused`

#### Scenario: Double settle
- WHEN settle is called while settling
- THEN it panics "illegal transition"
- unit: `double_settle_refused`

### Requirement: One transfer in flight at a time
Settle SHALL run in legs: payee leg (register the payee on the token, then
pay `amount` only if the work was delivered), rest leg (the rest to the
parent; root: to the manager), balance leg (`ft_balance_of`). Each leg
carries an attempt number.

#### Scenario: Delivered sub-job
- WHEN a delivered sub-job (pay 30, holding 35) settles
- THEN only `storage_deposit(worker)` + `ft_transfer(worker, 30)` are in flight
- AND after it succeeds, `ft_transfer(parent, 5)` runs
- unit: `settle_delivered_sub_registers_then_pays_worker_first`, `payee_paid_then_rest_leg_returns_to_parent`

#### Scenario: Unregistered payee
- WHEN the worker never registered on the token
- THEN settle registers it and pays it anyway
- sandbox: `g1_delete_only_after_confirmed_transfer_and_zero_balance`

#### Scenario: Payee transfer fails
- WHEN the payee transfer fails
- THEN its tokens stay on the job, the rest leg still runs, the job returns to its pre-settle status, and the next settle pays only the payee
- unit: `payee_failure_still_returns_rest_then_resettle_pays_only_payee`

#### Scenario: Rest transfer fails
- WHEN the rest transfer fails after the payee was paid
- THEN the next settle sends only the rest
- unit: `rest_failure_never_pays_payee_twice`

### Requirement: Delete only at a confirmed zero balance
The account SHALL be deleted only in the balance-leg callback when the token
reports zero; the beneficiary is the parent; the parent is told via
`child_closed(returned)`.

#### Scenario: Zero balance
- THEN the account is deleted to the parent and `child_closed` reports what came back
- unit: `zero_real_balance_deletes_and_notifies_parent`; sandbox: G1

#### Scenario: Tokens above the ledger
- WHEN the balance check reads more than the ledger holds (stray tokens, or a refused `ft_transfer_call` that the token will refund a block later)
- THEN the surplus is never booked; a sweep leg sends it up the tree, it counts only if that transfer succeeds, then the balance is checked again
- unit: `stray_tokens_are_swept_up_then_the_job_deletes`, `phantom_surplus_from_a_refunded_transfer_call_cannot_lock_settle`; sandbox: G1

### Requirement: Stuck settle recovery
If a leg's callback never ran (out of gas, panic), the manager SHALL be able
to call `resume_settle` after `RESUME_AFTER_SECS` (600 s), and anyone after
`RESUME_ANYONE_SECS` (1 day). It bumps the attempt
(so the late original callback is refused), reads the real balance and
decides from it alone whether the leg's transfer happened. Because only one
transfer is ever in flight, the answer is unambiguous; tokens above the
ledger are never booked here. Known limit: a refused `ft_transfer_call` in
flight at the moment of the read can make a paid leg look unpaid; that is why
the payee cannot choose the moment during the first day.

#### Scenario: Too early
- unit: `resume_before_timeout_refused`, `resume_outside_settle_refused`, `stranger_cannot_resume_in_the_first_day`, `anyone_can_resume_after_a_day`

#### Scenario: Leg happened
- WHEN real balance equals the ledger's expected balance
- THEN the leg counts as done and settle moves on without paying twice
- unit: `resume_payee_leg_that_happened_moves_on_without_paying_twice`, `resume_rest_leg_then_balance_leg_deletes`

#### Scenario: Leg did not happen
- unit: `resume_payee_leg_that_failed_keeps_payee_tokens`, `resume_never_books_extra_tokens`, `resume_of_a_lost_sweep_restarts_settle`

#### Scenario: Late original callback
- WHEN the original callback arrives after a resume
- THEN it panics "stale settle callback" and changes nothing
- unit: `late_original_callback_cannot_act_after_resume`, `wrong_leg_callback_refused`

#### Scenario: Token does not answer
- THEN the job stays Settling and resume can be retried
- unit: `resume_with_no_token_answer_stays_stuck_and_can_retry`

### Requirement: The tree folds and money is conserved
- unit: `two_level_tree_folds_bottom_up`; core property: `money_prop`; sandbox: `two_level_tree_folds_bottom_up_and_money_rule_holds`

### Requirement: The payee can take its pay through NEAR Intents
The payee may route its pay to the NEAR Intents Verifier before settle
(`set_payout_via`). The payee leg is then `ft_transfer_call(verifier, pay,
msg = payee)`, so the pay lands as the payee's balance in the Verifier. It
counts as paid only if the Verifier kept all of it; otherwise the route is
dropped and the next settle pays plainly.

#### Scenario: Routed pay
- WHEN the payee set a route and the job settles
- THEN the Verifier is registered on the token and receives the pay for the payee; no plain transfer
- unit: `the_payee_can_route_its_pay_to_the_intents_verifier`, `paid_only_when_the_verifier_kept_the_whole_amount`, `the_payee_can_clear_the_route`

#### Scenario: Only the payee, only before settle
- unit: `only_the_payee_picks_the_route`, `the_route_is_fixed_once_settle_starts`

#### Scenario: Refused or failed deposit
- THEN the leg is not paid, the route is dropped, and resettle pays by plain transfer
- unit: `a_refused_deposit_falls_back_to_a_plain_transfer_on_resettle`, `a_failed_call_is_not_paid`

#### Scenario: Swap and cash out on the real Verifier
- WHEN a job pays a worker through the official Verifier wasm on a local node
- THEN the worker and a solver swap with signed NEP-413 `token_diff` intents (one side alone is refused, a replay fails on the spent nonce) and the worker withdraws the token it swapped into
- sandbox: `pay_lands_in_the_intents_verifier_and_the_worker_swaps_it_atomically`
