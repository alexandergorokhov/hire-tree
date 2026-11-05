# dispute

## Purpose
Let the manager challenge a sub-job delivery: two more checks with fresh
seeds; 2 junk votes of 3 refund the job's pay to the parent.

## Requirements

### Requirement: Who and when
Only the manager, on a sub-job, while the window is open, on a Delivered or
Checked job with a pinned checker, once per job, with no note sale in flight.
- unit: `manager_disputes_sub_job`, `stranger_cannot_dispute`, `dispute_after_window_refused`, `second_dispute_refused`, `root_dispute_not_built`

### Requirement: 2-of-3 decides
The original check's score (if any) plus two new scores (panels 1 and 2)
vote; a score < 6 is a junk vote. Two junk votes uphold the dispute: the job
becomes Refunded and settle sends the payee nothing (pay returns to the
parent). Otherwise it becomes Checked.

#### Scenario: Upheld
- unit: `dispute_upheld_refunds_parent_at_settle`

#### Scenario: Rejected
- unit: `dispute_rejected_back_to_checked`

#### Scenario: A check does not answer
- THEN that vote is missing; still 2 junk votes needed
- unit: `missing_votes_never_uphold`

#### Scenario: Bought note then disputed
- WHEN the lead's treasury bought the note and the lead disputes
- THEN an upheld dispute only costs the lead's treasury (collusion pays nothing)
- unit: `dispute_after_own_note_purchase_only_hurts_treasury`

### Requirement: Stuck dispute
If the dispute callback never ran, anyone may `resume_dispute` after
`RESUME_AFTER_SECS`; it counts as rejected. A late callback is refused.
- unit: `stuck_dispute_resumes_as_rejected`
