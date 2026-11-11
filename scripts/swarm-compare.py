#!/usr/bin/env python3
"""Compare the main swarm run with the control run (buyers blind to records).

    python3 scripts/swarm-compare.py web/swarm.json web/swarm-blind.json [--readme]

Same market, same agents, same contract; only the buyers' use of on-chain
records differs. Prints one row per measure. With --readme, also checks that
README.md or docs/RESULTS.md states the control-run numbers it prints.
"""
import json
import sys

BAD = ("junk", "polished")


def won(d, rs=None):
    rs = d["rounds"] if rs is None else rs
    return [p for r in rs for p in r["parts"] if p.get("winner")]


def measures(d):
    w = won(d)
    q = len(d["rounds"]) // 3

    def share(rs):
        x = won(d, rs)
        return round(100 * sum(p["strategy"] in BAD for p in x) / max(len(x), 1))

    bad_paid = sum(p["price"] for p in w if p["strategy"] in BAD and p["paid"])
    return {
        "bad wins": sum(p["strategy"] in BAD for p in w),
        "bad share first": share(d["rounds"][:q]),
        "bad share last": share(d["rounds"][-q:]),
        "fired": sum(1 for p in w if p.get("fired")),
        "paid to bad": round(bad_paid, 2),
        "parts paid": sum(1 for p in w if p["paid"]),
        "parts": len(w),
    }


def main():
    a, b = (json.load(open(f)) for f in sys.argv[1:3])
    assert not a.get("blind") and b.get("blind"), "first run must use records, second must be --blind"
    ma, mb = measures(a), measures(b)
    print(f"{'measure':<18}{'records':>10}{'blind':>10}")
    for k in ma:
        print(f"{k:<18}{ma[k]:>10}{mb[k]:>10}")
    if "--readme" in sys.argv:
        readme = (open("README.md").read() + open("docs/RESULTS.md").read())
        want = [
            f"{mb['bad share last']}% of wins in the last {len(b['rounds']) // 3} rounds",
            f"{mb['bad wins']} junk wins",
            f"{mb['fired']} fired",
            f"{mb['paid to bad']:.2f} fUSD paid to junk",
            f"{ma['paid to bad']:.2f} fUSD with records",
        ]
        bad = [s for s in want if s not in readme]
        for s in want:
            print(("FAIL" if s in bad else "ok  "), "README states:", s)
        sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
