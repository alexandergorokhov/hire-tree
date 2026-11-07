#!/usr/bin/env python3
"""Draw docs/screens/recursive-market.svg from one real tree in web/swarm.json.

Picks the deepest paid tree of the run and draws it: client -> factory fee ->
job account -> parts -> sub-parts, each node a NEAR account with its money.
"""
import json, sys
from html import escape

d = json.load(open(sys.argv[1] if len(sys.argv) > 1 else "web/swarm.json"))
out = sys.argv[2] if len(sys.argv) > 2 else "docs/screens/recursive-market.svg"

best = None
for r in d["rounds"]:
    parts = {p["id"]: p for p in r["parts"]}
    for p in r["parts"]:
        if p["level"] == 1 and p.get("winner"):
            kids = lambda q: [k for k in r["parts"] if k.get("parent") == q["id"] and k.get("winner")]
            chain = [p]
            while kids(chain[-1]):
                chain.append(max(kids(chain[-1]), key=lambda k: k["level"]))
            key = (len(chain), all(c["paid"] for c in chain))
            if best is None or key > best[0]:
                best = (key, r, chain)
_, r, chain = best
top = chain[0]
root_parts = [p for p in r["parts"] if p["lead"] == top["lead"] and p["level"] == 1]
budget = sum(p["max"] for p in root_parts)
bps = d.get("platform_fee_bps", 50)
fee = budget * bps / 10000

W, H = 960, 570
bond = round(float(d.get("bond_near", 0.05)), 4)
short = lambda j: j.replace(".sandbox", "").replace(".testnet", "")
o = []
a = o.append
a(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" font-family="Inter,Segoe UI,Helvetica,Arial,sans-serif">')
a('<rect width="100%" height="100%" rx="14" fill="#fbfaf7"/>')
a('<defs><marker id="ar" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0L10 5L0 10z" fill="#6b6a66"/></marker></defs>')
a(f'<text x="28" y="40" font-size="20" font-weight="700" fill="#1f1e1c">Every job is a NEAR account. Any worker can hire below itself.</text>')
a(f'<text x="28" y="64" font-size="13.5" fill="#55534e">One real tree from the swarm run (round {r["round"]}, {escape(top["lead"])} lead). Money only flows down a level when that account\'s work is bought.</text>')

def box(x, y, w, h, title, sub, sub2, fill, stroke):
    a(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>')
    a(f'<text x="{x+14}" y="{y+24}" font-size="14" font-weight="700" fill="#1f1e1c">{escape(title)}</text>')
    a(f'<text x="{x+14}" y="{y+44}" font-size="12.5" fill="#3d3c38">{escape(sub)}</text>')
    if sub2:
        a(f'<text x="{x+14}" y="{y+62}" font-size="12" fill="#6b6a66">{escape(sub2)}</text>')

def arrow(x1, y1, x2, y2, label=None):
    a(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="#6b6a66" stroke-width="1.6" marker-end="url(#ar)"/>')
    if label:
        a(f'<text x="{max(x1, x2)+10}" y="{(y1+y2)/2+4}" font-size="11.5" fill="#6b6a66">{escape(label)}</text>')

# client and job account
box(28, 92, 200, 74, "Client", f"posts {budget:.2f} fUSD", "one ft_transfer_call", "#ffffff", "#c9c6bf")
lead_acct = short(top["job"]).split(".", 1)[1] if top.get("job") else "job-n.hub"
box(300, 92, 250, 74, f"{lead_acct}  ({top['lead']} lead)", f"holds {budget - fee:.2f} fUSD", f"factory keeps {bps/100:g}% = {fee:.2f}", "#eef3fb", "#7a9cd1")
arrow(228, 129, 298, 129)

# chain of parts, each a level down
colors = [("#eef7ef", "#6fae78"), ("#fdf3e6", "#d79a4a"), ("#f6eefb", "#a57ac9")]
x, y = 330, 214
prev = (340, 166)
for i, p in enumerate(chain):
    fill, stroke = colors[min(i, 2)]
    title = f"{short(p['job'])}  ·  {p['skill']}"
    sub = f"{p['winner']} won at {p['price']:.2f} of {p['max']:.2f} max"
    nb = len(p.get("bids", []))
    sub2 = f"{nb} {'sealed ' if p.get('sealed') else ''}bids, {bond:g} NEAR bond each · check {p.get('score', '?')}/10 · {'paid early' if p['paid'] else 'not paid, buyer keeps bond'}"
    box(x, y, 430, 74, title, sub, sub2, fill, stroke)
    if i == 0:
        label = "hires: open bids, award"
    else:
        q = chain[i-1]
        label = f"{q['winner']} hires out of its own pay"
        if q["paid"] and p["paid"]:
            label += f", keeps {q['price'] - p['price']:.2f}"
    arrow(prev[0], prev[1], prev[0], y, label)
    prev = (x + 30, y + 74)
    x += 30
    y += 108

# rules column
rx, ry = 28, 200
rules = [
    "What each account does by itself:",
    "1  takes bids (sealed: commit, then reveal)",
    "2  holds the winner's NEAR bond",
    "3  checks the NEAR AI signature on the work",
    "4  pays early when the check passes",
    "5  junk: buyer keeps the bond, no pay",
    "6  sends the rest up and deletes itself",
]
for i, t in enumerate(rules):
    a(f'<text x="{rx}" y="{ry + 18 + i*22}" font-size="{13 if i==0 else 12.5}" font-weight="{700 if i==0 else 400}" fill="#3d3c38">{escape(t)}</text>')
a(f'<text x="28" y="{H-20}" font-size="11.5" fill="#8a8882">Drawn by scripts/pitch-svg.py from web/swarm.json (make swarm). Fake test USD on a local NEAR node.</text>')
a('</svg>')
open(out, "w").write("\n".join(o))
print("wrote", out, "depth", len(chain), [c["winner"] for c in chain])
