# settle

## Purpose
Close a job bottom-up: pay the payee, return the rest, prove the real token
balance is zero, delete the account to the parent. Never pay anyone twice,
never strand money, never get stuck for good (the parent is never left
waiting for a child that is already gone).

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
The balance-leg callback SHALL go on only when the token reports zero. A
root then deletes itself (beneficiary: the factory). A sub-job tells its
parent `child_closed(returned, paid)` and deletes itself (beneficiary: the
parent) only in the callback after that call succeeded, so the parent has
closed the child and taken its allocation back before the account is gone.
The notice and its callback are one chain; the track-record call stays
detached (a callback may not return a joint promise).

#### Scenario: Zero balance
- THEN the parent is sent `child_closed` with what came back; nothing is deleted until it answers; on success the account is deleted to the parent
- unit: `zero_real_balance_notifies_the_parent_then_deletes_on_its_answer`, `root_delete_goes_to_factory_without_child_closed`; sandbox: G1

#### Scenario: The parent's notice fails
- WHEN `child_closed` fails on the parent (a panic, out of gas)
- THEN the child is not deleted: it stays Settled with `close_pending` (not in flight) and emits `close_pending`; anyone may call `resume_close` at once, which sends the same notice again (the record is not sent again) and deletes after the parent answers
- unit: `a_failed_parent_notice_keeps_the_child_alive_until_resume_close`, `resume_close_needs_gas`, `parent_ack_rejects_outside_callers`

#### Scenario: The parent's answer never comes back
- WHEN the callback after `child_closed` never ran
- THEN `resume_close` is refused while the notice is in flight, and allowed after `RESUME_AFTER_SECS` (600 s); a late failure of an older notice changes nothing, and any success deletes
- unit: `resume_close_waits_while_the_notice_is_in_flight`, `resume_close_resends_a_notice_whose_answer_was_lost`, `a_late_failed_answer_cannot_stall_a_newer_notice`

#### Scenario: Only a settled sub-job with a pending notice
- unit: `resume_close_only_for_a_settled_child_with_a_pending_notice`, `resume_close_refused_before_settle`

#### Scenario: A repeated notice credits nothing
- WHEN a child already closed on the parent sends `child_closed` again (it never saw the first answer)
- THEN the call succeeds (so the child can delete) and changes nothing: no second credit, no bond paid twice; a stranger or a failed child is still refused
- unit: `a_repeated_child_closed_credits_nothing`, `unknown_caller_cannot_close_a_child`, `failed_child_cannot_close`

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

#### Scenario: An early pay sale (Job Note) follows the route too
- WHEN a checked worker with a route sells its payout early to the lead's treasury
- THEN the price goes `ft_transfer_call(verifier, price, msg = worker)` and the sale stands only if the Verifier kept it all; a refused payment undoes the sale and drops the route; the route cannot change while the sale is in flight
- unit: `a_routed_worker_is_paid_now_into_the_intents_verifier`, `a_refused_routed_note_payment_undoes_the_sale_and_drops_the_route`, `no_route_change_while_a_note_sale_is_in_flight`

#### Scenario: A guest agent cashes out over MCP
- WHEN a judge's agent routes its pay with the `payout_via` tool, wins a round, and calls `cash_out`
- THEN its 19.80 fUSD in the Verifier is swapped with the house solver at 0.2 wNEAR per fUSD and withdrawn: 3.96 wNEAR in its own account, 0 left in the Verifier
- sandbox: `a_judges_agent_competes_in_the_open_market`

#### Scenario: Swap and cash out on the real Verifier
- WHEN a job pays a worker through the official Verifier wasm on a local node
- THEN the worker and a solver swap with signed NEP-413 `token_diff` intents (one side alone is refused, a replay fails on the spent nonce) and the worker withdraws the token it swapped into
- sandbox: `pay_lands_in_the_intents_verifier_and_the_worker_swaps_it_atomically`
