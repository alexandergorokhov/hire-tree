#!/usr/bin/env bash
# Run the swarm for each seed in $1 (e.g. "1 2 3"), with records and blind,
# $2 runs at a time. Called by make swarm-seeds from agents/ with the node
# environment set. Writes out/seeds/{rec,blind}-<seed>.json and .log.
set -euo pipefail
seeds="$1"; runs="$2"; out="$(cd .. && pwd)/out/seeds"
mkdir -p "$out"
cargo build --quiet --locked --bin hiretree
one() {
  local s=$1 arm=$2 flag=""
  [ "$arm" = blind ] && flag=--blind
  ./target/debug/hiretree swarm --seed "$s" $flag --rounds 12 --out "$out/$arm-$s.json" > "$out/$arm-$s.log" 2>&1 \
    && echo "seed $s $arm: done" || echo "seed $s $arm: FAILED (see $out/$arm-$s.log)"
}
export -f one; export out
for s in $seeds; do printf '%s rec\n%s blind\n' "$s" "$s"; done | xargs -P "$runs" -L 1 bash -c 'one "$0" "$1"'
