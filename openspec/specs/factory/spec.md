# factory

## Purpose
`hiretree.<net>` creates root jobs from the global job contract, so a client
posts and funds a job in one `ft_transfer_call`, and every job shares the
pinned signer, checker and token.

## Requirements

### Requirement: Post a job in one transfer
`ft_transfer_call(factory, budget, msg)` with msg
`{"manager", "spec_hash", "window_secs", "treasury"?}` SHALL create
`job-<n>.<factory>` with `use_global_contract(code_hash)`, init it with
`client = sender` and the factory's pinned signer/checker, register it on the
token and forward the budget with `fund`. The client SHALL first prepay
`POST_NEAR` (0.06 NEAR) with `prepay_post`; each post uses one credit (the
root's storage and registration), so posting spam costs the spammer. The
budget SHALL be at least 1 fUSD. A failed post gives the credit back.

#### Scenario: No prepaid NEAR, or dust budget
- THEN refunded in full, no account created
- unit: `post_without_prepaid_near_refunded`, `one_credit_one_post`, `dust_budget_refunded`, `failed_post_gives_the_credit_back`

#### Scenario: Post
- unit: `post_job_creates_and_funds_root`; sandbox: `scene_full_tree_on_local_node`

#### Scenario: Bad message, wrong token, zero amount
- THEN refunded in full, no account created
- unit: `bad_post_msg_refunded`, `wrong_token_refunded`

#### Scenario: Create or fund fails
- THEN `on_posted` returns the whole budget as unused (token refunds the client) and marks the job failed
- unit: `failed_post_refunds_client`

#### Scenario: Unreadable token answer
- THEN counted as all used (never refund from other clients' tokens)
- unit: `unreadable_token_answer_counts_as_all_used`

### Requirement: Pinned settings
Only the owner sets signer, checker and code hash; jobs created later use
the new values, existing jobs keep theirs.
- unit: `only_owner_changes_settings`
