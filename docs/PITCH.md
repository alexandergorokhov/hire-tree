# Hire Tree: pitch, speech and video script

## One line
AI agents hire AI agents, and every job is its own NEAR account: it holds
that job's money, proves the AI did the work, pays the worker early if the
work checks out, and deletes itself when the job is done.

## The problem (say it in 3 sentences)
Agents already hire agents, but the money sits with a platform, and an agent
that subcontracts has no safe way to hand part of its budget to a sub-agent.
Workers wait for the whole job to close before they see a cent. And nobody
can prove a deliverable came out of a model call made for this task rather
than being copied or made up.

## What is new (3 NEAR features, combined)
| Piece | NEAR feature | What it gives |
|---|---|---|
| Every job is an account (`t1.job-1.hiretree`) | sub-accounts + one global contract by code hash + `delete_account` | escrow with no owner key; nests to any depth; folds and refunds its storage when done |
| Proof of AI work | NEAR AI Cloud TEE receipt + `env::ecrecover` | the contract checks the model's signature and that the request carries `HT1:<this job>:<spec>:<delivery>`; a copied receipt fails |
| Paid now | Job Note sold to the lead's own treasury for 1% | a verified, checked worker is paid in the same transaction; only the lead carries the risk |
| Quality | one AI check per delivery, started by the contract (OutLayer on testnet) | junk cannot be re-rolled until it passes; 2-of-3 disputes |

## 30-second spoken pitch
"Agents hire agents now, but who holds the money? In Hire Tree, nobody does:
every job is its own NEAR account. The lead agent hires workers, and each
hire creates a sub-account holding exactly that job's pay. When a worker
delivers through NEAR AI, the contract checks the AI's signature on-chain and
that the request was made for this exact task; a copied receipt is refused.
If the work also passes a quality check, the lead's treasury buys the
worker's payout for one percent, so the worker is paid now, not in three
days. When the job ends, the tree folds: every account pays out, proves it
is empty, and deletes itself. Money in equals money out, and you can watch it
happen."

## 3-minute video script
Record the screen of `make web` (the tree page, built from a real `make demo`
run) next to a terminal. One scene per page step. Voice-over in quotes.

| # | Time | Screen | Voice-over |
|---|---|---|---|
| 0 | 0:00-0:15 | Title, then the empty page | "Agents hire agents. Who holds the money, and who proves the work was real? This is Hire Tree, on NEAR." |
| 1 | 0:15-0:35 | Scene 1: job-1 appears with 100 fUSD | "A client posts a job with one token transfer. The factory creates job-1 from a global contract and funds it. No company holds this money: the job account does, and it has no owner key." |
| 2 | 0:35-0:55 | Scene 2: t1, t2, t3 appear | "The lead agent hires three workers. Each hire is a new sub-account holding exactly that job's pay: thirty, twenty, ten. A lead can never give out more than its job holds." |
| 3 | 0:55-1:15 | Scene 3: the refusal text, highlighted | "Here's a cheater. The translator grabs the researcher's real NEAR AI receipt and submits it on its own job. Refused: the request inside the signature names t1, not t2." |
| 4 | 1:15-1:40 | Scene 4: t1 shows the receipt tick and check 8/10 | "The researcher delivers with its own receipt. The contract hashes the request itself, checks the tag binds it to this job and this delivery, and recovers NEAR AI Cloud's signing key with ecrecover. Then the contract, not the worker, starts one quality check: eight out of ten." |
| 5 | 1:40-2:00 | Scene 5: researcher +29.70 | "Now the part workers care about: the lead's treasury buys the researcher's payout for one percent off. The researcher is paid now, in the same transaction, instead of waiting three days." |
| 6 | 2:00-2:15 | Scene 6: t2 no receipt | "The translator used no verified model. It still gets paid, but only at the end, and nobody can buy its note." |
| 7 | 2:15-2:30 | Scene 7: t3 Refunded | "Junk got past one check. The lead disputes; two fresh checks say junk; two of three votes refund the pay up the tree." |
| 8 | 2:30-2:50 | Scenes 8-9: accounts grey out, totals | "And the tree folds. Each account pays out, proves its token balance is zero, deletes itself, and returns its storage to its parent. A hundred in, a hundred out." |
| 9 | 2:50-3:00 | Terminal: `make test` and `make demo` passing; repo URL | "It's all open source and runs on a local NEAR node with one command: make demo. Hire Tree." |

Recording tips: browser at 1280x800, page zoom 110%. Use the page's Play
button (6 s per scene) or the arrow keys. Show the terminal once at the end,
not throughout.

## If a judge asks
| Question | Answer |
|---|---|
| Why NEAR? | Named sub-accounts that a contract can create and delete, global contracts (one code hash for every job), NEAR AI Cloud's TEE-signed receipts, and `ecrecover` on-chain. The design is the account tree; elsewhere it would be a mapping inside one contract. |
| How is this different from market.near.ai? | That market holds funds itself and does not nest. Here each job is its own keyless account, sub-jobs nest, and pay can go out early against an on-chain AI proof. |
| What is real and what is mocked? | Real: contracts, factory, global contract, sub-account create/delete, token transfers, ecrecover, all on a local NEAR node. Mocked in the sandbox: the quality checker (fixed scores) and the NEAR AI signing key (a test key; the text format matches a real NEAR AI Cloud receipt recorded on testnet). On testnet the signer is pinned to NEAR AI Cloud's address. |
| Can the lead self-deal with the note? | Only the lead's own treasury may buy, and only the lead may dispute a sub-job; an upheld dispute only costs the treasury. There is no outside buyer to cheat. |
| What if the AI checker is fooled? | Then the lead, who paid early, loses; not the client. Disputes add two more checks with seeds the worker cannot choose. |
| What if a transaction fails half-way? | Settle sends one transfer at a time; a stuck step can be resumed by anyone and is decided from the real token balance. 10,000 random trees with random failures keep the money rule. |

## Tracks
`swarms` (agents hiring agents in a tree) and `realworld` (escrow and early
payment for agent work). Not `agents`, `business` or `security` (those are
Agent Exit's).
