# agents

## Purpose
The agents that use the contracts: a lead that plans, picks, hires,
reviews and pays early; workers that deliver NEAR AI output with its signed
receipt; a keeper that answers the contract's quality question with a
signed NEAR AI answer. Each decision is a model call; each action is a tool
call from one shared list, which the MCP server also exposes.

## Requirements

### Requirement: The lead picks workers by their on-chain record
Before each hire the lead SHALL read every candidate's record with
`get_record` and give it to the picker model with the candidates' skills.
A worker with jobs it was not paid for SHALL lose to a matching worker
without such jobs.

#### Scenario: Junk bot avoided in the second job
- GIVEN job 1 where the cheap bot was hired for two parts and paid for none
- WHEN job 2 is planned
- THEN the picker hires the translator and the researcher, and says it avoided the bot on its record
- sandbox: `agents_story_with_mock_model`; unit: `picker_takes_the_lowest_bid_unless_its_record_is_bad`

### Requirement: Workers deliver with the job's tag and NEAR AI's signature
A worker that uses a model SHALL send `HT1:<job>:<spec hash>:1` in its
request and deliver the exact request bytes, response bytes and signature.
- sandbox: `agents_story_with_mock_model` (the contract marks the delivery verified)

### Requirement: The keeper asks exactly the contract's question
The keeper SHALL take the request bytes from `check_request`, send them
unchanged to the judge model and submit the answer and signature with
`submit_check`; after 5 minutes anyone finalizes.
- sandbox: `agents_story_with_mock_model` (8/10 for real work, 2/10 for filler); unit: `mock_judge_scores_filler_low`

### Requirement: Reviewer pays early only for quoted, good work
The reviewer SHALL buy a note only when `quote_note` is not null; otherwise
it waits for settle or disputes.
- unit: `reviewer_buys_only_quoted_good_work`

### Requirement: The run conserves money and folds the tree
After all three jobs: every job account is deleted, the lead holds three fees, the
client paid only fees and delivered work, and no fUSD was created or lost.
- sandbox: `agents_story_with_mock_model`, `agents_story_replays_from_a_cassette`

### Requirement: Cassettes replay real signatures and refuse anything else
A cassette SHALL store each request, response and signature as sent and
received. Replay SHALL answer only a byte-identical request; `verify` SHALL
fail if any answer was edited.

#### Scenario: Recorded run replays on a fresh node
- THEN the second run makes the same calls and the contracts accept every signature again
- sandbox: `agents_story_replays_from_a_cassette`

#### Scenario: Edited answer or unseen request
- THEN `verify` fails / replay errors with "diverged"
- unit: `cassette_verifies_and_catches_tampering`, `replay_refuses_a_request_it_never_saw`

### Requirement: NEAR AI client refuses a format mismatch
The live client SHALL fetch the signature for each chat and stop with
NO-GO if NEAR AI's signed text is not `model:sha256(our request
bytes):sha256(the response bytes we got)`, or if the signature does not
recover to the address NEAR AI reports.
- live only: `make nearai-check` (needs the owner's key)

### Requirement: MCP server exposes the same tools
`hiretree mcp --as NAME` SHALL answer `initialize`, `tools/list` and
`tools/call` over stdio JSON-RPC with the same tool list the agents use,
signing as `NAME.<operator>` on testnet.
- manual: `echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | hiretree mcp --as lead`

### Requirement: Clients buy together through a pool
In job 3 of the story three client agents join one pool (30 fUSD each of
90), the client launches it and binds the job, the lead runs the job as
usual, two members vote to approve, and after the job deletes itself each
member claims its share of the unspent budget (7 fUSD).
- sandbox: `agents_story_with_mock_model` (job 3); see `openspec/specs/pool/spec.md`

### Requirement: Any MCP client can use Hire Tree on a local node
`hiretree mcp --local` (started by `.mcp.json` through
`scripts/mcp-local.sh`) SHALL answer the MCP handshake at once, boot a local
node with funded demo accounts on the first tool call, let every tool act as
any demo account (`as`), and add `guide`, `accounts`, `wait`, `do_work`
(a signed delivery from the test model), `judge_check` (the contract's own
question, signed answer) and `run_story`. Only JSON-RPC reaches stdout.
- sandbox: `a_judge_walks_the_guide_over_mcp` (post, bid refused above max, award, signed delivery, 8/10, paid 24.75 early, fold)
