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
`finish` SHALL share the pool's balance only when the job account no longer
exists (it deletes itself after settle, after its refund arrived), or
joining closed short of the target, or the launch failed. Each member
`claim`s `payout x stake / raised` once; a failed transfer can be retried.
- unit: `finish_waits_until_the_job_account_is_gone`, `an_open_pool_cannot_finish_before_joining_closes`, `a_pool_that_missed_its_target_refunds_after_closing`, `an_unreadable_balance_goes_back_to_the_previous_state`, `members_claim_pro_rata_once`, `a_failed_claim_can_be_retried`, `outsiders_cannot_claim`

#### Scenario: Three clients buy one job
- GIVEN three clients each join with 30 of a 90 fUSD target
- WHEN the pool launches, the lead finishes the job (fee 9, work 60) and two members vote to approve
- THEN the job deletes itself, 21 comes back to the pool, and each member claims 7
- sandbox: `agents_story_with_mock_model` (job 3)
