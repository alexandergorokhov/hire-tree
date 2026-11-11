#!/usr/bin/env python3
"""The swarm checker must catch a broken run: each case below breaks one
rule in a copy of web/swarm.json (or docs/SWARM.md) and expects a FAIL.

    python3 scripts/test-swarm-check.py
"""
import copy, json, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CHECK = os.path.join(ROOT, "scripts", "swarm-check.py")
base = json.load(open(os.path.join(ROOT, "web", "swarm.json")))


def won(d):
    return [p for r in d["rounds"] for p in r["parts"] if p.get("winner")]


def case_paid_flag_without_money(d):
    # Mark a failed part paid: the on-chain balances disagree.
    p = next(p for p in won(d) if not p["paid"])
    p["paid"], p["score"] = True, 9
    if p.get("audit"):
        p["audit"]["caught"] = False


def case_fired_a_good_worker(d):
    p = next(p for p in won(d) if p["paid"])
    p["fired"] = True


def case_fees_off(d):
    d["platform_fees"] += 1


def case_overpaid(d):
    p = won(d)[0]
    p["price"] = p["max"] + 1


def case_audit_against_votes(d):
    p = next(p for p in won(d) if p.get("audit"))
    p["audit"]["caught"] = not p["audit"]["caught"]


def with_earned_fees(d, rule="contract"):
    # Fill in lead_fee_paid as the contract pays it, or by a broken rule:
    # "whole" pays the whole fee whatever the outcome (the old rule);
    # "count_replaced" also counts a failed part its runner-up redid;
    # "no_floor" divides by what was hired even below the spend floor;
    # "old" is the rule before the floor, in a run that records none.
    if rule == "old":
        d.pop("fee_floor_bps", None)
    else:
        d["fee_floor_bps"] = 5000
    budget = round(d["job_budget"] * 1e6)
    funded = budget - budget * d["platform_fee_bps"] // 10000
    for r in d["rounds"]:
        for f in r["intents"]:
            mine = [p for p in r["parts"] if p.get("winner") and p["level"] == 1 and p.get("lead") == f["lead"]]
            def first(p):  # the first hire for this part's task
                while p.get("replaces") is not None:
                    p = r["parts"][p["replaces"]]
                return p["id"]
            redone = {first(p) for p in mine if p["paid"]}
            counted = [p for p in mine if p["paid"] or rule == "count_replaced" or first(p) not in redone]
            hired = sum(round(p["price"] * 1e6) for p in counted)
            paid = sum(round(p["price"] * 1e6) for p in mine if p["paid"])
            fee = round(f["lead_fee"] * 1e6)
            floor = 0 if rule in ("no_floor", "old") else -(-(funded - fee) // 2)
            f["lead_fee_paid"] = (fee if rule == "whole" else (fee * paid // max(hired, floor) if hired else 0)) / 1e6


def case_lead_fee_paid_whatever_the_outcome(d):
    with_earned_fees(d, rule="whole")


def case_lead_fee_counts_a_part_its_runner_up_redid(d):
    with_earned_fees(d, rule="count_replaced")


def case_lead_fee_ignores_the_spend_floor(d):
    with_earned_fees(d, rule="no_floor")


CASES = [case_paid_flag_without_money, case_fired_a_good_worker, case_fees_off, case_overpaid, case_audit_against_votes, case_lead_fee_paid_whatever_the_outcome, case_lead_fee_counts_a_part_its_runner_up_redid, case_lead_fee_ignores_the_spend_floor]


def run(d, readme=None):
    with tempfile.TemporaryDirectory() as t:
        path = os.path.join(t, "swarm.json")
        json.dump(d, open(path, "w"))
        os.mkdir(os.path.join(t, "docs"))
        shutil.copy(os.path.join(ROOT, "docs", "SWARM.md") if readme is None else readme, os.path.join(t, "docs", "SWARM.md"))
        r = subprocess.run([sys.executable, CHECK, path, "--readme"], cwd=t, capture_output=True, text=True)
        return r.returncode, r.stdout


fails = 0
code, out = run(base)
print(("ok  " if code == 0 else "FAIL"), "the committed run passes")
fails += code != 0
d = copy.deepcopy(base)
with_earned_fees(d)
code, out = run(d)
ok = code == 0 and "every lead earned its fee pro rata to its parts that passed, over at least 50% of the work budget" in out
print(("ok  " if ok else "FAIL"), "a run whose leads earned their fees pro rata over the spend floor passes")
fails += not ok
d = copy.deepcopy(base)
with_earned_fees(d, rule="old")
code, out = run(d)
ok = code == 0 and "every lead earned its fee pro rata to its parts that passed (" in out
print(("ok  " if ok else "FAIL"), "a run made before the spend floor (no fee_floor_bps) passes by the old rule")
fails += not ok
for case in CASES:
    d = copy.deepcopy(base)
    case(d)
    code, out = run(d)
    ok = code != 0
    print(("ok  " if ok else "FAIL"), "caught:", case.__name__[5:].replace("_", " "))
    fails += not ok

# A docs/SWARM.md number with extra leading digits must not pass.
readme = open(os.path.join(ROOT, "docs", "SWARM.md")).read()
n = sum(1 for p in won(base) if p["paid"])
assert f"{n} passed" in readme
with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as f:
    f.write(readme.replace(f"{n} passed", f"9{n} passed"))
code, _ = run(base, f.name)
os.unlink(f.name)
print(("ok  " if code != 0 else "FAIL"), "caught: a docs/SWARM.md number with an extra leading digit")
fails += code == 0

print("swarm-check self-test:", "all ok" if not fails else f"{fails} failed")
sys.exit(1 if fails else 0)
