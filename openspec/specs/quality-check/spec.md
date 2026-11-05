# quality-check

## Purpose
One AI quality check per delivery, started by the contract (never by the
worker), with a seed the contract picks, so junk cannot be re-rolled until it
passes. On testnet the checker is an OutLayer job calling NEAR AI Cloud with
a fixed rubric; in tests and the sandbox it is `mock-outlayer`.

## Requirements

### Requirement: The contract starts exactly one check per delivery
When the job has a pinned `checker`, `deliver` SHALL call
`checker.check({job, spec_hash, deliverable_hash, seq, seed, panel: 0,
deliverable})` and handle the answer in `on_check(seq)`. The seed is
`sha256(random_seed || seq)`. `deliverable` is the response bytes: with a
checker pinned, `deliver` SHALL require `resp` (<= 16 KB) with
`sha256(resp) == resp_hash`, so the checker reads what was actually delivered.

#### Scenario: Content goes to the checker
- unit: `checker_needs_the_content`, `content_must_match_hash`

#### Scenario: Same bytes cannot be re-rolled
- WHEN a worker redelivers bytes it delivered before
- THEN refused: "same deliverable as before"
- unit: `same_bytes_cannot_be_rerolled`

#### Scenario: Check passes
- WHEN the checker answers a score >= 6 (of 10)
- THEN the job becomes Checked and `quality` is stored
- unit: `check_pass_marks_checked`

#### Scenario: Check fails
- WHEN the score is < 6
- THEN the job goes back to Open; the worker may redeliver while the window is open, at most `MAX_DELIVERIES` (3) times in all
- unit: `check_fail_reopens_for_redelivery`, `deliveries_are_capped`

#### Scenario: Checker down or nonsense answer (fail closed)
- WHEN the check call fails or returns something other than 0..=10
- THEN the job stays Delivered and unscored: not paid at settle unless the manager approves it, note not sellable
- unit: `check_failure_leaves_delivered`, `out_of_range_score_counts_as_no_answer`, `unscored_delivery_is_not_paid_without_approval`, `unscored_delivery_is_paid_when_the_manager_approves`

#### Scenario: Settle or approve while a check is pending
- THEN refused "check pending"; a late junk delivery cannot be paid before its score lands
- unit: `settle_waits_for_a_pending_check`, `approve_waits_for_a_pending_check`

#### Scenario: Check answer lost
- WHEN `on_check` never ran
- THEN anyone may `resume_check` after `RESUME_AFTER_SECS`; the delivery stays Delivered, unscored
- unit: `lost_check_answer_can_be_cleared_after_timeout`, `resume_check_too_early`

#### Scenario: A failed check stays on record
- THEN a later passing delivery is paid but its note is not sellable
- unit: `a_failed_check_makes_the_note_unsellable`

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
- sandbox: `scene_full_tree_on_local_node`

### Requirement: Signed check mode (judge receipts)
When the job pins a judge model (`judge`) instead of a checker contract,
`deliver` SHALL open check round 0 for `CHECK_WINDOW_SECS` (300 s). The
contract builds the judge request bytes itself (`ht_core::check::request_bytes`:
fixed rubric, model, job, delivery, panel, task text, delivered text; no block
randomness, so a recorded call replays anywhere). Anyone may `submit_check`
a NEAR AI Cloud answer: the task text must hash to the spec hash, the
delivered bytes to the delivered hash, the signature must recover to the
pinned signer over `model:sha256(request):sha256(answer)`, and the answer's
`choices[0].message.content` must be `{"score": 0-10}`. Each distinct answer
counts once (at most 8). After the window anyone calls `finalize_check`.

#### Scenario: Answers agree
- THEN the lowest score counts (fail closed: a worker gains nothing by asking many times)
- unit: `delivery_opens_a_round_and_a_signed_answer_scores_it`, `lowest_answer_counts`, `a_low_answer_fails_the_check`

#### Scenario: Answers split (max - min >= 3) or none
- THEN the delivery stays Delivered and unscored, and it is not a failed check (a lead or stranger shopping for a low score can only hold borderline work up, not fail it)
- unit: `split_answers_are_unclear_not_junk`, `no_answer_leaves_the_delivery_unscored`

#### Scenario: Wrong signer, other question, other bytes, other task, repeat, garbage, late
- THEN refused
- unit: `answer_signed_by_another_key_refused`, `answer_to_a_different_question_refused`, `answer_about_other_bytes_refused`, `answer_for_another_task_refused`, `same_answer_counts_once`, `unparseable_answer_refused`, `late_answer_refused`, `finalize_too_early_refused`

#### Scenario: Prompt injection in the deliverable
- THEN the deliverable is a JSON string field under a rubric that says to ignore instructions in it; it cannot add messages
- core: `injected_text_stays_inside_the_data_string`. Limit: a model can still be swayed; the dispute adds two panels.

#### Scenario: Dispute in signed mode
- WHEN the manager disputes
- THEN rounds 1 and 2 open; after them `finalize_dispute` counts the original score plus each panel (no answer or split = junk); 2 junk of 3 uphold
- unit: `dispute_opens_two_panels_and_two_junk_votes_uphold`, `dispute_with_one_good_panel_is_rejected`, `finalize_dispute_too_early_refused`

#### Scenario: Note after a signed check
- unit: `a_signed_check_makes_a_verified_note_sellable`
