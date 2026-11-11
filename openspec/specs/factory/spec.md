# factory

## Purpose
`hiretree.<net>` creates root jobs from the global job contract, so a client
posts and funds a job in one `ft_transfer_call`, and every job shares the
pinned signer, checker and token.

## Requirements

### Requirement: Post a job in one transfer
`ft_transfer_call(factory, budget, msg)` with msg
`{"manager", "spec_hash", "window_secs", "treasury"?, "lead_fee"?}` SHALL create
`job-<n>.<factory>` with `use_global_contract(code_hash)`, init it with
`client = sender` and the factory's pinned signer/checker, register it on the
token and forward the budget with `fund`. The client SHALL first prepay
`POST_NEAR` (0.06 NEAR) with `prepay_post`; each post uses one credit (the
root's storage and registration), so posting spam costs the spammer. A
prepay SHALL be at least 0.001 NEAR (a credit is a storage entry for any
account id the caller names). The budget SHALL be at least 1 fUSD. A
failed post gives the credit back.

#### Scenario: No prepaid NEAR, or dust budget
- THEN refunded in full, no account created
- unit: `post_without_prepaid_near_refunded`, `one_credit_one_post`, `dust_budget_refunded`, `failed_post_gives_the_credit_back`, `a_dust_prepay_is_refused`

#### Scenario: Post
- unit: `post_job_creates_and_funds_root`; sandbox: `scene_full_tree_on_local_node`

#### Scenario: Bad message, wrong token, zero amount
- THEN refunded in full, no account created
- unit: `bad_post_msg_refunded`, `wrong_token_refunded`

#### Scenario: Lead fee
- WHEN the msg names a `lead_fee`
- THEN it becomes the root's `amount` (the lead's fee cap, earned pro rata to the paid parts at settle: `job-tree`); a fee above the budget is refunded
- unit: `lead_fee_goes_into_the_root_init_and_cannot_exceed_the_budget`

#### Scenario: Create or fund fails
- THEN `on_posted` returns the whole budget as unused (token refunds the client) and marks the job failed
- unit: `failed_post_refunds_client`

#### Scenario: Unreadable token answer
- THEN counted as all used (never refund from other clients' tokens)
- unit: `unreadable_token_answer_counts_as_all_used`

#### Scenario: A window shorter than a minute is refused
- WHEN a post asks for `window_secs` below 60
- THEN the budget is refunded, so no root is created whose window closes before its fund arrives (and a root that was never funded still settles without its fee)
- unit: `a_window_shorter_than_a_minute_is_refused`, `a_root_that_was_never_funded_still_settles`, `a_root_never_funded_settles_without_its_fee`

### Requirement: Pinned settings
Only the owner sets signer, checker and code hash; jobs created later use
the new values, existing jobs keep theirs.
- unit: `only_owner_changes_settings`

### Requirement: Worker track records
Every sub-job SHALL report its worker's outcome to its registry (the
factory, passed down the tree at hire) when it deletes: delivered, checked,
junk (dispute upheld), paid, and the check score. The factory SHALL accept
reports only from accounts under `.<factory>` (only the factory and its
keyless jobs can create those). `get_record(account)` returns the counts.
Records are evidence, not proof: a lead can hire itself to build one.

#### Scenario: A finished sub-job reports
- unit: `deleted_sub_reports_the_workers_record_to_the_registry`, `root_reports_no_record`, `hire_passes_the_registry_down`; factory: `jobs_of_this_factory_build_a_workers_record`; sandbox: `scene_full_tree_on_local_node` (researcher checked + paid, junk worker junk)

#### Scenario: A stranger writes a record
- THEN refused
- factory: `strangers_cannot_write_records`

#### Scenario: Records weigh money and distinct payers
- WHEN a job reports its worker
- THEN the record adds the job's amount to `paid_value` or `unpaid_value`, and counts each paying manager once in `payers` (a worker paid by itself is not counted)
- unit: `a_record_counts_money_and_distinct_payers`; swarm: `records_are_weighed_by_money_and_payers`

#### Scenario: A bounded breakdown of who paid
- WHEN a job reports a paid worker with a payer
- THEN the factory also keeps, per worker, a breakdown of paid money by payer: at most 16 entries, each at least 1 fUSD to get its own entry (dust and pay from the worker itself or without a payer go to `other`), so the entries and `other` always add up to `paid_value`. A payer that has an entry adds to it. With the list full, a new payer takes the place of the smallest entry (the oldest of equals) only by paying strictly more than it; that entry's money moves to `other`. So a sybil spraying small payments from many accounts displaces nobody, and displacing an entry costs more than the entry, paid to the worker. `get_payers(account)` returns the breakdown
- unit: `the_payer_breakdown_adds_up_to_paid_value`, `dust_never_takes_a_place`, `the_breakdown_is_bounded`, `spraying_small_payers_cannot_evict_an_honest_one`, `a_full_list_is_displaced_only_by_paying_more_than_its_smallest`, `an_entry_only_grows_and_is_never_the_one_displaced_by_its_own_top_up`

#### Scenario: A record as seen by a buyer who lists its payers
- WHEN anyone calls `get_record_from(account, payers)` with at most 16 managers
- THEN the record comes back with `paid_value` and `payers` counting only the listed managers' entries (a listed payer that lost its entry counts as nothing: the read errs toward distrust; a payer listed twice counts once); every other field, unpaid money included, is the whole record's. A longer list is refused. One small record comes back however long the breakdown is
- unit: `a_read_from_listed_payers_counts_only_their_pay`, `unpaid_money_stays_whoever_is_listed`, `listing_more_than_sixteen_payers_is_refused`

#### Scenario: A hire nobody answered
- GIVEN a worker hired with `hire` (no bid) that never delivered and hired nobody
- WHEN the sub-job lapses and reports
- THEN it reports amount 0: `unpaid_value` does not move. A hire needs no consent, so it must not be able to stain a record. A worker that won the job with its own bid (the child is created with `bid: true`) and never delivered is marked with the whole pay
- unit: `a_hire_nobody_bid_for_leaves_no_unpaid_mark`, `a_bid_winner_that_never_delivers_is_marked_unpaid`, `an_award_tells_the_child_it_was_won_by_a_bid`

### Requirement: Platform fee
The owner SHALL be able to set a platform fee of at most 1% (100 bps). On
each post the factory SHALL keep the fee and fund the job with the rest;
a failed post SHALL refund the whole budget, fee included. Only the owner
withdraws fees; a failed withdrawal puts them back. The fee is 0 unless
set (the swarm runs at 0.5%).
- unit: `the_platform_fee_comes_off_the_top_and_is_kept_only_when_posted`, `the_fee_is_capped_at_one_percent`, `only_the_owner_withdraws_fees`

### Requirement: Signed job intents
A client SHALL be able to keep fUSD at the factory (`ft_transfer_call`
with msg "deposit"), register one ed25519 intent key, and sign job intents
off-chain: budget, spec hash, window, nonce, and a lead fee that rises
linearly (a Dutch auction) from 0 at `starts_ns` to `max_lead_fee` at
`ends_ns`. Any lead but the client SHALL be able to fill an intent with
`fill_intent`: the factory checks the signature over
`hiretree-intent:<json>`, that it names this factory, that it has not
expired, been cancelled or filled (a nonce is used once), and that the
deposit covers the budget; it then posts the job with the filler as
manager and the current fee as lead fee (a cap: the lead earns it pro
rata to its paid parts, as on any root). A failed post SHALL return the
budget to the deposit, not the wallet. The client MAY cancel a nonce
(a nonce not yet spent attaches 0.001 NEAR for its storage) and
withdraw its deposit. An intent MAY name `leads`: then only those accounts
SHALL fill it. The filler becomes the job's manager and spends its budget,
so an intent without `leads` trusts whoever fills first. This is intent-based settlement: the client signs
what it wants and pays nothing until a lead commits to deliver it.

#### Scenario: A lead fills an intent
- unit: `the_lead_fee_rises_from_zero_to_the_max`, `a_lead_fills_a_signed_intent_and_the_job_is_posted`

#### Scenario: Replays, tampering and the wrong factory
- THEN refused
- unit: `an_intent_fills_only_once`, `a_tampered_intent_is_refused`, `an_intent_for_another_factory_is_refused`, `an_expired_intent_is_refused`, `a_cancelled_intent_is_refused`, `an_intent_above_the_deposit_is_refused`, `the_client_cannot_fill_its_own_intent`

#### Scenario: A failed post
- unit: `a_failed_intent_post_goes_back_to_the_deposit`

#### Scenario: Named leads
- GIVEN an intent signed with `leads: [lead, lead2]`
- WHEN a stranger fills it
- THEN refused; lead2 fills it and becomes the manager
- unit: `an_intent_that_names_its_leads_is_filled_only_by_them`, `a_named_lead_fills_the_intent`, `an_intent_can_name_its_leads`

#### Scenario: Storage is paid for
- WHEN a caller cancels a new nonce with no deposit
- THEN refused; cancelling the same nonce again needs none
- unit: `cancelling_an_intent_pays_for_its_storage`, `cancelling_twice_pays_once`
