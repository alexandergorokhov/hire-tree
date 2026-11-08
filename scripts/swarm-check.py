#!/usr/bin/env python3
"""Check a swarm run (web/swarm.json by default) against its own rules and,
with --readme, the numbers the README states about the committed run.

  python3 scripts/swarm-check.py [swarm.json] [--readme]
"""
import json, re, sys

args = [a for i, a in enumerate(sys.argv[1:], 1) if not a.startswith("--") and sys.argv[i - 1] != "--same-as"]
path = args[0] if args else "web/swarm.json"
d = json.load(open(path))
fails = []
def check(ok, what):
    print(("ok   " if ok else "FAIL ") + what)
    if not ok:
        fails.append(what)

parts = [p for r in d["rounds"] for p in r["parts"]]
won = [p for p in parts if p.get("winner")]
subs = [p for p in won if p["level"] > 1]
jobs = len(d["rounds"]) * len(d["leads"])
paid = [p for p in won if p["paid"]]
lost = sum(a.get("bonds_lost", 0) for a in d["agents"])
fee = jobs * d.get("job_budget", 70) * d["platform_fee_bps"] / 10000

check(len(won) == len(parts), f"every part found a seller ({len(won)}/{len(parts)})")
check(all(p["price"] <= p["max"] + 1e-9 for p in won), "no winner was paid above the part's max")
check(len(subs) > 0, f"contractors bought sub-parts out of their own pay ({len(subs)})")
for p in subs:
    r = next(r for r in d["rounds"] if p in r["parts"])
    parent = r["parts"][p["parent"]]
    if p["max"] >= parent["price"]:
        check(False, f"sub-part max {p['max']} below its contractor's price {parent['price']}")
        break
else:
    check(True, "every sub-part's max is below what its contractor won")
check(all(p["passed"] if "passed" in p else True for p in paid), "paid parts passed the check")
check(all(p["score"] >= 6 for p in paid) and all(p["score"] < 6 for p in won if not p["paid"]), "paid exactly when the score was 6 or more")
bonded_unpaid = [p for p in won if not p["paid"] and p.get("replaces") is None]
check(lost == len(bonded_unpaid), f"every unpaid winner lost its bond ({lost} bonds, {len(bonded_unpaid)} unpaid)")
fired = [p for p in won if p.get("fired")]
check(all(not p["paid"] and p["score"] < 6 for p in fired), f"only failed winners were fired ({len(fired)})")
reps = [(r, p) for r in d["rounds"] for p in r["parts"] if p.get("replaces") is not None]
ok = True
for r, p in reps:
    f = r["parts"][p["replaces"]]
    ok &= f.get("fired") and f["level"] == p["level"] and f.get("parent") == p.get("parent") and f["winner"] != p["winner"]
check(ok and len(reps) <= len(fired), f"every runner-up replaces a fired winner on the same part ({len(reps)})")
check(any(p["sealed"] for p in won), "sealed bids were used")
check(abs(d["platform_fees"] - fee) < 0.01, f"factory fees = {jobs} jobs x {d.get('job_budget', 70)} x {d['platform_fee_bps'] / 100}% = {fee:.2f}")

if "--same-as" in sys.argv:
    other = json.load(open(sys.argv[sys.argv.index("--same-as") + 1]))
    key = lambda x: [(p["id"], p.get("winner"), p.get("price"), p.get("score"), p.get("paid")) for r in x["rounds"] for p in r["parts"]]
    check(key(d) == key(other), f"same winners, prices, scores and payouts as {sys.argv[sys.argv.index('--same-as') + 1]}")

if "--readme" in sys.argv:
    readme = open("README.md").read()
    sec = readme[readme.index("## Swarm"):readme.index("## The problem")]
    def junk(rs):
        w = [p for r in rs for p in r["parts"] if p.get("winner")]
        return round(100 * sum(p["strategy"] == "junk" for p in w) / len(w))
    margin = 0.0
    for r in d["rounds"]:
        for p in r["parts"]:
            k = [q for q in r["parts"] if q.get("parent") == r["parts"].index(p) and q.get("winner") and not q.get("fired")]
            if k and p.get("winner") and p["paid"] and all(q["paid"] for q in k):
                margin += p["price"] - sum(q["price"] for q in k)
    want = {
        f"{len(won)} parts were bought": True,
        f"{len(subs)} of them by contractors": True,
        f"{len(paid)} passed": True,
        f"{junk(d['rounds'][:4])}% of wins in the first 4": True,
        f"{junk(d['rounds'][-4:])}% in the last 4": True,
        f"({lost} bonds)": True,
        f"{d['platform_fees']:.2f} fUSD in platform fees": True,
        f"{margin:.2f} fUSD of margin": True,
        f"fired {len(fired)} failed winners": True,
        f"{sum(p['paid'] for _, p in reps)} of {len(reps)} runner-ups passed": True,
    }
    flat = re.sub(r"\s+", " ", sec)
    for s in want:
        check(s in flat, f"README states: {s}")

print("swarm check:", "all ok" if not fails else f"{len(fails)} failed")
sys.exit(1 if fails else 0)
