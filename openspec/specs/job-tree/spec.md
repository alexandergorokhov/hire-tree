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
- unit: `root_init_sets_fields`, `sub_init_sets_worker_as_payee`, `init_must_be_called_by_parent_account`, `root_with_amount_refused`, `sub_with_zero_amount_refused`, `client_and_worker_both_refused`, `bad_spec_hash_refused`, `bad_signer_refused`, `window_too_long_refused`

### Requirement: Funding and bond
Only the client funds the root, only the parent funds a sub-job, only the
manager bonds the root; wrong token, zero amount, unknown msg and late
funding are refunded in full with no state change.
- unit: `client_funds_root`, `wrong_token_is_refunded_without_state_change`, `stranger_funding_root_is_refunded`, `unknown_msg_is_refunded`, `zero_amount_is_refunded`, `manager_posts_bond_on_root`, `bond_by_stranger_or_on_sub_is_refunded`, `sub_accepts_funding_only_from_parent`, `funding_after_window_is_refunded`

### Requirement: Hire
Manager only, Open, window open, <= 16 children, amount > 0, within the
unallocated budget (never the bond), >= 150 Tgas. State before promises;
create or fund failure rolls back and closes an unfunded child.
- unit: `hire_creates_child_from_global_contract`, `non_manager_cannot_hire`, `hire_over_budget_refused`, `bond_cannot_be_hired_with`, `create_failure_rolls_back`, `fund_failure_rolls_back_and_closes_child`, `partial_funding_keeps_books_right`, `funding_callback_cannot_run_twice`
- sandbox: `g4_global_contract_and_hire_create_sub_accounts`
