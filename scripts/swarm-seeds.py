#!/usr/bin/env python3
"""Repeat the control comparison across seeds.

    python3 scripts/swarm-seeds.py [--readme] [--check]

Reads the committed runs: seed 0 is web/swarm.json (records) and
web/swarm-blind.json (blind); every other run is runs/seeds/<arm>-<n>.json
with arm rec, blind or audit (audit-all: buyers ignore records but audit
every pass). An arm is kept only if it has a run for every seed. Writes
web/swarm-seeds.json: per arm, each measure for every seed, and prints
mean, min and max per arm.

--check   recompute from the raw runs and fail if web/swarm-seeds.json
          differs (does not write it).
--readme  fail unless README.md states every mean it prints.
"""
import glob, json, os, re, sys

BAD = ("junk", "polished")
ARMS = {"rec": "records", "blind": "blind", "audit": "audit-all"}


def measures(d):
    w = [p for r in d["rounds"] for p in r["parts"] if p.get("winner")]
    top_paid = [p for p in w if p["level"] == 1 and p["paid"]]
    good = [p for p in top_paid if p["strategy"] not in BAD]
    spent = sum(l["spent"] for l in d["leads"])
    return {
        "junk wins": sum(p["strategy"] in BAD for p in w),
        "fired": sum(1 for p in w if p.get("fired")),
        "junk paid fUSD": round(sum(p["price"] for p in w if p["strategy"] in BAD and p["paid"]), 2),
        "fUSD spent": round(spent, 2),
        "fUSD per good part": round(spent / max(len(good), 1), 2),
        "audits": sum(1 for p in w if p.get("audit")),
    }


def arm_of(d):
    return d.get("arm") or ("blind" if d.get("blind") else "records")


runs = {}  # arm -> {seed: file}
for f, arm in (("web/swarm.json", "records"), ("web/swarm-blind.json", "blind")):
    runs.setdefault(arm, {})[0] = f
for f in sorted(glob.glob("runs/seeds/*.json")):
    m = re.search(r"(rec|blind|audit)-(\d+)\.json$", f)
    if m:
        runs.setdefault(ARMS[m.group(1)], {})[int(m.group(2))] = f

seeds = sorted(runs["records"])
arms = [a for a in ("records", "blind", "audit-all") if a in runs and sorted(runs[a]) == seeds]
out = {"seeds": seeds, "arms": arms}
for a in arms:
    out[a] = []
    for n in seeds:
        d = json.load(open(runs[a][n]))
        assert arm_of(d) == a, f"{runs[a][n]}: says arm {arm_of(d)}"
        assert d.get("seed", 0) == n, f"{runs[a][n]}: says seed {d.get('seed')}"
        out[a].append(measures(d))

keys = list(out["records"][0])
out["mean"] = {a: {k: round(sum(m[k] for m in out[a]) / len(seeds), 2) for k in keys} for a in arms}
print(f"{len(seeds)} seeds: {seeds}")
print(f"{'measure':22}" + "".join(f"{a + ' mean (min-max)':>30}" for a in arms))
for k in keys:
    row = ""
    for a in arms:
        v = [m[k] for m in out[a]]
        row += f"{out['mean'][a][k]:.2f} ({min(v):g}-{max(v):g})".rjust(30)
    print(f"{k:22}{row}")
claims = []
for a in arms[1:]:
    lower = sum(r["junk wins"] < b["junk wins"] for r, b in zip(out["records"], out[a]))
    claims.append(f"fewer junk wins than {a} buyers on {lower} of {len(seeds)} seeds")
    print("records had", claims[-1])

bad = 0
if "--check" in sys.argv:
    have = json.load(open("web/swarm-seeds.json"))
    ok = have == out
    print(("ok  " if ok else "FAIL"), "web/swarm-seeds.json matches the raw runs")
    bad += not ok
else:
    json.dump(out, open("web/swarm-seeds.json", "w"), indent=1)

if "--readme" in sys.argv:
    readme = re.sub(r"\s+", " ", open("README.md").read())
    for a in arms:
        for k in keys:
            s = f"{out['mean'][a][k]:.2f}"
            ok = s in readme
            print(("ok  " if ok else "FAIL"), f"README states the {a} mean of {k}: {s}")
            bad += not ok
    for s in claims:
        ok = s in readme
        print(("ok  " if ok else "FAIL"), f"README states: {s}")
        bad += not ok
sys.exit(1 if bad else 0)
