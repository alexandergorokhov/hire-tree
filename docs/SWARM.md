# The swarm, in detail

What `make swarm` runs and what emerged in the committed run. The control runs and their table are in the [README](../README.md#swarm).

## Mechanics

`make swarm` runs a recursive labor market on a local node, 12 rounds:
- **Intents** (our own signed intents, not NEAR Intents). Each round 3 clients sign a job intent off-chain (budget,
  brief, a lead fee that rises from 0 like a Dutch auction) against a fUSD
  deposit at the factory. Three competing leads fill them with
  `fill_intent`: the one that asks the lowest fee fills first and picks
  workers first. The contract checks the signature, spends the deposit
  once and posts the job (AI Swarms brief: intent-based settlement).
- **The lead's model plans the tree.** The lead asks its model which parts
  the brief needs and the most it pays for each (research, translation,
  summary), read safely and capped by the budget.
- **On-chain bidding.** 36 worker agents with 10 strategies bid for each
  part (one lead buys the cheapest after records, one weighs records
  heavily and uses sealed bids, one offers only 75% of the max). Buyers
  price records by money: the share of fUSD a worker was paid or lost,
  and from how many different managers.
  The pick itself runs on-chain: the buyer calls `award_best` with its
  record weight and newcomer premium, the job contract reads each bidder's
  record from the factory and hires the lowest score by the public rule
  (`ht_core::pick`), and the event lists every input. A failed record read
  awards nobody; `award` stays for a buyer that picks by hand.
- **Junk storm.** `make swarm-seeds STORM=6` drops 30 fresh sybil accounts
  into the market at round 6, bidding half price with polished junk, in
  each arm (records, blind, audit-all). Results: README, Junk storm; data
  `runs/storm/` and `web/swarm-storm.json`, charted on the page.
- **Function-call keys for workers.** A worker delivers with a key that can
  call only `deliver` on its one job account: no deposit, no transfers, no
  deletion (`agents/tests/keys.rs`). Bids attach NEAR and stay on the
  owner's key; see `docs/THREATS.md`, Agent keys.
- **Contractors grow the tree.** A winner can put a piece of its part out
  for bids and hire a sub-agent out of its own pay, after asking the model
  "make it myself, or buy it, and for what share?"; trees reach 3 levels, counting the root job.
- **Bonds and underwriters.** Every bid stakes 0.05 NEAR. A newcomer with
  no record asks an underwriter to back it: a trusted agent of the same
  skill puts 0.45 NEAR behind the bid for a premium out of the newcomer's
  bond, if its ask looks like real work. Buyers cut the newcomer premium
  by the bid's total bond, read on-chain. Junk loses both bonds.
- **Fire and replace.** When a winner fails the signed check, the buyer
  fires it on-chain at once (no pay, its bond to the buyer) and hires the
  runner-up in the same round. Passes from workers that are not yet
  trusted (30 fUSD paid by 2 different managers) are audited by two more
  judge panels. The factory keeps a 0.5% fee.

## What emerged

Nothing about the winners is scripted. What emerged in the committed run
(`web/swarm.json`, page `web/swarm.html`):
- 36 signed client intents were filled. Lead fees converged from 4.86 to
  6.28 fUSD in round 1 to about 5.50 by round 12 as leads raced each other.
- 108 parts were bought on-chain, 9 of them by contractors out of their
  own pay; 89 passed the contract's check and were paid early.
- Buyers fired 18 failed winners on the spot; 17 of 18 runner-ups passed,
  so those parts were still delivered in the same round.
- Junk took 21% of wins in the first 4 rounds and 3% in the last 4.  Junk is
  never paid; every unpaid winner (mostly junk) loses its 0.05 NEAR bond
  to the buyer (18 bonds), and each junk bot quits after 2 fresh accounts.
- The judge can be fooled. A "polished" agent hands in fluent text with
  nothing in it; it fooled the first check 2 times. Buyers audit every pass
  from a worker that is not yet trusted (a dispute: 2 more panels, 2 of 3
  decide): 54 audits, and audits caught 2, so the polished agent was paid
  0 times.
- Underwriters backed 3 newcomers once trusted agents existed; all 3
  delivered, so each backer got its NEAR back plus the premium. They
  declined 24 newcomer asks that were too far below the market to be real
  work.
- Make or buy: contractors asked the model 10 times and bought 6.
  Contractors kept 32.80 fUSD of margin between what they won and what
  they paid their sub-agents.
- Lean agents earn the most (373 fUSD), then adaptive (192), undercut
  (125), the model-driven bidder (111) and contractors (91). Greedy agents
  leave.
- The factory kept 12.60 fUSD in platform fees.

In this run the model is the scripted test model. With a NEAR AI key the
same prompts go to NEAR AI (`make record-swarm`).
