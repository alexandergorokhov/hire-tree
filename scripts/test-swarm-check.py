#!/usr/bin/env python3
"""The swarm checker must catch a broken run: each case below breaks one
rule in a copy of web/swarm.json (or the README) and expects a FAIL.

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


CASES = [case_paid_flag_without_money, case_fired_a_good_worker, case_fees_off, case_overpaid, case_audit_against_votes]


def run(d, readme=None):
    with tempfile.TemporaryDirectory() as t:
        path = os.path.join(t, "swarm.json")
        json.dump(d, open(path, "w"))
        shutil.copy(os.path.join(ROOT, "README.md") if readme is None else readme, os.path.join(t, "README.md"))
        r = subprocess.run([sys.executable, CHECK, path, "--readme"], cwd=t, capture_output=True, text=True)
        return r.returncode, r.stdout


fails = 0
code, out = run(base)
print(("ok  " if code == 0 else "FAIL"), "the committed run passes")
fails += code != 0
for case in CASES:
    d = copy.deepcopy(base)
    case(d)
    code, out = run(d)
    ok = code != 0
    print(("ok  " if ok else "FAIL"), "caught:", case.__name__[5:].replace("_", " "))
    fails += not ok

# A README number with extra leading digits must not pass.
readme = open(os.path.join(ROOT, "README.md")).read()
n = sum(1 for p in won(base) if p["paid"])
assert f"{n} passed" in readme
with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as f:
    f.write(readme.replace(f"{n} passed", f"9{n} passed"))
code, _ = run(base, f.name)
os.unlink(f.name)
print(("ok  " if code != 0 else "FAIL"), "caught: a README number with an extra leading digit")
fails += code == 0

print("swarm-check self-test:", "all ok" if not fails else f"{fails} failed")
sys.exit(1 if fails else 0)
