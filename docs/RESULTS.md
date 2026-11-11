# Results: control runs, junk storm, washed storm

Every number here is recomputed from the committed raw runs by `make lint`
(`scripts/swarm-seeds.py`, `scripts/swarm-compare.py`). The market and its
agents are described in [SWARM.md](SWARM.md).

## Control runs (calm market)

Do the on-chain records matter, or would audits alone do?
`make swarm-seeds ARMS="rec blind audit"` runs the same market three ways
on 5 seeds (seed 0 is the committed run in `web/swarm.json` and
`web/swarm-blind.json`; every other run is in `runs/seeds/`, and `make lint`
recomputes this table from them):
- **With records:** buyers price records and audit passes from workers
  that are not yet trusted.
- **Blind:** buyers ignore records and never audit.
- **Audit-all:** buyers ignore records but audit every pass.

Mean per run of 12 rounds. A *good part* is a part a lead bought, paid,
from a worker that is not junk; *fUSD per good part* is all the leads
spent divided by good parts. *Sub-parts* are the parts the lead's winning
contractors bought one level down (deeper levels are not counted).

| | With records | Blind | Audit-all |
|---|---|---|---|
| Junk wins | 19.00 | 25.60 | 24.20 |
| Winners fired | 17.80 | 22.80 | 22.40 |
| fUSD paid to junk | 8.68 | 28.34 | 16.34 |
| Audits run | 56.60 | 0.00 | 115.20 |
| fUSD spent by leads | 1037.64 | 1004.20 | 1000.52 |
| fUSD per good part delivered | 12.68 | 12.52 | 12.30 |
| Sub-parts bought by contractors | 13.20 | 26.40 | 26.40 |

- **Records beat audits alone.** Buyers with records had fewer junk wins
  than blind buyers on 5 of 5 seeds, and fewer junk wins than audit-all
  buyers on 5 of 5 seeds. They fired about a fifth fewer winners and paid
  junk about half as much (lower on 3 of 5 seeds, equal on 1) while running half the
  audits, each of which costs two more model calls.
- **What records cost.** A good part (paid, not from junk) costs 12.68 fUSD
  with records against 12.30 with audit-all and 12.52 blind: about 3% more
  than audit-all and 1.3% more than blind on average, because buyers skip cheap asks from accounts with bad
  records. Per seed it swings from 8% cheaper to 11% dearer.
- **Records make trees smaller.** Contractors buy about half as many
  sub-parts with records. Junk no longer sets the going rate, so the
  contractor's model reckons a bought sub-part at 83-87% of its max,
  counting junk risk, above its own cost (80%), and does it itself. Blind
  contractors see junk's low winning prices and keep buying.
- **Junk pay is noisy.** It ranges from 0 to 11 fUSD with records, 0 to
  37.8 with audit-all and 5.5 to 71.2 blind. All of it, in every arm,
  went to polished junk that fooled the check and was not caught by an
  audit.
- **Why blind junk vanishes late.** Blind junk burns through its fresh
  accounts sooner, so it holds 0% of wins in the last 4 rounds on every
  seed.

## Junk storm

`make swarm-seeds STORM=6 ARMS="rec blind audit"`: at
round 6 of 12, 30 fresh accounts with no record join and bid on every part
at about half the usual price, with fluent, empty answers that sometimes
fool the judge. Mean per run over 4 seeds (every run is in `runs/storm/`;
`make lint` recomputes these from them):

| | With records | Blind | Audit-all |
|---|---|---|---|
| Parts won by storm accounts | 10.75 | 71.50 | 71.50 |
| Junk wins, all junk | 27.75 | 96.75 | 95.25 |
| fUSD paid to junk | 26.95 | 210.88 | 133.03 |
| fUSD per good part delivered | 13.66 | 26.03 | 24.06 |
| fUSD earned by storm accounts | 21.50 | 176.75 | 115.25 |
| fUSD held by storm keys (max) | 0.00 | 0.00 | 0.00 |
| Sub-parts bought by contractors | 14.00 | 25.25 | 24.25 |

- **Records hold; audits alone do not.** In the storm round, junk takes
  12% of wins from buyers with records, against 96% blind and 100% for
  buyers that audit everything. Records come out ahead on every seed on
  storm wins, junk wins, junk pay and cost per good part.
- **Under attack, records are the cheaper market.** A good part costs
  about half as much with records, and leads spend less in total (1060.67
  fUSD per run, against 1134.45 blind and 1166.20 audit-all). Audits catch
  the junk after it wins, so the part is bought twice.
- **The attackers still earn something.** Polished junk that fools the
  check is paid: 21.50 fUSD per run with records, against 176.75 blind.
  What never happens is money held outside escrow: storm accounts are paid
  only by job accounts that settled, and what they hold beyond settled pay,
  read from chain balances, stays 0.00 in every round of every run.

## Washed storm

Records can be bought: a sybil manager posts real jobs
to its own sybils and pays them, so each crosses the trusted line (30 fUSD
from 2 payers) before the storm. `make swarm-seeds STORM=6 WASH=1
ARMS="rec allow"` runs that attack. Arm `allow` is the same buyers passing
`award_best` a `payers` list: the three leads plus every account that the leads' own pay has made trusted (read on-chain; the buyer does not know who is a contractor or a sybil). The factory keeps who paid each worker how much
(`get_record_from`), so pay from anyone else counts for nothing. Mean per
run over 4 seeds (`runs/wash/`; `make lint` recomputes these):

| | Records | Records + payer list |
|---|---|---|
| Parts won by storm accounts | 72.00 | 19.50 |
| fUSD earned by storm accounts | 176.75 | 20.88 |
| fUSD per good part delivered | 25.36 | 13.58 |
| Wash cost: fUSD out of the storm's pocket | 10.29 | 6.88 |
| Wash cost: NEAR for storage and gas | 6.84 | 4.91 |

- **Washing beats plain records.** For about 10 fUSD of platform fees and
  about 7 NEAR, washed sybils win as much as against buyers with
  no records at all (71.50 in the plain storm).
- **The payer list undoes it.** Buyers had fewer storm wins with the payer list on 4 of 4 seeds, and the cost per good part
  is back near the unwashed storm (13.66). The list is the buyer's own
  choice, passed per award; the contract adds no allow-list of its own.
- **What the list costs.** Before the storm, in rounds 4 and 5, junk
  takes about 15% of wins with the payer list against about 4% without:
  some honest pay no longer counts, so more good workers look like
  newcomers and junk competes with them on price. The storm accounts still
  take a fifth of wins after round 6 (19.50 parts), not zero.
- **No inside knowledge.** The buyers build the list from on-chain records
  only. An earlier version that used the agents' hidden types picked the
  same winners on all 4 seeds.
- **Caveat.** Under CPU load, 2 audits in the payer-list runs came too
  late (the job window had closed), so those passes stood unaudited; the
  run counts them as `late_audits`. That favours the storm.

## Seed 0

The run the page shows: blind buyers gave 24 junk wins
(18 with records), 23 fired (18), and 5.50 fUSD paid to junk vs
0.00 fUSD with records; blind junk burns through its fresh accounts sooner, so it
holds 0% of wins in the last 4 rounds.

The model in these runs is the scripted test model. With a NEAR AI key the
same prompts go to NEAR AI (`make record-swarm`).

