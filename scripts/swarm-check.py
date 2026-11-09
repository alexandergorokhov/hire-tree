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
# Independent of the run's own flags: every agent's fUSD balance, read on-chain
# at the end, must equal what the paid flags say it earned (a lead-bought part
# paid early costs the worker the 1% note fee on what it kept; a contractor
# pays its paid sub-parts out of its pay).
exp = {a["agent"]: 0.0 for a in d["agents"]}
for r in d["rounds"]:
    for p in r["parts"]:
        if not p.get("winner") or not p["paid"]:
            continue
        kids = sum(k["price"] for k in r["parts"] if k.get("parent") == p["id"] and k.get("winner") and k["paid"])
        note_fee = 0.01 * (p["price"] - kids) if (p["paid_now"] and p["buyer"].startswith("lead-")) else 0
        exp[p["winner"]] += p["price"] - note_fee
        if p.get("parent") is not None:
            exp[r["parts"][p["parent"]]["winner"]] -= p["price"]
off = [a["agent"] for a in d["agents"] if abs(exp[a["agent"]] - a["earned"]) > 0.01]
check(not off, f"every agent's on-chain fUSD balance matches the parts marked paid ({len(d['agents'])} agents{', off: ' + ', '.join(off) if off else ''})")
caught = lambda p: bool(p.get("audit") and p["audit"].get("caught"))
check(all(p["score"] >= 6 and not caught(p) for p in paid) and all(p["score"] < 6 or caught(p) for p in won if not p["paid"]), "paid exactly when the score was 6 or more and no audit caught it")
audits = [p for p in won if p.get("audit")]
check(all(caught(p) == (sum(v < 6 for v in p["audit"]["votes"]) >= 2) for p in audits), f"every audit followed 2-of-3 failing votes ({len(audits)} audits, {sum(map(caught, audits))} caught)")
bonded_unpaid = [p for p in won if not p["paid"] and p.get("replaces") is None]
check(lost == len(bonded_unpaid), f"every unpaid winner lost its bond ({lost} bonds, {len(bonded_unpaid)} unpaid)")
fired = [p for p in won if p.get("fired")]
check(all(not p["paid"] and (p["score"] < 6 or caught(p)) for p in fired), f"only failed winners were fired ({len(fired)})")
reps = [(r, p) for r in d["rounds"] for p in r["parts"] if p.get("replaces") is not None]
ok = True
for r, p in reps:
    f = r["parts"][p["replaces"]]
    ok &= f.get("fired") and f["level"] == p["level"] and f.get("parent") == p.get("parent") and f["winner"] != p["winner"]
check(ok and len(reps) <= len(fired), f"every runner-up replaces a fired winner on the same part ({len(reps)})")
replaced = {(r["round"], p["replaces"]) for r, p in reps}
unreplaced = [(r["round"], p) for r in d["rounds"] for p in r["parts"] if p.get("fired") and (r["round"], p["id"]) not in replaced]
if any("no_runner_up" in p for _, p in unreplaced) or not unreplaced:
    check(all(p.get("no_runner_up") for _, p in unreplaced), f"every fired winner without a runner-up says why ({len(unreplaced)})")
    check(not any((p.get("no_runner_up") or "").startswith("hire failed") for _, p in unreplaced), "no runner-up hire failed")
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
        f"fooled the first check {sum(p['strategy'] == 'polished' and p['score'] >= 6 for p in won)} times": True,
        f"audits caught {sum(caught(p) for p in won if p['strategy'] == 'polished')}": True,
        f"paid {sum(p['strategy'] == 'polished' and p['paid'] for p in won)} times": True,
    }
    flat = re.sub(r"\s+", " ", sec)
    for s in want:
        # A number must stand alone: "120 passed" is not found in "9120 passed".
        pat = r"(?<![\d.])" + re.escape(s) + (r"(?![\d])" if s[-1].isdigit() else "")
        check(re.search(pat, flat) is not None, f"README states: {s}")

print("swarm check:", "all ok" if not fails else f"{len(fails)} failed")
sys.exit(1 if fails else 0)
