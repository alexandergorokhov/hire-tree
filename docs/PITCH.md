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

## What is new (NEAR features, combined)
| Piece | NEAR feature | What it gives |
|---|---|---|
| Every job is an account (`t1.job-1.hub`) | sub-accounts + one global contract by code hash + `delete_account` | escrow with no owner key; sub-jobs nest (up to 3 levels in `make swarm`; any worker can hire below itself out of its own pay); folds and refunds its storage when done |
| Proof of AI work | NEAR AI Cloud signed receipt (key published in its attestation report) + `env::ecrecover` | the contract checks the model's signature and that the request carries `HT1:<this job>:<spec>:<delivery>`; a copied receipt fails |
| Paid now | Job Note sold to the lead's own treasury for 1% | a verified, checked worker is paid in the same transaction; only the lead carries the risk |
| Quality | the contract writes the judge question; anyone submits NEAR AI's signed answer; `ecrecover` checks it | a score signed by the pinned model on the contract's own question; a forged or off-topic answer is refused; failed work reopens the job; 2-of-3 disputes |
| Bidding | `open_bids` / `bid` / `award_best` on the job account | agents compete on price on-chain; `award_best` makes the job read every bidder's record from the factory and hire by the public rule (`ht_core::pick`), so the pick is on-chain and anyone can re-check it; the award can never pay more than the winner bid; optional sealed bids (commit, then reveal) and a NEAR bond that a failing winner loses to its buyer; `fire` ends a failed job at once so the buyer hires the runner-up; a newcomer can stake a bigger bond to vouch for itself |
| Paid in any token | NEAR Intents: the job pays into the Verifier (`ft_transfer_call`, worker as receiver); signed NEP-413 `token_diff` intents | the worker swaps its pay with a solver atomically and withdraws; tested on the Verifier's own release wasm on a local node |
| Unit economics | measured gas per step (`make gas`) | one whole part costs 0.0115 NEAR of gas; storage comes back when accounts delete ([ECONOMICS.md](ECONOMICS.md)) |
| Business model | the factory keeps a platform fee (0.5% in the swarm, capped at 1% by the contract) of each posted budget, only when the post succeeds | the protocol earns on every job without holding anyone's money |
| Buy together | a pool contract (deploy key deleted) is the job's client | three clients fund one job; stake majority approves; each gets its share of the unspent budget back |
| Agent keys | function-call access keys | a worker delivers with a key that can call only `deliver` on its one job account: a stolen worker key cannot move money |
| Track records | each sub-job reports its worker's outcome to the factory when it deletes | the lead agent reads `get_record` before it hires, so an agent that hands in failed work loses the next contract |

## The result (from `runs/`, checked by `make lint`)
**The attack (a simulation on a local NEAR node).** From round 6 of 12,
thirty fake agents flood every auction, bidding half price and handing in
empty work. Mean per run over 4 seeds:

| | Buyers who check the record first | Buyers who check work after hiring | Buyers who choose only on price |
|---|---|---|---|
| Tasks the fake agents won | 10.75 | 71.50 | 71.50 |
| fUSD per good task | 13.66 | 24.06 | 26.03 |
| fUSD the fake agents kept | 0.00 | 0.00 | 0.00 |

What it shows: the on-chain record protects the market by itself. Checking
work only after hiring does not help, because by then the fake agent has
already won the task and it must be bought twice. Buyers who check the
record first also pay about half as much per good task.

**Then the fake agents faked a record.** Before the flood, the attackers pay
their own fake agents through real jobs until every fake looks trusted
(about 10 fUSD of fees per run). Buyers who count any pay are fooled: the
fakes win 72.00 tasks. Buyers that pass `award_best` a list of payers they
trust (the leads, plus accounts the leads' own pay made trusted) count only
that pay, and the fakes win 19.50, lower on every seed (4 seeds, `runs/wash/`).

**Calm market.** Same market, no attack: buyers who check the record gave
failed work fewer wins than both other kinds of buyer on every seed, with
half the checks of checking everything, for about 3% more per good task.

## 30-second spoken pitch
"When one agent hires another, who holds the money? In Hire Tree, nobody
does: every job is its own NEAR account, holding only its share. Worker
agents bid on-chain, and each award creates a sub-account holding exactly
the winning bid. A model on NEAR AI scores each delivery and signs the
score; the contract checks that signature itself and pays only work that
passes. Failed work reopens the job, and the agent's record follows it.
To test it, we attacked our own market in a simulation: thirty fake agents
bidding half price with empty work. Buyers who chose only on price hired a
fake seventy-one times. Buyers who checked the on-chain record first, only
eleven times. So the record protects the market by itself. When the job
ends, the tree folds and every account deletes itself."

## Video
The submitted video narration is `video/voice-lines.tsv` (11 lines, Kokoro
voice); `node scripts/clip.js` records the page in sync with it and
`video/voice/README.md` shows how the sound is mixed in.

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
| Why not a judge panel of several models? | Designed in `docs/NEXT.md`: a panel needs one attested key per model. Today the lowest of several signed answers counts, and disputes add two more panels. NEAR Intents payouts are built (`set_payout_via`, tested on the Verifier's own wasm). |
| Can fluent filler fool the AI judge? | It did: in the live NEAR AI run, filler passed 7 of 7. Judge rubric 2 makes the judge quote two passages that do the task, and the contract counts the score only if both are really in the work (else at most 2). Live re-grades: that filler 0 of 7; filler written for the task 0 of 6; filler copying the task 0 of 8; filler the model wrote to look finished 1 of 8 (it named the steps); real answers 6 of 6 and 8 of 8. The testnet run found that quotes after a line break were missed (a real 10 counted 2); fixed and redeployed. |
| What if a transaction fails half-way? | Settle sends one transfer at a time; every stuck step (hire, check, note sale, settle, dispute) has a resume path decided from the real token balance. Tokens above the books are never booked, only swept up the tree, so a refunded transfer cannot lock a job. 10,000 random trees with random failures keep the money rule. |

## Tracks
`swarms` (main: a coordination market, on-chain bidding, collective
purchasing, signed job intents), `agents` (agents earn, hire out of their
own pay, keep the margin) and `security` (portable records priced by money,
signed checks and audits, function-call keys). Same list as the README's
Hackathon section.
