# Owner session: real NEAR AI and testnet (about 1 hour, mostly waiting)

Everything here runs on the owner's PC (the build box cannot reach NEAR AI
or NEAR RPC). Keys stay in environment variables; nothing writes them to
disk. All money is fake (testnet NEAR, fUSD).

## 0. Setup (once)
```bash
git pull && make setup && make build      # cargo-near runs wasm-opt: job code ~275 KB
export NEARAI_API_KEY=...                 # NEAR AI Cloud key
export HT_MODEL=deepseek-ai/DeepSeek-V4.1-Flash-1017   # or the cheapest model nearai-check lists
```

## 1. Format check (3 small calls, < $0.002)
```bash
make nearai-check
```
| You see | Do |
|---|---|
| last line `GO` | go on |
| `ok (format); the attestation lookup failed, so export HT_SIGNER=<addr>` | run that export; it ends with `GO` too |
| `NO-GO: ... (try another HT_MODEL)` | pick another model from the `models served` line, export `HT_MODEL`, rerun |
| any other `NO-GO` | stop; send the output to the build thread |

## 2. Record the cassette (local node, ~55 small calls, about $0.02)
```bash
make record-cassette        # writes demo/cassette.json, then verifies every signature
git add demo/cassette.json && git commit -m "Real NEAR AI cassette" && git push
```
Anyone can now run `make demo-replay`: the same story with NEAR AI's own
answers and signatures, offline and free. Run it once before committing:
it proves the recording replays. The plan, picks and scores are the
model's own, so they may differ from the scripted `make demo`.

## 3. Testnet (needs ~45 testnet NEAR; about 25 minutes)
The global job code burns ~27.5 NEAR (10x storage price), the factory and
token ~5, seven demo accounts 1 NEAR each. The dry run on a local node
measured 43.6 NEAR in total (`make testnet-dryrun`).
```bash
export HT_OPERATOR=hiretree.testnet
export HT_OPERATOR_KEY=ed25519:...        # the operator's full-access key
make testnet-all                          # writes web/scene-testnet.json with explorer links
```
- Interrupted? Run it again: deployed contracts and accounts are reused
  (keys are derived from the operator key); the story restarts from a new job.
- The story waits out real 5-minute check windows and one 20-minute job
  window (the junk bot's reopened parts).
- Then: `git add web/scene-testnet.json && git commit -m "Testnet run" && git push`.
  Open the page with `?scene=scene-testnet.json`.

## Owner-only (not automated)
Submitting, the X post (tag @NEARProtocol #NEARHackathon), making the repo
public, the video voice-over (script in `docs/PITCH.md`).
