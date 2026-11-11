# pool

## Purpose
Collective purchasing: several clients club together to buy one job that
none of them wants to fund alone. A pool contract (one per purchase, its
deploy key deleted) collects the members' shares, posts the job through
the factory as its client, lets the members approve the work by stake
majority, and pays back what the job did not spend, pro rata.

## Requirements

### Requirement: Joining is capped by the target and the deadline
`ft_transfer_call(pool, amount, "join")` SHALL keep at most `target -
raised` and refund the rest; it SHALL refuse another token, any other msg,
a closed or launched pool, and a 17th member.
- unit: `joins_add_up_and_the_excess_is_refunded`, `joins_are_refused_from_the_wrong_token_with_a_bad_msg_or_late`, `the_pool_takes_at_most_sixteen_members`

### Requirement: Launch posts the whole budget as the client, once
`launch` SHALL need the full target, 0.06 NEAR and 240 Tgas; it prepays the
factory and sends the whole target in one `ft_transfer_call` with the job
terms fixed at `new`. If the factory keeps less than all of it, the pool is
`LaunchFailed` and members get their money back.
- unit: `launch_needs_the_full_target`, `launch_needs_the_post_deposit`, `launch_needs_gas`, `launch_prepays_and_posts_the_whole_budget_as_client`, `a_refused_post_is_a_failed_launch_and_members_get_it_back`, `no_answer_from_the_token_is_a_failed_launch`

### Requirement: The pool binds only its own job
`bind(n)` SHALL accept factory job `n` only if the factory's record names
this pool as the client, with the pool's budget, in state Posted.
- unit: `bind_takes_only_a_job_whose_client_is_this_pool`, `bind_needs_a_posted_pool`

### Requirement: Members approve by stake majority
`vote_approve` SHALL be members-only; when voters hold more than half the
stake the pool calls the job's `approve` once; a refused approve may be
resent by a later vote.
- unit: `the_pool_approves_once_more_than_half_the_stake_votes`, `a_refused_approve_can_be_resent_by_a_later_vote`, `outsiders_cannot_vote`

### Requirement: Finish only after the job is gone; claims are pro rata
When the bound job reports its refund (`job_refund`, sent by the root
after its refund transfer settled), `finish` SHALL share exactly that, with
no balance read. Otherwise `finish` SHALL share the pool's balance only
when the job account no longer exists or reports Settled, or joining
closed short of the target, or the launch failed, and never more than the
reported refund or, without one, what members put in (a refused
`ft_transfer_call` can show tokens for a block). Each member
`claim`s `payout x stake / raised` less what it already claimed; a failed
transfer can be retried. Anyone may `finish` again after `Done` (no claim in
flight) to recount: `payout = paid out + balance now`, so tokens that
arrive late are shared too.
- unit: `a_reported_refund_is_shared_exactly_with_no_read`, `only_the_bound_job_reports_a_refund`, `a_read_never_counts_more_than_members_put_in`, `a_settled_job_counts_as_gone`, `a_settled_root_tells_its_client_what_came_back`, `finish_waits_until_the_job_account_is_gone`, `an_open_pool_cannot_finish_before_joining_closes`, `a_pool_that_missed_its_target_refunds_after_closing`, `an_unreadable_balance_goes_back_to_the_previous_state`, `members_claim_pro_rata_once`, `a_failed_claim_can_be_retried`, `outsiders_cannot_claim`, `tokens_arriving_after_finish_are_shared_by_a_recount`, `a_recount_waits_for_claims_in_flight`

#### Scenario: Three clients buy one job
- GIVEN three clients each join with 30 of a 90 fUSD target
- WHEN the pool launches, the lead finishes the job (fee cap 9, work 60, every part paid so the whole fee is earned) and two members vote to approve
- THEN the job deletes itself, 21 comes back to the pool, and each member claims 7 (an unpaid part would also send its share of the fee back to the pool: the root's fee rule, `job-tree`)
- sandbox: `agents_story_with_mock_model` (job 3)

### Requirement: A stuck pool can be resumed without paying twice
At most one step that waits on a callback SHALL be in flight: a launch, a
finish, or one claim (`claim` refuses while one is in flight). Each carries
an attempt number. If its callback never ran, anyone SHALL be able to call
`resume` after `RESUME_AFTER_SECS` (600 s) with 40 Tgas. It bumps the
attempt first, so the late original callback is refused and changes
nothing.
- finish (reads only, no money moves): the pool goes back to its state
  before `finish`; finish may run again.
- launch: the pool's real token balance decides. Below the target: the
  factory kept the budget, Posted. Otherwise it came back, LaunchFailed, and
  members get it back.
- claim: had the transfer happened the pool holds `payout - paid_out`; at
  least that plus the claim means it did not, and the claim is undone so the
  member may claim again. Tokens above that are never counted here (a
  recount shares them). Known limit: a gift of at least the claim arriving
  while the claim is stuck makes a paid claim look unpaid; the member is then
  paid twice out of that gift, never out of other members' shares.
- No answer from the token: the step stays stuck and `resume` can run again.

#### Scenario: Too early, nothing stuck, too little gas
- unit: `resume_waits_until_the_step_is_stuck`, `resume_needs_a_step_in_flight`, `resume_needs_gas`

#### Scenario: Stuck launch
- unit: `a_stuck_launch_whose_budget_left_resumes_to_posted`, `a_stuck_launch_whose_budget_came_back_resumes_to_launch_failed`, `a_late_launch_answer_after_resume_changes_nothing`, `an_unanswered_resume_stays_stuck_and_can_retry`

#### Scenario: Stuck finish
- WHEN a finish's callbacks never ran, and a member claims after the resume
- THEN the pool is back where it was, and the lost round's balance read cannot land later and count the paid claim twice
- unit: `a_stuck_finish_goes_back_and_can_run_again`, `a_stale_balance_from_a_lost_finish_cannot_inflate_the_payout`

#### Scenario: Stuck claim
- unit: `one_claim_in_flight_at_a_time`, `a_stuck_claim_that_was_paid_is_not_paid_again`, `a_stuck_claim_that_failed_can_be_claimed_again`, `a_late_claim_answer_after_resume_changes_nothing`
