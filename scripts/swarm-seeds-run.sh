#!/usr/bin/env bash
# Run the swarm for each seed in $1 (e.g. "1 2 3") and each arm in $3
# (default "rec blind"; also "audit"), $2 runs at a time. Called by make
# swarm-seeds from agents/ with the node environment set. Writes
# runs/seeds/<arm>-<seed>.json and out/seeds/<arm>-<seed>.log.
set -euo pipefail
seeds="$1"; runs="$2"; arms="${3:-rec blind}"
out="$(cd .. && pwd)/runs/seeds"; logs="$(cd .. && pwd)/out/seeds"
mkdir -p "$out" "$logs"
cargo build --quiet --locked --bin hiretree
one() {
  local s=$1 arm=$2 flag=""
  case "$arm" in blind) flag=--blind ;; audit) flag=--audit-all ;; esac
  ./target/debug/hiretree swarm --seed "$s" $flag --rounds 12 --out "$out/$arm-$s.json" > "$logs/$arm-$s.log" 2>&1 \
    && echo "seed $s $arm: done" || echo "seed $s $arm: FAILED (see $logs/$arm-$s.log)"
}
export -f one; export out logs
for s in $seeds; do for a in $arms; do printf '%s %s\n' "$s" "$a"; done; done | xargs -P "$runs" -L 1 bash -c 'one "$0" "$1"'
