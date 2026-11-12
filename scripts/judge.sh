#!/usr/bin/env bash
# One command for a reviewer (person or AI): every offline check, one
# PASS/FAIL line each, then a single verdict line. No node, no key, no
# network after the crates are fetched. About 2-3 min warm (a cold first
# build of the crates adds several minutes).
#
#   make judge        # or: scripts/judge.sh
#
# Last line on success: "ALL PASS (<n> checks)". Full logs: out/judge/.
cd "$(dirname "$0")/.." || exit 1
mkdir -p out/judge
pass=0
failed=()
row() { printf '%-5s %-22s %6ss  %s\n' "$1" "$2" "$3" "$4"; }
check() {
  local name="$1"; shift
  local log="out/judge/$name.log" t0=$SECONDS
  if "$@" >"$log" 2>&1; then
    pass=$((pass + 1))
    row PASS "$name" $((SECONDS - t0)) "$(grep -v '^\s*$' "$log" | tail -n 1 | cut -c1-90)"
  else
    failed+=("$name")
    row FAIL "$name" $((SECONDS - t0)) "see $log: $(grep -v '^\s*$' "$log" | tail -n 1 | cut -c1-70)"
  fi
}

echo "Hire Tree: offline checks (logs in out/judge/)"
check tests          make --no-print-directory test
check cassettes      make --no-print-directory verify-cassette
check attestation    python3 -I scripts/check-attestation.py demo/nearai-attestation.json
check swarm-run      python3 scripts/swarm-check.py web/swarm.json --readme
check live-run       python3 scripts/swarm-check.py web/swarm-live.json
check seeds          python3 scripts/swarm-seeds.py --check --readme
[ -d runs/storm ] && check storm python3 scripts/swarm-seeds.py --storm --check --readme
[ -d runs/wash ]  && check wash  python3 scripts/swarm-seeds.py --wash --check --readme
[ -s demo/rubric2-cassette.json ] && check rubric-2 make --no-print-directory rubric-check
[ -f web/scene-testnet.json ] && check testnet python3 scripts/testnet-check.py --readme
check economics      python3 scripts/economics.py --check
check spec-tests     scripts/check-spec-tests.sh

echo
if [ ${#failed[@]} -eq 0 ]; then
  echo "ALL PASS ($pass checks)"
else
  echo "FAIL (${#failed[@]} of $((pass + ${#failed[@]})) checks): ${failed[*]}"
  exit 1
fi
