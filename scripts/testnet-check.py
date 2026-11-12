#!/usr/bin/env python3
"""Check the testnet run's record against the README.

    python3 scripts/testnet-check.py [web/scene-testnet.json] [--readme]

- every transaction in the scene succeeded, and it ran on NEAR testnet;
- the README states its transaction and NEAR AI call counts and its factory;
- every transaction the README links is in the scene;
- if demo/testnet-judge-cassette.json exists (`hiretree testnet-answers`),
  it holds one signed answer per submit_check transaction, by the scene's
  signer (make verify-cassette re-checks the signatures), each in rubric-2
  format.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    scene_f = pathlib.Path(args[0]) if args else ROOT / "web/scene-testnet.json"
    s = json.loads(scene_f.read_text())
    run = s["run"]
    txs = run["transactions"]
    bad = []
    if s.get("network") != "NEAR testnet":
        bad.append(f"network is {s.get('network')!r}, not NEAR testnet")
    failed = [t["tx"] for t in txs if not t.get("ok")]
    if failed:
        bad.append(f"{len(failed)} failed transactions: {failed[:3]}")
    checks = run.get("transactions") and [t for t in txs if t["method"] == "submit_check"]
    if "--readme" in sys.argv:
        readme = (ROOT / "README.md").read_text()
        for want in (f"{len(txs)} transactions", f"{run['model_calls']} signed NEAR AI calls", f"`{run['factory']}`"):
            if want not in re.sub(r"\s+", " ", readme):
                bad.append(f"README does not state {want!r}")
        hashes = {t["tx"] for t in txs}
        for h in re.findall(r"testnet\.nearblocks\.io/txns/(\w+)", readme):
            if h not in hashes:
                bad.append(f"README links {h}, which is not in {scene_f.name}")
    cas_f = ROOT / "demo/testnet-judge-cassette.json"
    if cas_f.exists():
        cas = json.loads(cas_f.read_text())
        if len(cas["entries"]) != len(checks):
            bad.append(f"{cas_f.name} has {len(cas['entries'])} answers, the scene {len(checks)} submit_check transactions")
        if cas["signer"].lower() != run["signer"].lower():
            bad.append(f"{cas_f.name} signer {cas['signer']} is not the scene's {run['signer']}")
        for i, e in enumerate(cas["entries"]):
            content = json.loads(e["resp"])["choices"][0]["message"]["content"]
            if '"quotes"' not in content:
                bad.append(f"answer {i} is not in rubric-2 format: {content[:60]}")
    if bad:
        print("FAIL testnet run:\n  " + "\n  ".join(bad))
        sys.exit(1)
    extra = f", {len(checks)} signed judge answers on file" if cas_f.exists() else ""
    print(f"ok   testnet run: {len(txs)} transactions, 0 failed, factory {run['factory']}{extra}")


if __name__ == "__main__":
    main()
