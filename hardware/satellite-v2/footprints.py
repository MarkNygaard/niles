"""Generates the project's custom footprints into niles_satellite.pretty.

Dimensions are from circuit/research-notes.md (datasheet-backed). Print each
footprint 1:1 and lay the real part on it before ordering boards.

Run:  python footprints.py
"""
from pathlib import Path

OUT = Path(__file__).parent / "niles_satellite.pretty"


def fp(name, descr, body, tags="", attr="smd", jumpers=False):
    return (f'(footprint "{name}"\n  (version 20241229)\n  (generator "niles_footprints")\n'
            f'  (layer "F.Cu")\n  (descr "{descr}")\n  (tags "{tags}")\n  (attr {attr})\n'
            + ('  (duplicate_pad_numbers_are_jumpers yes)\n' if jumpers else '') +
            f'  (property "Reference" "REF**" (at 0 -2) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))\n'
            f'  (property "Value" "{name}" (at 0 2) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))\n'
            f'{body})\n')


def rect(layer, x0, y0, x1, y1, w=0.1):
    return f'  (fp_rect (start {x0:.3f} {y0:.3f}) (end {x1:.3f} {y1:.3f}) (stroke (width {w}) (type solid)) (fill no) (layer "{layer}"))\n'


def line(layer, x0, y0, x1, y1, w=0.12):
    return f'  (fp_line (start {x0:.3f} {y0:.3f}) (end {x1:.3f} {y1:.3f}) (stroke (width {w}) (type solid)) (layer "{layer}"))\n'


def text(layer, s, x, y, size=0.8):
    return f'  (fp_text user "{s}" (at {x:.3f} {y:.3f}) (layer "{layer}") (effects (font (size {size} {size}) (thickness 0.12))))\n'


def tht(num, x, y, pad, drill, shape="circle"):
    return (f'  (pad "{num}" thru_hole {shape} (at {x:.3f} {y:.3f}) (size {pad} {pad}) (drill {drill}) '
            f'(layers "*.Cu" "*.Mask"))\n')


def npth(x, y, d):
    return f'  (pad "" np_thru_hole circle (at {x:.3f} {y:.3f}) (size {d} {d}) (drill {d}) (layers "*.Cu" "*.Mask"))\n'


def smd(num, x, y, w, h):
    return (f'  (pad "{num}" smd rect (at {x:.3f} {y:.3f}) (size {w} {h}) '
            f'(layers "F.Cu" "F.Paste" "F.Mask"))\n')


def keepout(pts):
    """Rule area on all copper layers: no vias (Bel: "clear of any via hole or component pad"; other parts are kept out by the courtyard, the jack's own pads sit at the edges)."""
    xy = " ".join(f"(xy {x:.3f} {y:.3f})" for x, y in pts)
    return ('  (zone (net 0) (net_name "") (layers "*.Cu") (name "Bel keep-out") (hatch edge 0.5)\n'
            '    (connect_pads (clearance 0)) (min_thickness 0.25)\n'
            '    (keepout (tracks allowed) (vias not_allowed) (pads allowed) (copperpour allowed) (footprints allowed))\n'
            '    (fill (thermal_gap 0.5) (thermal_bridge_width 0.5))\n'
            f'    (polygon (pts {xy})))\n')


def bel_magjack():
    """Bel 0826-1X1T-GJ-F: 10/100 magjack, PoE+ (802.3at). Bel drawing dr-mag-0826-1x1t-gj-f.

    Origin midway between the locating posts. -Y is the jack's front (the RJ45 opening).
    """
    b = ""
    for x in (-6.35, 6.35):
        b += npth(x, 0.0, 3.25)
    for x in (-8.065, 8.065):
        b += tht("SH", x, -3.17, 2.3, 1.57)
    # Centre-tap pins: 13/15 and 14/16 are only 1.53 mm apart, so small pads.
    for num, x, y in ((15, -5.59, 6.60), (16, 5.59, 6.60), (13, -5.59, 8.13), (14, 5.59, 8.13)):
        b += tht(num, x, y, 1.25, 0.89)
    for num, x, y in ((4, 1.02, 15.24), (5, 3.05, 15.24), (6, 5.08, 15.24),
                      (10, 2.03, 17.78), (11, 4.06, 17.78), (12, 6.10, 17.78)):
        b += tht(num, x, y, 1.4, 0.89, "rect" if num == 4 else "circle")
    for num, x in ((17, -6.10), (18, -3.56), (19, 3.56), (20, 6.10)):
        b += tht(num, x, 20.32, 1.6, 1.02)
    w = 17.83 / 2
    b += rect("F.Fab", -w, -10.92, w, 21.85)
    b += rect("F.CrtYd", -w - 0.25, -11.17, w + 0.25, 22.10, 0.05)
    b += rect("F.SilkS", -w - 0.12, -11.04, w + 0.12, 21.97, 0.12)
    b += text("F.Fab", "FRONT (RJ45 opening)", 0, -9.5)
    # Bel sheet 4: shaded areas clear of any via or pad. Front: a U (full-width bar
    # 3.05 deep, legs |x| 6.0..8.92 down to 6.91 from the front). Rear: two strips.
    b += keepout([(-8.92, -10.92), (8.92, -10.92), (8.92, -4.01), (6.0, -4.01), (6.0, -7.87),
                  (-6.0, -7.87), (-6.0, -4.01), (-8.92, -4.01)])
    for sx in (-1, 1):
        b += keepout([(sx * 6.555, 13.60), (sx * 8.915, 13.60), (sx * 8.915, 20.07), (sx * 6.555, 20.07)])
    return fp("Bel_0826-1X1T-GJ-F", "Bel 0826-1X1T-GJ-F RJ45 magjack 10/100 PoE+ (802.3at), tab up; "
              "https://www.belfuse.com/resources/drawings/magneticsolutions/dr-mag-0826-1x1t-gj-f.pdf", b,
              "RJ45 magjack PoE", attr="through_hole",
              jumpers=True)     # the two "SH" tabs are one shell: no track between them


def sk6812_side():
    """OPSCO SK6812SIDE-A (4020 side-view). Light leaves toward +Y, away from the tabs."""
    b = ""
    b += smd(1, -1.575, 0.0, 0.55, 1.20) + smd(1, -2.075, -0.075, 0.45, 0.55)
    b += smd(2, -0.45, 0.0, 0.70, 1.20)
    b += smd(3, 0.575, 0.0, 0.45, 1.20)
    b += smd(4, 1.575, 0.0, 0.55, 1.20) + smd(4, 2.075, -0.075, 0.45, 0.55)
    b += rect("F.Fab", -2.0, -0.2, 2.0, 1.4)
    b += line("F.Fab", -0.6, 1.4, 0.0, 2.0, 0.1) + line("F.Fab", 0.6, 1.4, 0.0, 2.0, 0.1)
    b += text("F.Fab", "light", 0, 2.6, 0.5)
    b += rect("F.CrtYd", -2.55, -0.85, 2.55, 1.65, 0.05)
    b += line("F.SilkS", -2.5, 0.85, -2.5, 1.5) + line("F.SilkS", 2.5, 0.85, 2.5, 1.5)
    b += text("F.SilkS", "1", -2.0, -1.2, 0.6)
    return fp("LED_SK6812SIDE-A_4020", "OPSCO SK6812SIDE-A side-emitting addressable RGB LED 4020; "
              "light toward +Y; LCSC C5378721", b, "LED RGB SK6812 side 4020")


def touch_pad():
    """Capacitive touch electrode for the AT42QT1070, under the cover glass / top cap. Copper only:
    solder mask over it (no mask opening, no paste), out of the BOM and the position file."""
    b = ('  (pad "1" smd circle (at 0 0) (size 12 12) (layers "F.Cu"))\n'
         '  (fp_circle (center 0 0) (end 6.25 0) (stroke (width 0.05) (type solid)) (fill no) (layer "F.CrtYd"))\n'
         '  (fp_circle (center 0 0) (end 6 0) (stroke (width 0.1) (type solid)) (fill no) (layer "F.Fab"))\n')
    b += text("F.Fab", "touch", 0, 0, 0.8)
    return fp("TouchPad_D12mm", "Capacitive touch electrode, 12 mm, under the solder mask", b,
              "touch electrode capacitive", attr="smd exclude_from_pos_files exclude_from_bom")


if __name__ == "__main__":
    OUT.mkdir(exist_ok=True)
    for name, gen in (("Bel_0826-1X1T-GJ-F", bel_magjack), ("LED_SK6812SIDE-A_4020", sk6812_side),
                      ("TouchPad_D12mm", touch_pad)):
        (OUT / f"{name}.kicad_mod").write_text(gen(), encoding="utf-8")
        print("wrote", name)
