"""JLCPCB order files (KiCad's Python):  export_order.py <board.kicad_pcb> <out_dir>

- gerbers.zip: every copper layer, masks, silks, paste, Edge.Cuts, and Excellon drills (PTH and
  NPTH apart), through kicad-cli;
- bom.csv (Comment, Designator, Footprint, LCSC Part #): fitted parts only (test points, fiducials,
  mounting holes and solder jumpers are bare copper), one line per footprint and LCSC number;
- cpl.csv (Designator, Mid X, Mid Y, Layer, Rotation): from kicad-cli's position file, in the
  Gerbers' coordinates, DNP parts left out, turned and shifted onto JLC's own footprints
  (jlc_corrections.json, from jlc_fp_check.py).
Parts with no LCSC number are listed at the end: those go through Global Sourcing.
"""
import csv
import json
import math
import os
import subprocess
import sys
import zipfile

import pcbnew

CLI = os.environ.get("KICAD_CLI", r"C:\Program Files\KiCad\10.0\bin\kicad-cli.exe")
board_path, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
gdir = os.path.join(out, "gerbers")
os.makedirs(gdir, exist_ok=True)
layers = "F.Cu,In1.Cu,In2.Cu,B.Cu,F.Mask,B.Mask,F.Silkscreen,B.Silkscreen,F.Paste,B.Paste,Edge.Cuts"
subprocess.run([CLI, "pcb", "export", "gerbers", "--layers", layers, "--subtract-soldermask",
                "--no-protel-ext", "-o", gdir + os.sep, board_path], check=True)
subprocess.run([CLI, "pcb", "export", "drill", "--format", "excellon", "--excellon-separate-th",
                "--excellon-units", "mm", "--generate-map", "--map-format", "gerberx2", "-o", gdir + os.sep, board_path], check=True)
with zipfile.ZipFile(os.path.join(out, "gerbers.zip"), "w", zipfile.ZIP_DEFLATED) as z:
    for f in sorted(os.listdir(gdir)):
        z.write(os.path.join(gdir, f), f)

board = pcbnew.LoadBoard(board_path)
NOT_PARTS = {"TestPoint", "Fiducial", "MountingHole", "Jumper"}     # copper only: nothing to place


def field(f, name):
    fl = f.GetField(name)
    return fl.GetText() if fl else ""


groups, comments, no_lcsc = {}, {}, []
for f in board.GetFootprints():
    a = f.GetAttributes()
    if f.IsDNP() or (a & pcbnew.FP_EXCLUDE_FROM_BOM) or f.GetFPID().GetLibNickname().wx_str() in NOT_PARTS:
        continue
    lcsc = field(f, "LCSC")
    fp = f.GetFPID().GetLibItemName().wx_str()
    # one line per LCSC part: JLC unticks lines that share a part ("10k" and "10k 1%") until each is confirmed
    key = (fp, lcsc) if lcsc else (fp, "", f.GetValue())
    groups.setdefault(key, []).append(f.GetReference())
    comments.setdefault(key, set()).add(f.GetValue())
    if not lcsc:
        no_lcsc.append(f"{f.GetReference()} {f.GetValue()} {field(f, 'MPN')}")


def natural(r):
    head = r.rstrip("0123456789")
    return (head, int(r[len(head):] or 0))


with open(os.path.join(out, "bom.csv"), "w", newline="", encoding="utf-8") as fh:
    w = csv.writer(fh)
    w.writerow(["Comment", "Designator", "Footprint", "LCSC Part #"])
    for key, refs in sorted(groups.items(), key=lambda kv: natural(sorted(kv[1], key=natural)[0])):
        fp, lcsc = key[0], key[1]
        val = max(comments[key], key=len)
        w.writerow([val, ",".join(sorted(refs, key=natural)), fp, lcsc])

# JLC places its own (EasyEDA) footprint for each LCSC part at the CPL's position and rotation.
# jlc_fp_check.py finds, per part, the turn theta and shift t with JLC's pads = ours turned by theta + t
# (y up, counter-clockwise); then JLC's rotation is ours - theta and its centre ours - R(that) t.
CORR_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "jlc_corrections.json")
corr = {r["ref"]: r for r in json.load(open(CORR_PATH, encoding="utf-8")) if "theta" in r} if os.path.exists(CORR_PATH) else {}
lcsc_of = {r: k[1] for k, refs in groups.items() for r in refs}
corrected, stale = [], []


def jlc_place(ref, x, y, rot):
    c = corr.get(ref)
    if not c:
        return x, y, rot
    if c["lcsc"] != lcsc_of.get(ref):
        stale.append(ref)
        return x, y, rot
    jr = (rot - c["theta"]) % 360
    tx, ty = c["offset"]
    if math.hypot(tx, ty) < 0.05:
        tx = ty = 0.0
    a = math.radians(jr)
    x2, y2 = x - (tx * math.cos(a) - ty * math.sin(a)), y - (tx * math.sin(a) + ty * math.cos(a))
    if c["theta"] or tx or ty:
        corrected.append(f"{ref} {c['theta']:+d}deg" + (f" {math.hypot(tx, ty):.2f}mm" if tx or ty else ""))
    return x2, y2, jr


pos = os.path.join(out, "pos_kicad.csv")
subprocess.run([CLI, "pcb", "export", "pos", "--format", "csv", "--units", "mm", "--side", "front",
                "--exclude-dnp", "-o", pos, board_path], check=True)
fitted = {r for refs in groups.values() for r in refs}
n = 0
with open(pos, newline="", encoding="utf-8") as fi, open(os.path.join(out, "cpl.csv"), "w", newline="", encoding="utf-8") as fo:
    w = csv.writer(fo)
    w.writerow(["Designator", "Mid X", "Mid Y", "Layer", "Rotation"])
    for row in csv.DictReader(fi):
        if row["Ref"] not in fitted:
            continue
        x, y, rot = jlc_place(row["Ref"], float(row["PosX"]), float(row["PosY"]), float(row["Rot"]))
        w.writerow([row["Ref"], f"{x:.4f}mm", f"{y:.4f}mm", "Top" if row["Side"] == "top" else "Bottom", f"{rot:g}"])
        n += 1
os.remove(pos)
print(f"export_order: gerbers.zip, bom.csv ({len(groups)} lines, {len(fitted)} parts), cpl.csv ({n} parts)")
print("no LCSC number (Global Sourcing):", no_lcsc or "none")
print(f"JLC footprint corrections ({len(corrected)}):", ", ".join(corrected) or "none")
print("no JLC footprint check (place by eye in JLC's preview):", sorted(fitted - set(corr)) or "none")
if stale:
    print("STALE corrections (LCSC changed; re-run jlc_fp_check.py):", stale)
