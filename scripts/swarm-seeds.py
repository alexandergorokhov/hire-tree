#!/usr/bin/env python3
"""Repeat the control comparison across seeds.

    python3 scripts/swarm-seeds.py [--readme] [--committed]

Reads every pair out/seeds/rec-<n>.json + out/seeds/blind-<n>.json, plus
the committed pair web/swarm.json + web/swarm-blind.json (seed 0), and
writes web/swarm-seeds.json: per arm, each measure for every seed. Prints
mean, min and max per arm. With --readme, checks README.md states the
means it prints. With --committed, reads web/swarm-seeds.json instead of
the runs (out/ is not committed), so make lint can check the README.
"""
import glob, json, os, re, sys

BAD = ("junk", "polished")


def measures(d):
    w = [p for r in d["rounds"] for p in r["parts"] if p.get("winner")]
    top_paid = [p for p in w if p["level"] == 1 and p["paid"]]
    spent = sum(l["spent"] for l in d["leads"])
    return {
        "junk wins": sum(p["strategy"] in BAD for p in w),
        "junk paid fUSD": round(sum(p["price"] for p in w if p["strategy"] in BAD and p["paid"]), 2),
        "fired": sum(1 for p in w if p.get("fired")),
        "fUSD per delivered part": round(spent / max(len(top_paid), 1), 2),
    }


if "--committed" in sys.argv:
    out = json.load(open("web/swarm-seeds.json"))
    pairs = out["seeds"]
else:
    out = None
pairs = [] if out else [(0, "web/swarm.json", "web/swarm-blind.json")]
for f in [] if out else sorted(glob.glob("out/seeds/rec-*.json")):
    n = int(re.search(r"rec-(\d+)", f).group(1))
    b = f"out/seeds/blind-{n}.json"
    if os.path.exists(b):
        pairs.append((n, f, b))

out = out or {"seeds": [n for n, _, _ in pairs], "records": [], "blind": []}
for n, rf, bf in pairs:
    r, b = json.load(open(rf)), json.load(open(bf))
    assert not r.get("blind") and b.get("blind"), f"seed {n}: arms swapped"
    assert r.get("seed", 0) == n and b.get("seed", 0) == n, f"seed {n}: file says another seed"
    out["records"].append(measures(r))
    out["blind"].append(measures(b))

summary = {}
print(f"{len(out['seeds'])} seeds: {out['seeds']}")
print(f"{'measure':26}{'records mean (min-max)':>26}{'blind mean (min-max)':>26}")
for k in out["records"][0]:
    row = []
    for arm in ("records", "blind"):
        v = [m[k] for m in out[arm]]
        mean = sum(v) / len(v)
        summary[(arm, k)] = mean
        row.append(f"{mean:.2f} ({min(v):g}-{max(v):g})")
    print(f"{k:26}{row[0]:>26}{row[1]:>26}")
out["mean"] = {arm: {k: round(summary[(arm, k)], 2) for k in out["records"][0]} for arm in ("records", "blind")}
if "--committed" not in sys.argv:
    json.dump(out, open("web/swarm-seeds.json", "w"), indent=1)

if "--readme" in sys.argv:
    readme = re.sub(r"\s+", " ", open("README.md").read())
    bad = 0
    for arm in ("records", "blind"):
        for k in ("junk wins", "junk paid fUSD"):
            s = f"{summary[(arm, k)]:.2f}"
            ok = s in readme
            print(("ok  " if ok else "FAIL"), f"README states the {arm} mean of {k}: {s}")
            bad += not ok
    sys.exit(1 if bad else 0)
