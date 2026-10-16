"""Small helpers for writing the satellite circuits in SKiDL.

Every part is defined here in code (no KiCad symbol libraries), with an
explicit footprint and pin numbers taken from its datasheet. Each part's
`tag` is its reference, so KiCad keeps placements when the netlist is
imported again after a change.
"""
import os

os.environ.setdefault("KICAD10_SYMBOL_DIR", r"C:\Program Files\KiCad\10.0\share\kicad\symbols")

from skidl import Part, Pin, Net, SKIDL  # noqa: E402

LIB = "niles_satellite"  # project footprint library for the custom footprints


def part(ref, value, footprint, pins, *, mpn="", lcsc="", dnp=False, desc=""):
    """A part with pins given as [(number, name), ...]."""
    fields = {"MPN": mpn, "LCSC": lcsc, "Description": desc}
    if dnp:
        fields["DNP"] = "DNP"
    p = Part(
        tool=SKIDL,
        name=value,
        ref=ref,
        ref_prefix="".join(c for c in ref if c.isalpha()),
        footprint=footprint,
        value=value,
        tag=ref,
        pins=[Pin(num=str(n), name=str(name)) for n, name in pins],
    )
    # Fields go into the netlist and on to KiCad's BOM (what PCBWay orders from).
    for k, v in fields.items():
        if v:
            p.fields[k] = v
    return p


# Orderable resistors and capacitors by (value, package): (MPN, LCSC). Checked 2026-10-06 against
# LCSC and DigiKey (active parts, LCSC stock preferred). Unrated caps sit on rails of 5 V or less.
# JLCPCB basic-library parts where an exact equivalent exists (same value, package, tolerance,
# voltage, dielectric): no extended-part fee. Precision (0.1 %) and odd E96 values stay as they are.
PASSIVES = {
    ("0R", "R_0402"): ("UNI-ROYAL 0402WGF0000TCE", "C17168"),
    ("10R 1%", "R_0402"): ("UNI-ROYAL 0402WGF100JTCE", "C25077"),
    ("33R", "R_0402"): ("UNI-ROYAL 0402WGF330JTCE", "C25105"),
    ("220R", "R_0402"): ("UNI-ROYAL 0402WGF2200TCE", "C25091"),
    ("22R", "R_0402"): ("UNI-ROYAL 0402WGF220JTCE", "C25092"),
    ("49.9R 1%", "R_0402"): ("UNI-ROYAL 0402WGF499JTCE", "C25120"),
    ("330R", "R_0402"): ("UNI-ROYAL 0402WGF3300TCE", "C25104"),
    ("499R", "R_0402"): ("Yageo RC0402FR-07499RL", "C137970"),
    ("1k", "R_0402"): ("UNI-ROYAL 0402WGF1001TCE", "C11702"),
    ("5.10k 1%", "R_0402"): ("UNI-ROYAL 0402WGF5101TCE", "C25905"),
    ("10k", "R_0402"): ("UNI-ROYAL 0402WGF1002TCE", "C25744"),
    ("10k 1%", "R_0402"): ("UNI-ROYAL 0402WGF1002TCE", "C25744"),
    ("10k 0.1%", "R_0402"): ("Yageo RT0402BRD0710KL", "C190095"),
    ("10.2k 1%", "R_0402"): ("Yageo RC0402FR-0710K2L", "C138069"),
    ("12.1k 1%", "R_0402"): ("Yageo RC0402FR-0712K1L", "C132157"),
    ("12.4k 1%", "R_0402"): ("Yageo RC0402FR-0712K4L", "C138060"),
    ("22k 1%", "R_0402"): ("UNI-ROYAL 0402WGF2202TCE", "C25768"),
    ("22.1k 1%", "R_0402"): ("Yageo RC0402FR-0722K1L", "C163478"),
    ("23.2k 1%", "R_0402"): ("UNI-ROYAL 0402WGF2322TCE", "C25873"),
    ("24.0k 1%", "R_0402"): ("Yageo RC0402FR-0724KL", "C138026"),
    ("53.6k 1%", "R_0402"): ("Yageo RC0402FR-0753K6L", "C273330"),
    ("75k 1%", "R_0402"): ("Yageo RC0402FR-0775KL", "C137935"),
    ("75.0k 1%", "R_0402"): ("Yageo RC0402FR-0775KL", "C137935"),
    ("100k", "R_0402"): ("UNI-ROYAL 0402WGF1003TCE", "C25741"),
    ("100k 1%", "R_0402"): ("UNI-ROYAL 0402WGF1003TCE", "C25741"),
    ("120k 0.1%", "R_0402"): ("Yageo RT0402BRD07120KL", "C852501"),
    ("1M", "R_0402"): ("UNI-ROYAL 0402WGF1004TCE", "C26083"),
    ("4.7k", "R_0402"): ("UNI-ROYAL 0402WGF4701TCE", "C25900"),
    ("4.7k 1%", "R_0402"): ("UNI-ROYAL 0402WGF4701TCE", "C25900"),
    ("2.2k 1%", "R_0402"): ("UNI-ROYAL 0402WGF2201TCE", "C25879"),
    ("47uF 10V", "C_1206"): ("Samsung CL31A476MPHNNNE", "C96123"),
    ("47k", "R_0402"): ("UNI-ROYAL 0402WGF4702TCE", "C25792"),
    ("24.3k 1%", "R_0402"): ("Yageo RC0402FR-0724K3L", "C276264"),
    ("30.1k 1%", "R_0402"): ("Yageo RC0402FR-0730K1L", "C138009"),
    ("0R", "R_0603"): ("UNI-ROYAL 0603WAF0000T5E", "C21189"),
    ("1M", "R_1206"): ("UNI-ROYAL 1206W4F1004T5E", "C17927"),   # 200 V
    ("1M", "R_0603"): ("Yageo RC0603FR-071ML", "C105578"),
    ("27pF C0G", "C_0402"): ("Samsung CL05C270JB5NNNC", "C86287"),
    ("330pF C0G 50V", "C_0402"): ("Murata GRM1555C1H331JA01D", "C76965"),
    ("6.8nF", "C_0402"): ("Samsung CL05B682KB5NNNC", "C318580"),
    ("10nF", "C_0402"): ("Samsung CL05B103KB5NNNC", "C15195"),
    ("10nF 50V", "C_0402"): ("Samsung CL05B103KB5NNNC", "C15195"),
    ("22nF", "C_0402"): ("FH 0402B223K500NT", "C1532"),
    ("100nF", "C_0402"): ("Samsung CL05B104KO5NNNC", "C1525"),
    ("100nF 16V", "C_0402"): ("Samsung CL05B104KO5NNNC", "C1525"),
    ("1uF", "C_0402"): ("Samsung CL05A105KA5NQNC", "C52923"),
    ("1uF 50V", "C_0603"): ("Taiyo Yuden UMK107AB7105KA-T", "C105174"),
    ("4.7uF", "C_0603"): ("Samsung CL10A475KO8NNNC", "C19666"),
    ("4.7uF 10V", "C_0603"): ("Samsung CL10A475KO8NNNC", "C19666"),
    ("4.7uF 35V", "C_0603"): ("Murata GRM188R6YA475KE15D", "C162274"),
    ("4.7uF 50V", "C_0805"): ("Murata GRM21BC71H475KE11L", "C162320"),   # X7S: less DC-bias loss than X5R
    ("10uF 6.3V", "C_0603"): ("Murata GRM188R60J106ME47D", "C77041"),
    ("10nF 100V", "C_0805"): ("Samsung CL21B103KCANNNC", "C18741"),
    ("10uF 10V", "C_0805"): ("Samsung CL21B106KPQNNNE", "C32635"),
    ("10uF 10V X7R", "C_0805"): ("Samsung CL21B106KPQNNNE", "C32635"),
    ("10uF 25V", "C_0805"): ("Samsung CL21A106KAYNNNE", "C15850"),    # basic; was a 16 V extended part
    ("22uF 25V", "C_0805"): ("Samsung CL21A226MAQNNNE", "C45783"),    # basic; was a 10 V extended part
    ("10uF 25V", "C_1206"): ("Murata GRM31CR71E106KA12L", "C77093"),
    ("1nF 2kV", "C_1206"): ("Yageo CC1206KKX7RDBB102", "C23631"),
    ("100nF 50V", "C_0402"): ("Samsung CL05B104KB54PNC", "C307331"),
    ("100nF 50V", "C_0603"): ("Yageo CC0603KRX7R9BB104", "C14663"),
    ("10uF 50V", "C_1206"): ("Samsung CL31B106KBHNNNE", "C89632"),
    ("22uF 25V", "C_1206"): ("Samsung CL31A226KAHNNNE", "C12891"),
}


def two(ref, value, footprint, a, b, **kw):
    """A two-terminal part between nets a and b (pin 1 on a)."""
    known = PASSIVES.get((value, footprint.split(":")[-1][:6]))
    if known and not kw.get("dnp"):
        kw.setdefault("mpn", known[0])
        kw.setdefault("lcsc", known[1])
    p = part(ref, value, footprint, [(1, "1"), (2, "2")], **kw)
    a += p["1"]
    b += p["2"]
    return p


R0402 = "Resistor_SMD:R_0402_1005Metric"
R0603 = "Resistor_SMD:R_0603_1608Metric"
R1206 = "Resistor_SMD:R_1206_3216Metric"
C0402 = "Capacitor_SMD:C_0402_1005Metric"
C0603 = "Capacitor_SMD:C_0603_1608Metric"
C0805 = "Capacitor_SMD:C_0805_2012Metric"
C1206 = "Capacitor_SMD:C_1206_3216Metric"
LED0603 = "LED_SMD:LED_0603_1608Metric"
TP = "TestPoint:TestPoint_Pad_D1.0mm"
SJ_BRIDGED = "Jumper:SolderJumper-2_P1.3mm_Bridged_RoundedPad1.0x1.5mm"


def R(ref, value, a, b, fp=R0402, **kw):
    return two(ref, value, fp, a, b, **kw)


def C(ref, value, a, b, fp=C0402, **kw):
    return two(ref, value, fp, a, b, **kw)


LEDS = {"green": ("Lite-On LTST-C191KGKT", "C125098")}   # 571 nm, Vf 2.0 V


def led(ref, a_net, k_net, color="green"):
    mpn, lcsc = LEDS.get(color, ("", ""))
    p = part(ref, f"LED_{color}", LED0603, [(1, "K"), (2, "A")], mpn=mpn, lcsc=lcsc)
    a_net += p["A"]
    k_net += p["K"]
    return p


def testpoint(ref, net):
    p = part(ref, "TestPoint", TP, [(1, "1")])
    net += p["1"]
    return p


def jumper(ref, a, b):
    """Solder jumper bridged by default: cut it to isolate a section."""
    return two(ref, "SolderJumper_bridged", SJ_BRIDGED, a, b)


__all__ = ["Net", "part", "two", "R", "C", "led", "testpoint", "jumper", "LIB",
           "R0402", "R0603", "R1206", "C0402", "C0603", "C0805", "C1206", "LED0603", "TP"]
