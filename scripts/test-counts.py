#!/usr/bin/env python3
"""Check the README's test counts against a `cargo test` log.

    python3 scripts/test-counts.py <log> [--fix]

`make test` writes out/test.log and runs this, so a stale count in the
README fails `make test` (--fix rewrites the counts instead).
"""
import re
import sys

NAMES = {  # test binary -> README label
    "ht_core": "core", "money_prop": "property", "pick_prop": "property",
    "hire_tree_factory": "factory", "hire_tree_job": "job",
    "hire_tree_pool": "pool", "mock_outlayer": "mock checker",
    "mock_usdc": "mock-usdc", "hire_tree_agents": "agents", "hiretree": "agents",
}
log = open(sys.argv[1]).read()
counts, cur = {}, None
for line in log.splitlines():
    m = re.search(r"Running .*\(target/[^)]*/deps/([a-z_]+)-[0-9a-f]+\)", line)
    if m:
        cur = NAMES.get(m.group(1), m.group(1))
        continue
    m = re.match(r"test result: ok\. (\d+) passed", line.strip())
    if m and cur:
        counts[cur] = counts.get(cur, 0) + int(m.group(1))
        cur = None
    elif line.strip().startswith("test result:") and "ok." not in line:
        sys.exit("FAIL tests did not pass")
total = sum(counts.values())
want = {
    r"`make test` \((\d+) offline tests\)": total,
    r"\| `make test` \| (\d+) unit \+ property tests": total,
    r"core \((\d+) \+": counts.get("core", 0),
    r"\+ (\d+) property tests": counts.get("property", 0),
    r"job contract \((\d+)\)": counts.get("job", 0),
    r"factory \((\d+)\)": counts.get("factory", 0),
    r"pool \((\d+)\)": counts.get("pool", 0),
    r"mock checker \((\d+)\)": counts.get("mock checker", 0),
    r"mock-usdc \((\d+)\)": counts.get("mock-usdc", 0),
    r"agents \((\d+)\)": counts.get("agents", 0),
}
readme = open("README.md").read()
bad = []
for pat, n in want.items():
    m = re.search(pat, readme)
    if not m:
        bad.append(f"README has no match for {pat}")
    elif int(m.group(1)) != n:
        bad.append(f"README says {m.group(1)}, tests ran {n}: {pat}")
        readme = readme[: m.start(1)] + str(n) + readme[m.end(1):]
if bad and "--fix" in sys.argv:
    open("README.md", "w").write(readme)
    print("fixed README test counts:", *bad, sep="\n  ")
elif bad:
    print("FAIL", *bad, sep="\n  ")
    sys.exit(1)
else:
    print(f"ok   README test counts match the run: {total} tests")
