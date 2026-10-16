"""Compare every placed footprint with JLC's own (EasyEDA) footprint for its LCSC part.

    jlc_fp_check.py <board.kicad_pcb> <edafp dir> <out.json>

<edafp dir>/<LCSC>.pretty/*.kicad_mod come from easyeda2kicad. Pads are matched by number; the best
of the four 90-degree turns (and the matching shift) is found by least squares, in a y-up frame where
positive angles are counter-clockwise seen from the top, as in KiCad's position file. For each part:
pad-name sets, the residual (how far our pads sit from JLC's after the best fit), pad-size differences,
the turn theta (JLC footprint = our footprint turned by theta) and the origin offset.
"""
import json
import math
import sys
from pathlib import Path

import pcbnew

board_path, eda_dir, out = sys.argv[1], Path(sys.argv[2]), sys.argv[3]
board = pcbnew.LoadBoard(board_path)


def pads_of(fp, local):
    res = {}
    for p in fp.Pads():
        num = p.GetNumber()
        if not num:
            continue
        if local:
            pos = p.GetFPRelativePosition()
            ang = p.GetFPRelativeOrientation().AsDegrees()
        else:
            pos = p.GetPosition()
            ang = p.GetOrientation().AsDegrees()
        sz = p.GetSize(pcbnew.F_Cu)
        res.setdefault(num, []).append((pcbnew.ToMM(pos.x), -pcbnew.ToMM(pos.y), pcbnew.ToMM(sz.x), pcbnew.ToMM(sz.y), ang,
                                        p.GetAttribute() == pcbnew.PAD_ATTRIB_PTH))
    return res


def rot(x, y, deg):
    c, s = math.cos(math.radians(deg)), math.sin(math.radians(deg))
    return x * c - y * s, x * s + y * c


def fit(ours, theirs):
    """Pads present in both, one-to-one by number (multi-pad numbers paired in order after sorting)."""
    best = None
    names = sorted(n for n in set(ours) & set(theirs) if len(ours[n]) == len(theirs[n]))
    if len(names) < 2:                        # numbering differs (e.g. a switch 1,1,2,2 vs 1-4): match by shape
        return geo_fit(ours, theirs)
    for th in (0, 90, 180, 270):
        pairs = []
        for n in names:
            a = sorted(ours[n], key=lambda q: (round(rot(q[0], q[1], th)[0], 1), round(rot(q[0], q[1], th)[1], 1)))
            b = sorted(theirs[n], key=lambda q: (round(q[0], 1), round(q[1], 1)))
            for qa, qb in zip(a, b):
                pairs.append((qa, qb))
        if not pairs:
            continue
        ra = [rot(qa[0], qa[1], th) for qa, _ in pairs]
        tx = sum(qb[0] - r[0] for r, (_, qb) in zip(ra, pairs)) / len(pairs)
        ty = sum(qb[1] - r[1] for r, (_, qb) in zip(ra, pairs)) / len(pairs)
        err = max(math.hypot(r[0] + tx - qb[0], r[1] + ty - qb[1]) for r, (_, qb) in zip(ra, pairs))
        size = 0.0
        for qa, qb in pairs:
            w, h = (qa[2], qa[3]) if (th + qa[4]) % 180 == 0 else (qa[3], qa[2])
            wb, hb = (qb[2], qb[3]) if qb[4] % 180 == 0 else (qb[3], qb[2])
            size = max(size, abs(w - wb), abs(h - hb))
        cand = (err, th, tx, ty, size, len(pairs))
        if best is None or cand[0] < best[0] - 1e-6:
            best = cand
    return best


def flat(d):
    return [q for v in d.values() for q in v]


def geo_fit(ours, theirs):
    a, b = flat(ours), flat(theirs)
    best = None
    for th in (0, 90, 180, 270):
        ra = [rot(q[0], q[1], th) for q in a]
        tx = sum(q[0] for q in b) / len(b) - sum(r[0] for r in ra) / len(ra)
        ty = sum(q[1] for q in b) / len(b) - sum(r[1] for r in ra) / len(ra)
        err = max(min(math.hypot(r[0] + tx - q[0], r[1] + ty - q[1]) for q in b) for r in ra)
        if best is None or err < best[0]:
            best = (err, th, tx, ty, float("nan"), 0)
    return best


def nearest(ours, theirs, th, tx, ty):
    """Every pad of ours to the nearest JLC pad and back: catches extra/missing pads and odd shells."""
    a = [(rot(q[0], q[1], th)[0] + tx, rot(q[0], q[1], th)[1] + ty) for q in flat(ours)]
    b = [(q[0], q[1]) for q in flat(theirs)]
    d1 = max(min(math.hypot(x - u, y - v) for u, v in b) for x, y in a)
    d2 = max(min(math.hypot(x - u, y - v) for x, y in a) for u, v in b)
    return round(d1, 3), round(d2, 3)


bom = {}
for line in open(Path(board_path).parents[1] / "order" / "bom.csv", encoding="utf-8").read().splitlines()[1:]:
    import csv
    row = next(csv.reader([line]))
    for ref in row[1].split(","):
        bom[ref.strip()] = (row[0], row[2], row[3].strip())

results = []
for fp in board.GetFootprints():
    ref = fp.GetReference()
    if ref not in bom:
        continue
    value, fpname, lcsc = bom[ref]
    lib = eda_dir / f"{lcsc}.pretty"
    mods = list(lib.glob("*.kicad_mod")) if lib.exists() else []
    if not mods:
        results.append(dict(ref=ref, lcsc=lcsc, value=value, status="no JLC footprint"))
        continue
    eda = pcbnew.FootprintLoad(str(lib), mods[0].stem)
    ours, theirs = pads_of(fp, True), pads_of(eda, False)
    only_ours = sorted(set(ours) - set(theirs))
    only_theirs = sorted(set(theirs) - set(ours))
    counts = {n: (len(ours[n]), len(theirs[n])) for n in set(ours) & set(theirs) if len(ours[n]) != len(theirs[n])}
    f = fit(ours, theirs)
    rotation = fp.GetOrientation().AsDegrees()
    results.append(dict(ref=ref, lcsc=lcsc, value=value, ours=fpname, jlc=mods[0].stem, rot=rotation,
                        only_ours=only_ours, only_jlc=only_theirs, count_diff=counts,
                        resid=round(f[0], 3) if f else None, theta=f[1] if f else None,
                        offset=(round(f[2], 3), round(f[3], 3)) if f else None,
                        size_diff=round(f[4], 3) if f else None, matched=f[5] if f else 0,
                        nearest=nearest(ours, theirs, f[1], f[2], f[3]) if f else None))
json.dump(results, open(out, "w"), indent=1)
print("checked", len(results))
