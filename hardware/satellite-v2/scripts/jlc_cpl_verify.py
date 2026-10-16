"""End-to-end check of cpl.csv: put JLC's footprint for each part where cpl.csv says (y up, CCW
rotation, the Gerber origin), and measure each of its pads against our same-numbered pad on the board.

    jlc_cpl_verify.py <board.kicad_pcb> <cpl.csv> <edafp dir>
"""
import csv, math, sys
from pathlib import Path
import pcbnew

board = pcbnew.LoadBoard(sys.argv[1])
cpl = {r["Designator"]: r for r in csv.DictReader(open(sys.argv[2], encoding="utf-8"))}
bom = {}
for r in csv.DictReader(open(Path(sys.argv[2]).parent / "bom.csv", encoding="utf-8")):
    for ref in r["Designator"].split(","):
        bom[ref] = r["LCSC Part #"]
eda = Path(sys.argv[3])
worst = []
for fp in board.GetFootprints():
    ref = fp.GetReference()
    if ref not in cpl:
        continue
    mods = list((eda / f"{bom[ref]}.pretty").glob("*.kicad_mod"))
    if not mods:
        worst.append((None, ref, "no JLC footprint"))
        continue
    e = pcbnew.FootprintLoad(str(mods[0].parent), mods[0].stem)
    x0, y0 = float(cpl[ref]["Mid X"][:-2]), float(cpl[ref]["Mid Y"][:-2])
    a = math.radians(float(cpl[ref]["Rotation"]))
    ours = {}
    for p in fp.Pads():
        ours.setdefault(p.GetNumber(), []).append((pcbnew.ToMM(p.GetPosition().x), -pcbnew.ToMM(p.GetPosition().y)))
    d = 0.0
    n = 0
    for p in e.Pads():
        num = p.GetNumber()
        if num not in ours:
            continue
        lx, ly = pcbnew.ToMM(p.GetPosition().x), -pcbnew.ToMM(p.GetPosition().y)
        wx, wy = x0 + lx * math.cos(a) - ly * math.sin(a), y0 + lx * math.sin(a) + ly * math.cos(a)
        d = max(d, min(math.hypot(wx - u, wy - v) for u, v in ours[num]))
        n += 1
    worst.append((round(d, 3), ref, f"{n} pads"))
worst.sort(key=lambda w: -1 if w[0] is None else w[0], reverse=True)
for w in worst[:20]:
    print(w)
print("parts:", len(worst), " over 0.25 mm:", [w[1] for w in worst if w[0] is not None and w[0] > 0.25])
