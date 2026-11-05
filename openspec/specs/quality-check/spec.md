# quality-check

## Purpose
One AI quality check per delivery, started by the contract (never by the
worker), with a seed the contract picks, so junk cannot be re-rolled until it
passes. On testnet the checker is an OutLayer job calling NEAR AI Cloud with
a fixed rubric; in tests and the sandbox it is `mock-outlayer`.

## Requirements

### Requirement: The contract starts exactly one check per delivery
When the job has a pinned `checker`, `deliver` SHALL call
`checker.check({job, spec_hash, deliverable_hash, seq, seed, panel: 0})` and
handle the answer in `on_check(seq)`. The seed is `sha256(random_seed || seq)`.

#### Scenario: Check passes
- WHEN the checker answers a score >= 6 (of 10)
- THEN the job becomes Checked and `quality` is stored
- unit: `check_pass_marks_checked`

#### Scenario: Check fails
- WHEN the score is < 6
- THEN the job goes back to Open; the worker may redeliver while the window is open, at most `MAX_DELIVERIES` (3) times in all
- unit: `check_fail_reopens_for_redelivery`, `deliveries_are_capped`

#### Scenario: Checker down or nonsense answer
- WHEN the check call fails or returns something other than 0..=10
- THEN the job stays Delivered (paid at settle, note not sellable) and an event says so
- unit: `check_failure_leaves_delivered`, `out_of_range_score_counts_as_no_answer`

#### Scenario: Stale or spoofed check answer
- WHEN `on_check` runs for an old seq, outside Delivered, or from another caller
- THEN it panics and nothing changes
- unit: `stale_check_answer_refused`, `check_callback_rejects_outside_callers`

#### Scenario: Worker cannot start or repeat a check
- THEN no public method starts a check except `deliver` (one per seq) and `dispute` (manager)
- unit: `deliver_schedules_exactly_one_check`

### Requirement: Receipt-only mode (G3 fallback)
When no checker is pinned, a delivery with a verified receipt SHALL go
straight to Checked (quality unknown); one without stays Delivered.
- unit: `no_checker_verified_delivery_is_checked`, `no_checker_unverified_stays_delivered`

### Requirement: Mock checker for the sandbox
`mock-outlayer` returns a score from a table the test sets per deliverable
hash (default 8), and counts calls.
- sandbox: `note_sale_and_dispute_on_local_node` (planned)
