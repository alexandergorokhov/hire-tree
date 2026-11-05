# job-note

## Purpose
A worker whose delivery is verified and checked can be paid now instead of
at settle: the lead's own treasury buys the payout right (the "Job Note") for
99% of face value. Only the lead carries the risk, on workers it hired
itself; there is no outside buyer, so there is no victim for self-dealing.

## Requirements

### Requirement: Only a checked, verified, unsold, unpaid note is sellable
`quote_note()` SHALL be `amount - 1%` (fee rounds down) only when the job is
Checked, the receipt was verified, a checker is pinned, no check on this
job ever failed, the note is unsold, the payee was not paid, and no sale is
in flight; otherwise null.
- unit: `no_checker_no_note`, `a_failed_check_makes_the_note_unsellable`
- unit: `quote_after_verified_and_checked`, `quote_none_when_unchecked_or_unverified`; core: ledger tests

### Requirement: Sale to the pinned treasury, exact price, atomic
The treasury buys with `ft_transfer_call(job, quote, "buy_note")`. The job
SHALL accept only from the pinned treasury and only the exact quote, set
`payee = treasury` before any promise, then register and pay the worker. If
paying the worker fails, the sale is undone and the treasury is refunded.

#### Scenario: Sale
- THEN the worker gets the quote now, the treasury becomes payee and gets the full amount at settle
- unit: `treasury_buys_note_and_worker_is_paid_now`, `bought_note_pays_treasury_at_settle`

#### Scenario: Stranger buys
- THEN refunded in full, no state change
- unit: `stranger_cannot_buy_note`

#### Scenario: Wrong price
- unit: `wrong_price_refunded`

#### Scenario: Unchecked or unverified
- unit: `unchecked_note_refused`, `unverified_note_refused`

#### Scenario: Paying the worker fails
- THEN payee and note_sold are restored and the whole amount goes back to the treasury
- unit: `failed_worker_payment_undoes_sale`

#### Scenario: Double sale / sale in flight
- THEN the second is refunded; settle and dispute wait for the sale to finish
- unit: `second_sale_refused`, `settle_waits_for_note_sale`

#### Scenario: Sale callback lost
- WHEN `on_note_paid` never ran
- THEN anyone may `resume_note` after `RESUME_AFTER_SECS`; the sale is undone (the token refunded the treasury) and, if the worker was already paid from the job's tokens, that amount is booked as prepaid pay
- unit: `stuck_sale_worker_unpaid_is_undone`, `stuck_sale_worker_paid_from_job_counts_as_prepaid`, `resume_note_too_early`
