# dispute

## Purpose
Let the manager challenge a sub-job delivery: two more checks with fresh
seeds; 2 junk votes of 3 refund the job's pay to the parent.

## Requirements

### Requirement: Who and when
Only the manager, on a sub-job, while the window is open, on a Delivered or
Checked job with a pinned checker, once per job, with no note sale or check
in flight. The manager SHALL send the delivered bytes (`resp`, whose sha256
must equal the delivered hash) so panels 1 and 2 read the same content.
- unit: `dispute_waits_for_a_pending_check`, `manager_disputes_sub_job`, `stranger_cannot_dispute`, `dispute_after_window_refused`, `second_dispute_refused`, `root_dispute_not_built`

### Requirement: 2-of-3 decides
The original check's score (if any) plus two new scores (panels 1 and 2)
vote; a score < 6 is a junk vote. Two junk votes uphold the dispute: the job
becomes Refunded and settle sends the payee nothing (pay returns to the
parent). Otherwise it becomes Checked.

#### Scenario: Upheld
- unit: `dispute_upheld_refunds_parent_at_settle`

#### Scenario: Rejected
- unit: `dispute_rejected_back_to_checked`

#### Scenario: A check does not answer (fail closed)
- THEN that vote counts as junk
- unit: `missing_dispute_votes_count_as_junk`

#### Scenario: Bought note then disputed
- WHEN the lead's treasury bought the note and the lead disputes
- THEN an upheld dispute costs the treasury the note's face value; the pay goes up the tree, so the lead cannot gain more than it paid
- unit: `dispute_after_own_note_purchase_only_hurts_treasury`

### Requirement: Stuck dispute
If the dispute callback never ran, anyone may `resume_dispute` after
`RESUME_AFTER_SECS`; it counts as upheld (never as rejected, so the worker
gains nothing by making the callback fail). A late callback is refused.
- unit: `stuck_dispute_resume_upholds_never_rejects`
