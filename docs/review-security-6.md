# Security review 6: award_best, records, keys, intents

Scope: code changed since `45bd89a` (on-chain award rule, `ht_core::pick`,
function-call delivery keys, storm option, MCP `award_best`), plus a second
look at the money paths around them (fire and rehire, backers, intents,
records). Method: read as an attacker, write a failing test for each real
finding, fix, rerun. Base `01d97af`: 450 tests. Now: 471, `make test` and
`make lint` pass. No swarm, sandbox, network or GitHub Actions were run.

## Findings

| # | Severity | Finding | Fixed |
|---|---|---|---|
| 1 | High | A buyer awards in the last second and keeps the winner's bond and its backer's NEAR | yes |
| 2 | Medium | Any lead can write unpaid money on a rival's record with a `hire` nobody asked for | yes |
| 3 | Medium | On a slot with no bond, a few yocto erase the newcomer premium | yes |
| 4 | Medium | One washed cent ends the newcomer premium in `award_best` | opt-in (`min_paid`) |
| 5 | Medium | An open intent hands the budget to whoever fills first | opt-in (`leads`) |
| 6 | Low | Free calls fill the factory's storage (cancelled nonces, 1-yocto credits) | yes |
| 7 | Low | A record above 42.9 M fUSD loses its unpaid share in the award rule | yes |
| 8 | Low | The skip list was not in any event, though the docs said so | yes |
| 9 | Low | A child account id too long panics the callback and strands the 0.035 NEAR | yes |
| 10 | Info | Stale lines in THREATS.md (8 bids per slot; records "not weighted by money") | yes |

### 1. Late award takes the bond (High)
`contracts/job/src/lib.rs` `hire_inner` gives the child what is left of the
parent's window (`remaining_secs.max(1)`). `award` and `award_best` had no
floor, so a buyer could wait until one second before its window ends and
then award. The winner cannot deliver in a 1 s window; the child settles
unpaid; `release_bond(i, false)` sends the winner's bond and its backer's
NEAR to the buyer (up to 5 + 5 NEAR per slot, 16 slots), and the winner's
record takes the whole pay as unpaid.
Fix: `MIN_WORK_SECS = 60` (lib.rs:99). `awardable_slot` (lib.rs:1133)
refuses when less is left, for both `award` and `award_best`; the callback
checks again and refunds (lib.rs:1291). A slot that can no longer be
awarded returns its bonds at settle, as before.
Tests: `a_late_award_cannot_take_the_winners_bond`,
`a_late_award_best_is_refused`,
`an_award_best_callback_that_lands_too_late_refunds`,
`an_award_leaves_the_winner_at_least_a_minute`.

### 2. Record poisoning by an unasked hire (Medium)
`hire` needs no consent from the worker. A lead could post a job, hire a
rival for most of the budget, let it lapse and get the budget back; the
child then reported `paid: false` with the full amount and the factory added
it to the rival's `unpaid_value`, which `award_best` prices at up to x2.5.
Cost to the attacker: the platform fee and 0.06 NEAR.
Fix: the child now knows whether its worker won it with a bid (`new(...,
bid)`, set by `award_inner`, lib.rs:561 and 1514). At delete it reports the
amount only if the worker was paid, bid for the job, delivered, or hired
with the pay; otherwise 0 (lib.rs:2963). A bid winner that never delivers
is still marked.
Tests: `a_hire_nobody_bid_for_leaves_no_unpaid_mark`,
`a_bid_winner_that_never_delivers_is_marked_unpaid`,
`an_award_tells_the_child_it_was_won_by_a_bid`.

### 3. Vouch on a slot with no bond (Medium)
`core/src/pick.rs` `vouch` divided by `slot_bond.max(1)`. With no slot bond
a bidder that attached 1000 yocto carried "1000x the bond" and its newcomer
premium was divided to nothing.
Fix: with no slot bond every bid carries 1 (pick.rs:39).
Tests: `a_slot_without_a_bond_gives_no_vouch`,
`yocto_on_a_slot_without_a_bond_do_not_cut_the_premium`.

### 4. A washed cent is a record (Medium, opt-in fix)
`score` charges the newcomer premium only when the record is empty. A fresh
account can post a 1 fUSD job, hire and pay its own second account, and is
no longer a newcomer.
Fix: `award_best` takes an optional `min_paid`. Paid money below it counts
as none (lib.rs:1306): a clean thin record is a newcomer again, and a thin
record with unpaid money weighs all unpaid, so the floor never helps a bad
record. Left out, the rule is unchanged, so the swarm's results do not
move. The MCP tool passes it through.
Limit: washing `min_paid` itself costs only the fee and 0.06 NEAR. Listed
in Known limits.
Tests: `paid_money_below_min_paid_does_not_make_a_record`,
`a_thin_record_with_unpaid_money_keeps_its_penalty`.

### 5. An open intent trusts the first filler (Medium, opt-in fix)
`fill_intent` makes the caller the job's manager. A manager can hire its
own second account and approve unscored work, so a stranger that fills an
open intent can route the budget to itself unless someone answers every
check within 300 s. `post_job` does not have this problem because the
client names its manager.
Fix: an intent may carry `leads`; then only those accounts fill it
(`contracts/factory/src/lib.rs`:70, 521). `sign_intent` in the MCP tools
takes `leads`. An intent without it behaves as before; THREATS.md now says
what that trusts.
Tests: `an_intent_that_names_its_leads_is_filled_only_by_them`,
`a_named_lead_fills_the_intent`, agents `an_intent_can_name_its_leads`.

### 6. Storage drain on the factory (Low)
`cancel_intent` stored a new `(account, nonce)` for free, and `prepay_post`
opened a credit entry for any account id for 1 yocto. Each entry locks
about 0.001 NEAR of the factory's balance for a few Tgas.
Fix: a new cancelled nonce attaches 0.001 NEAR; a prepay is at least
0.001 NEAR (factory lib.rs:39, 325, 579). Cancelling the same nonce again
is free. API change: `cancel_intent` is now payable.
Tests: `cancelling_an_intent_pays_for_its_storage`,
`cancelling_twice_pays_once`, `a_dust_prepay_is_refused`.

### 7. Saturation hides unpaid money (Low)
`weigh` clamped the total and the paid part to `u32::MAX` separately. A
record of 43 M paid and 43 M unpaid weighed as clean.
Fix: a record too large is shifted down as a whole (pick.rs:25). Nothing
changes below 42.9 M fUSD.
Tests: `a_huge_record_keeps_its_unpaid_share`, property
`weighing_never_hides_unpaid_money`.

### 8. Skip list not on the record (Low)
The doc comment and THREATS.md said the skip list is in the event. It was
not: a buyer could skip everyone but a friend and the `award_best` event
showed a clean one-bidder pick.
Fix: `award_best` emits `award_best_asked` with the weights, `min_paid`,
the skip list and the bidders it reads (lib.rs:1197).
Test: `the_skip_list_is_in_the_event`.

### 9. Child id too long strands the deposit (Low)
In the callback `hire_inner` panics if `t<n>.<job>` passes 64 bytes; the
0.035 NEAR attached to `award_best` then stays in the job.
Fix: the callback checks the id and refunds (lib.rs:1301).
Test: `a_child_id_too_long_refunds_instead_of_stranding`.

## Checked and found fine
- `on_award_best` is `#[private]` and calls `assert_self`; its arguments
  (buyer, deposit, by_worker) come only from the job itself.
- Double award: `awarded` is set in one place (`award_inner`); a second
  callback or a hand `award` in between is refused and refunded.
- Bids are frozen between `award_best` and its callback: bids, backing and
  reveals all need the slot open or the reveal open, and the award needs
  both closed.
- Unrevealed sealed bids are never read or scored; their bond goes to the
  buyer only when someone else is awarded; with nothing revealed nobody is
  awarded and bonds return at settle.
- Record reads: a bidder cannot make `get_record` fail or grow (the record
  is fixed-size); a missing record is the default, not an error; any failed
  or garbled read refunds.
- Gas: 16 bids x 5 Tgas + 20 + 150 = 250 Tgas, under the 300 Tgas cap. Not
  measured here (unit tests do not meter gas). If the callback ran out of
  gas, state reverts, the slot stays open and only the 0.035 NEAR stays in
  the job.
- `pick`: saturating math, deterministic, order-independent (property
  tests). Dust bids round the premium down by under one token unit.
- Fire and rehire, contractor carve and restore, backer premium (at most
  half the bond, zero on a failed hire): no new path found.
- `fill_intent`: signature covers the exact JSON with a domain tag and the
  factory id, unknown and duplicate fields are refused, a nonce is spent
  before the launch, the budget leaves the deposit before the launch and
  returns to it on a failed post.
- `record`: only accounts under `.<factory>` report; those have no keys and
  run the pinned code. A job created under another account with this
  factory as registry cannot report.
- Function-call keys: one receiver, one method (`deliver`), no deposit; the
  swarm deletes the key after the delivery. A thief can still spend the
  worker's three deliveries on junk; THREATS.md says so.
- Storm option and MCP `award_best`: off-chain only; the tool reads the
  outcome from `get_bids` after the callback.

## Left open (in THREATS.md, Known limits)
- Records can be washed for the fee and 0.06 NEAR; `min_paid`, `payers` and
  audits raise the price only.
- A bid is consent to the window as it stands; 60 s is a floor, not a fair
  work time.
- A buyer and a bidder together can take an underwriter's NEAR.
- Demo intent keys come from a public seed.

## Effect on the swarm
No change expected in swarm behaviour or results; not re-run. The pick rule
is the same for every input the swarm produces (slots always have a bond,
`min_paid` is not passed, records are far below 42.9 M). Every swarm
worker delivers, so the unasked-hire rule changes no record. Awards happen
long before the last 60 s of the 5400 s window. The contract wasm does
change (new `bid` init argument, one more event, new refusals), so
`out/*.wasm` and `SHA256SUMS` differ after `make build`.
