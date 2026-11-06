#!/usr/bin/env bash
# Every test the specs and THREATS.md name must exist as `fn <name>`.
set -euo pipefail
cd "$(dirname "$0")/.."
missing=0
names=$(grep -hoE '(unit|sandbox|property|prop)[^:]*:.*' openspec/specs/*/spec.md docs/THREATS.md | grep -oE '`[a-z][a-z0-9_]+`' | tr -d '`' | sort -u)
names="$names
$(grep -hE '^\|' docs/THREATS.md | awk -F'|' '{print $(NF-1)}' | grep -oE '`[a-z][a-z0-9_]+`' | tr -d '`' | sort -u)"
for n in $(echo "$names" | sort -u); do
  if [ -e "core/tests/$n.rs" ] || grep -rqE "(fn|mod) $n\b" core contracts agents/src agents/tests 2>/dev/null; then continue; fi
  if true; then
    echo "named test not found: $n"; missing=1
  fi
done
[ $missing = 0 ] && echo "spec tests: every named test exists"
exit $missing
