#!/usr/bin/env python3
"""The storm summary (swarm-seeds.py --storm) on tiny made-up runs: it
averages the per-round series over seeds, counts storm wins by name, and
--check catches a summary that differs from the raw runs.

    python3 scripts/test-swarm-storm.py
"""
import json, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT = os.path.join(ROOT, "scripts", "swarm-seeds.py")


def part(winner, strategy, price, paid):
    return {"winner": winner, "strategy": strategy, "price": price, "paid": paid, "level": 1}


def run(arm, seed, junk_share):
    parts = [part("storm-res-00", "polished", 6.0, True), part("res-steady", "steady", 20.0, True)]
    return {
        "arm": arm, "seed": seed, "blind": arm != "records",
        "rounds": [{"parts": parts}, {"parts": parts}],
        "leads": [{"spent": 26.0}],
        "storm": {"round": 2, "series": [
            {"round": r, "junk_share": junk_share, "junk_paid_fusd": 6.0, "storm_earned_fusd": 3.0 * r, "key_held_fusd": 0.0}
            for r in (1, 2)]},
    }


def go(cwd, *args):
    return subprocess.run([sys.executable, SCRIPT, "--storm", *args], cwd=cwd, capture_output=True, text=True)


fails = []
def check(ok, what):
    print(("ok   " if ok else "FAIL ") + what)
    if not ok:
        fails.append(what)

tmp = tempfile.mkdtemp()
try:
    os.makedirs(os.path.join(tmp, "runs", "storm"))
    os.makedirs(os.path.join(tmp, "web"))
    for arm, tag in (("records", "rec"), ("blind", "blind")):
        for seed, share in ((1, 0.2), (2, 0.4)):
            json.dump(run(arm, seed, share + (0.1 if arm == "blind" else 0)),
                      open(os.path.join(tmp, "runs", "storm", f"{tag}-{seed}.json"), "w"))
    r = go(tmp)
    check(r.returncode == 0, "the summary is written")
    s = json.load(open(os.path.join(tmp, "web", "swarm-storm.json")))
    check(s["storm_round"] == 2 and s["seeds"] == [1, 2] and s["arms"] == ["records", "blind"], "storm round, seeds and arms")
    check(abs(s["series"]["records"][0]["junk_share"] - 0.3) < 1e-9, "series are the mean over seeds")
    check(abs(s["series"]["blind"][1]["junk_share"] - 0.4) < 1e-9, "each arm has its own series")
    check(s["records"][0]["storm wins"] == 2 and s["records"][0]["storm paid fUSD"] == 12.0, "storm wins are counted by account name")
    check(s["records"][0]["max fUSD held by storm keys"] == 0.0, "fUSD held by storm keys is reported")
    check(go(tmp, "--check").returncode == 0, "--check passes on a fresh summary")
    s["mean"]["records"]["junk wins"] += 1
    json.dump(s, open(os.path.join(tmp, "web", "swarm-storm.json"), "w"))
    check(go(tmp, "--check").returncode == 1, "--check fails when the summary differs from the runs")
    empty = tempfile.mkdtemp()
    check(go(empty).returncode == 0 and not os.path.exists(os.path.join(empty, "web")), "no storm runs: nothing written")
    shutil.rmtree(empty)
finally:
    shutil.rmtree(tmp)
sys.exit(1 if fails else 0)
