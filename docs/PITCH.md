# Hire Tree: pitch, speech and video script

## One line
When an AI agent hires another AI agent, who holds the money? In Hire Tree
nobody does: every job is its own NEAR account that picks the winning bid
by a public rule, pays the work it can check, and then deletes itself. It
is the settlement layer agent markets plug in (docs/INTEGRATE.md): escrow
with no operator key, safe subcontracting, a contract-defined "done", and a
portable on-chain record.

Picture: `docs/screens/recursive-market.svg` (one real tree from `make swarm`).

## The problem (say it in 3 sentences)
On agent marketplaces the escrow sits in one platform contract, and an agent
that subcontracts has no safe way to hand part of its budget to a sub-agent.
Workers wait for the whole job to close before they see a cent. And nobody
can prove a deliverable came out of a model call made for this task rather
than being copied or made up.

## What is new (3 NEAR features, combined)
| Piece | NEAR feature | What it gives |
|---|---|---|
| Every job is an account (`t1.job-1.hiretree`) | sub-accounts + one global contract by code hash + `delete_account` | escrow with no owner key; sub-jobs nest (up to 3 levels in `make swarm`; any worker can hire below itself out of its own pay); folds and refunds its storage when done |
| Proof of AI work | NEAR AI Cloud signed receipt (key published in its attestation report) + `env::ecrecover` | the contract checks the model's signature and that the request carries `HT1:<this job>:<spec>:<delivery>`; a copied receipt fails |
| Paid now | Job Note sold to the lead's own treasury for 1% | a verified, checked worker is paid in the same transaction; only the lead carries the risk |
| Quality | the contract writes the judge question; anyone submits NEAR AI's signed answer; `ecrecover` checks it | a score signed by the pinned model on the contract's own question; a forged or off-topic answer is refused; junk reopens the job; 2-of-3 disputes |
| Bidding | `open_bids` / `bid` / `award_best` on the job account | agents compete on price on-chain; `award_best` makes the job read every bidder's record from the factory and hire by the public rule (`ht_core::pick`), so the pick is on-chain and anyone can re-check it; the award can never pay more than the winner bid; optional sealed bids (commit, then reveal) and a NEAR bond that a junk winner loses to its buyer; `fire` ends a failed job at once so the buyer hires the runner-up; a newcomer can stake a bigger bond to vouch for itself |
| Business model | the factory keeps a platform fee (0.5% in the swarm, capped at 1% by the contract) of each posted budget, only when the post succeeds | the protocol earns on every job without holding anyone's money |
| Buy together | a pool contract (deploy key deleted) is the job's client | three clients fund one job; stake majority approves; each gets its share of the unspent budget back |
| Agent keys | function-call access keys | a worker delivers with a key that can call only `deliver` on its one job account: a stolen worker key cannot move money |
| Track records | each sub-job reports its worker's outcome to the factory when it deletes | the lead agent reads `get_record` before it hires, so a junk bot loses work |

## The result (from `runs/`, checked by `make lint`)
**Under attack.** At round 6, 30 fresh sybil accounts flood every auction
with half-price junk. Mean per run over 4 seeds:

| | With records | Audit everything | Blind |
|---|---|---|---|
| Parts won by the storm | 10.75 | 71.50 | 71.50 |
| fUSD per good part | 13.66 | 24.06 | 26.03 |
| fUSD held by storm keys | 0.00 | 0.00 | 0.00 |

Records hold the market; audits alone do not, because they catch junk only
after it has won, so the part is bought twice. With records the market is
also the cheaper one: about half the cost per good part.

**Washed storm.** The sybils first pay themselves through real jobs until
their records look trusted (about 10 fUSD of fees per run). Plain records
then fail: 72.00 storm wins. Buyers that pass `award_best` a payer list
(the leads and the contractors they trust) count only that pay, and the
storm wins 19.50, lower on every seed (4 seeds, `runs/wash/`).

**Calm market.** Same market, no storm: records gave junk fewer wins than
both other buyers on every seed, with half the audits of audit-all, for
about 3% more per good part on average (8% cheaper to 11% dearer per seed).

## 30-second spoken pitch
"Agents hire agents now, but who holds the money? In Hire Tree, nobody does:
every job is its own NEAR account. A lead agent plans the job and puts each
part out for bids; worker agents bid on-chain, and each award creates a
sub-account holding exactly the winning bid. Workers
deliver through NEAR AI, and the contract checks the AI's signature
on-chain. Then the contract asks its own quality question; NEAR AI's signed
answer is the score. Junk reopens the job, and the junk bot's record
follows it: in the next job the job contract reads that record on-chain
and hires someone else. Then we attacked our own market: 30 sybil accounts
bidding half price with polished junk. From then on, buyers that audit
everything gave three of every four wins to the attackers. Buyers that read
on-chain records gave them about one in six, and paid half as much per good
part. Good work is paid now, through the lead's treasury, not at
the end. When the job ends, the tree folds and every account deletes
itself."

## 3-minute video script
Record the screen of `make web` (the tree page, built from `make demo`, or
from `make demo-replay` once a NEAR AI cassette is recorded) next to a
terminal. Voice-over in quotes. The page lists each agent decision and its
reason under the scene title: point at them.

| # | Time | Screen | Voice-over |
|---|---|---|---|
| 0 | 0:00-0:12 | Title | "Agents hire agents. Who holds the money, and who decides the work was good? This is Hire Tree, on NEAR." |
| 1 | 0:12-0:30 | Scene 1: job-1 appears | "A client posts a job with one token transfer. The factory creates a job account from a global contract. The account has no owner key: no company holds this money." |
| 2 | 0:30-0:55 | Scenes 2-3: bids listed, then awards; t1, t2, t3 appear | "The lead agent's model splits the task and puts each part out for bids on the job account. Worker agents bid their own price; the cheap bot bids lowest. Nobody has a record yet, so the bot wins two parts. Each award is a new sub-account holding exactly the winning bid." |
| 3 | 0:55-1:25 | Scene 4: deliveries and judge answers | "The researcher works through NEAR AI and delivers the raw response with NEAR AI's signature; the contract checks it was made for this job. The cheap bot sends filler. Then the contract asks its own question: the rubric, the task, the deliverable. A keeper sends exactly that to NEAR AI and submits the signed answer." |
| 4 | 1:25-1:40 | Scene 5: 8/10, 2/10, 2/10 | "The contract verified each signature and read the score: eight for the research, two for the filler. Junk reopens." |
| 5 | 1:40-1:55 | Scene 6: researcher +29.70 | "The lead's treasury buys the researcher's payout for one percent: paid now, not at the end." |
| 6 | 1:55-2:10 | Scene 7: accounts grey out; records line | "The tree folds. Each account pays out, proves it is empty and deletes itself, and reports its worker's outcome to the factory. The bot: two jobs, zero paid." |
| 7 | 2:10-2:35 | Job 2 awards: "avoided cheapbot on its on-chain record, though it bid less" | "Second job, same task. The bot bids low again, but this time the lead reads its record and awards the others. It reads the record on-chain and weighs it against the price." |
| 8 | 2:35-2:50 | Scenes 13-14: notes bought, tree folds | "Everyone checks out, everyone is paid now, and the tree folds again. The client paid the fee and the work that passed, nothing else." |
| 8b | (optional, +15 s) | Job 3: "three clients pool their money", "the pool pays its members back" | "Three clients who would not pay alone pool their money. The pool contract is the client: two of three vote to approve, and each gets its share of what was not spent." |
| 9 | 2:50-3:00 | Footer: transactions list; terminal `make demo` | "Every number on this page was read from the chain after a real transaction. Open source: make demo. Hire Tree." |

The numbers above are from the scripted run. With NEAR AI the plan, the
picks and the scores are the model's own: read the voice-over from the
page's decision lines, not from this table. Record from the cassette
(`make demo-replay`) or testnet if they exist.

Recording tips: browser at 1280x800, page zoom 110%. Arrow keys between
scenes. Say "test key" honestly if recording from `make demo`; from a
cassette or testnet the answers are NEAR AI's own.

## If a judge asks
| Question | Answer |
|---|---|
| Why NEAR? | Named sub-accounts that a contract can create and delete, global contracts (one code hash for every job), NEAR AI Cloud's signed receipts, and `ecrecover` on-chain. The design is the account tree; elsewhere it would be a mapping inside one contract. |
| How is this different from an agent marketplace? | A marketplace escrow is one contract holding everyone's funds, and a hired agent cannot safely re-hire with part of its budget. Here each job is its own keyless account, sub-jobs nest, and pay can go out early against an on-chain AI proof. |
| What is real and what is mocked? | Real: contracts, factory, global contract, sub-account create/delete, token transfers, ecrecover, the agents and their tool calls. In `make demo` the model is scripted and signs with a test key (the page says so). A cassette replays real NEAR AI answers with their original signatures, and the contract verifies them again. The factory pins the signer and the judge model. |
| Are the agents real? | With NEAR AI (cassette replay or testnet) every decision (plan, pick, review, deliver, judge) is a model call. In `make demo` a scripted stand-in answers the same prompts (simple rules, signed with a test key), and the page says so. Every action is a tool call from one list that `hiretree mcp` also serves to an MCP agent, limited to this factory's jobs. |
| Can the lead self-deal with the note? | Only the lead's own treasury may buy, and only the lead may dispute a sub-job. The worker already has its money; an upheld dispute moves the face value from the treasury back up the tree, so the lead at best breaks even. There is no outside buyer to cheat. |
| What if nobody answers the check? | The round closes after 5 minutes unscored: not paid early, the note is never sellable, and the lead decides at settle. Settle waits while a round is open. |
| Can a worker pick a friendly answer? | The model and signer are pinned, the question is built by the contract at temperature 0, the deliverable is wrapped as data, and the lowest answer counts; answers 3 or more apart make the check unclear. Disputes add two more rounds. If the judge is fooled, the lead who paid early loses, not the client. |
| Who earns, and how does it scale? | The factory keeps a capped fee on every posted budget. Contractors earn the margin between what they win and what they pay sub-agents. In `make swarm` three competing leads buy from 36 agents and nobody scripts the winners. |
| Why not a judge panel of several models, or NEAR Intents payouts? | Designed in `docs/NEXT.md`. A panel needs one attested key per model and a real receipt to test against; Intents is mainnet only. Today: the lowest of several signed answers counts, and disputes add two more panels. |
| What if a transaction fails half-way? | Settle sends one transfer at a time; every stuck step (hire, check, note sale, settle, dispute) has a resume path decided from the real token balance. Tokens above the books are never booked, only swept up the tree, so a refunded transfer cannot lock a job. 10,000 random trees with random failures keep the money rule. |

## Tracks
`swarms` (main: a coordination market, on-chain bidding, collective
purchasing, signed job intents), `agents` (agents earn, hire out of their
own pay, keep the margin) and `security` (portable records priced by money,
signed checks and audits, function-call keys). Same list as the README's
Hackathon section.
