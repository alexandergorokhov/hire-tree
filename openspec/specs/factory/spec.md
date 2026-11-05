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
token and forward the budget with `fund`. The factory pays the account's
NEAR (refunded when the root job is deleted to the factory).

#### Scenario: Post
- unit: `post_job_creates_and_funds_root`; sandbox: `scene_full_tree_on_local_node`

#### Scenario: Bad message, wrong token, zero amount
- THEN refunded in full, no account created
- unit: `bad_post_msg_refunded`, `wrong_token_refunded`

#### Scenario: Create or fund fails
- THEN `on_posted` returns the whole budget as unused (token refunds the client) and marks the job failed
- unit: `failed_post_refunds_client`

### Requirement: Pinned settings
Only the owner sets signer, checker and code hash; jobs created later use
the new values, existing jobs keep theirs.
- unit: `only_owner_changes_settings`
