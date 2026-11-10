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
--storm   the junk-storm runs instead: runs/storm/<arm>-<seed>.json (made by
          `make swarm-seeds STORM=6`) into web/swarm-storm.json: per arm and
          seed the totals, and per arm the mean per-round series (junk share
          of wins, fUSD paid to junk, storm earnings, fUSD held by storm keys).
          Nothing is written or checked when there are no storm runs.
          With --readme, fail unless README.md states the storm means in
          STORM_README.
--wash    the washed-storm runs: runs/wash/<arm>-<seed>.json (made by
          `make swarm-seeds STORM=6 WASH=1 ARMS="rec allow blind audit"`)
          into web/swarm-wash.json, the same summary as --storm for the
          arms rec (records), allow (records with the buyers' payer list),
          blind and audit, plus what washing cost the storm (platform fee
          fUSD, fUSD outlay, NEAR, sybils washed). Nothing is written or
          checked when there are no wash runs. With --readme, fail unless
          README.md states the wash means in WASH_README and how many seeds
          the payer list cut storm wins on.
"""
import glob, json, os, re, sys

BAD = ("junk", "polished")
ARMS = {"rec": "records", "blind": "blind", "audit": "audit-all", "allow": "records+allowlist"}
# Storm means the README must state (with --storm --readme).
STORM_README = ("junk wins", "storm wins", "storm earned fUSD", "junk paid fUSD", "fUSD per good part", "max fUSD held by storm keys", "sub-parts bought")
WASH_README = ("storm wins", "storm earned fUSD", "fUSD per good part", "wash fUSD outlay", "wash NEAR spent")


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
        "sub-parts bought": sum(1 for p in w if p["level"] == 2),
    }


def arm_of(d):
    a = d.get("arm") or ("blind" if d.get("blind") else "records")
    return a + "+allowlist" if d.get("allowlist") else a


def storm_summary(kind="storm"):
    runs = {}
    for f in sorted(glob.glob(f"runs/{kind}/*.json")):
        m = re.search(r"(rec|blind|audit|allow)-(\d+)\.json$", f)
        if m:
            runs.setdefault(ARMS[m.group(1)], {})[int(m.group(2))] = f
    if not runs:
        print(f"no {kind} runs (runs/{kind}/*.json): nothing to do")
        return 0
    seeds = sorted(set.intersection(*[set(v) for v in runs.values()]))
    order = ("records", "records+allowlist", "blind", "audit-all") if kind == "wash" else ("records", "blind", "audit-all")
    arms = [a for a in order if a in runs]
    out = {"seeds": seeds, "arms": arms}
    for a in arms:
        out[a], out.setdefault("series", {})[a] = [], []
        ds = []
        for n in seeds:
            d = json.load(open(runs[a][n]))
            assert arm_of(d) == a and d.get("seed", 0) == n, f"{runs[a][n]}: wrong arm or seed"
            assert d.get("storm"), f"{runs[a][n]}: not a storm run"
            assert (kind != "wash") or d.get("wash"), f"{runs[a][n]}: not a wash run"
            out.setdefault("storm_round", d["storm"]["round"])
            assert out["storm_round"] == d["storm"]["round"], "storm round differs between runs"
            ds.append(d)
            w = [p for r in d["rounds"] for p in r["parts"] if p.get("winner")]
            sw = [p for p in w if p["winner"].startswith("storm-")]
            ser = d["storm"]["series"]
            row = {
                **measures(d),
                "storm wins": len(sw),
                "storm paid fUSD": round(sum(p["price"] for p in sw if p["paid"]), 2),
                "storm earned fUSD": round(ser[-1]["storm_earned_fusd"], 2),
                "max fUSD held by storm keys": round(max(x["key_held_fusd"] for x in ser), 2),
            }
            if kind == "wash":
                wsh = d["wash"]
                row.update({
                    "sybils washed": wsh["sybils_washed"],
                    "wash platform fee fUSD": round(wsh["platform_fee_fusd"], 2),
                    "wash fUSD outlay": round(wsh["fusd_outlay"], 2),
                    "wash NEAR spent": round(wsh["near_spent"], 3),
                })
            out[a].append(row)
        out["rounds"] = len(ds[0]["rounds"])
        for i in range(out["rounds"]):
            row = {"round": i + 1}
            for k in ("junk_share", "junk_paid_fusd", "storm_earned_fusd", "key_held_fusd"):
                row[k] = round(sum(d["storm"]["series"][i][k] for d in ds) / len(ds), 4)
            out["series"][a].append(row)
    keys = list(out[arms[0]][0])
    out["mean"] = {a: {k: round(sum(m[k] for m in out[a]) / len(seeds), 2) for k in keys} for a in arms}
    print(f"storm at round {out['storm_round']}, {len(seeds)} seeds: {seeds}")
    for k in keys:
        print(f"{k:30}" + "".join(f"{a}: {out['mean'][a][k]:>9.2f}  " for a in arms))
    bad = 0
    if "--check" in sys.argv:
        ok = json.load(open(f"web/swarm-{kind}.json")) == out
        print(("ok  " if ok else "FAIL"), f"web/swarm-{kind}.json matches the raw {kind} runs")
        bad += not ok
    else:
        json.dump(out, open(f"web/swarm-{kind}.json", "w"), indent=1)
    if kind == "wash" and "records+allowlist" in arms:
        fewer = sum(x["storm wins"] < r["storm wins"] for x, r in zip(out["records+allowlist"], out["records"]))
        out["claim"] = f"fewer storm wins with the payer list on {fewer} of {len(seeds)} seeds"
        print(out["claim"])
    if "--readme" in sys.argv:
        readme = re.sub(r"\s+", " ", open("README.md").read())
        if "claim" in out:
            ok = out["claim"] in readme
            print(("ok  " if ok else "FAIL"), f"README states: {out['claim']}")
            bad += not ok
        for a in arms:
            for k in (STORM_README if kind == "storm" else WASH_README):
                s = f"{out['mean'][a][k]:.2f}"
                ok = s in readme
                print(("ok  " if ok else "FAIL"), f"README states the {kind} {a} mean of {k}: {s}")
                bad += not ok
    return 1 if bad else 0


if "--wash" in sys.argv:
    sys.exit(storm_summary("wash"))
if "--storm" in sys.argv:
    sys.exit(storm_summary())

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
