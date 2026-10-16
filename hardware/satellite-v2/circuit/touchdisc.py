"""Niles satellite v2: touch disc (touch keys + light sensor, flat under the cover).

A round two-sided board (about 52 mm) pressed against the underside of the cover glass (final
product) or the printed top cap (prototype), above the mic array and inside its four mic ports
(r 31.1). Top side, facing the cover: four 12 mm touch pads at r = 20 mm in a cross and the
LTR-303 light sensor; everything else on the bottom side. A 0.8 mm die-cut adhesive foam, with a
window over the LTR-303, bridges the sensor's height. One 6-pin JST-SH cable to the LED ring board
(straight-through, same pin order at both ends).

Run:  python touchdisc.py   ->  touchdisc.net
"""
from skidl import generate_netlist, ERC

from parts import Net, part, R, C, LIB

GND, V5, V3 = Net("GND"), Net("+5V"), Net("+3V3")
SDA, SCL, CHANGE = Net("I2C_SDA"), Net("I2C_SCL"), Net("TOUCH_CHANGE")

J1 = part("J1", "Conn_Ring", "Connector_JST:JST_SH_BM06B-SRSS-TB_1x06-1MP_P1.00mm_Vertical",
          [(1, "GND"), (2, "+5V"), (3, "+3V3"), (4, "SDA"), (5, "SCL"), (6, "CHANGE"), ("MP", "MP")],
          mpn="JST BM06B-SRSS-TB(LF)(SN)", lcsc="C160392")
GND += J1["GND"], J1["MP"]
V5 += J1["+5V"]
V3 += J1["+3V3"]
SDA += J1["SDA"]
SCL += J1["SCL"]
CHANGE += J1["CHANGE"]

# Microchip AT42QT1070 in comms mode (MODE to VSS), I2C address 0x1B (fixed), 7 channels of which 4
# are used; KEY4-KEY6 are left open and disabled in firmware. Datasheet 9596C: up to 10 mm glass /
# 5 mm plastic, 4.7-20 k in series with each electrode, 100 nF right at the pins, its own regulator
# rather than a shared logic rail, CHANGE open drain with 47 k to VDD.
V3T = Net("+3V3_TOUCH")
U1 = part("U1", "AT42QT1070-SSUR", "Package_SO:SOIC-14_3.9x8.7mm_P1.27mm",
          [(1, "VDD"), (2, "MODE"), (3, "SDA"), (4, "RESET"), (5, "CHANGE"), (6, "SCL"), (7, "KEY6"),
           (8, "KEY5"), (9, "KEY4"), (10, "KEY3"), (11, "KEY2"), (12, "KEY1"), (13, "KEY0"), (14, "VSS")],
          mpn="Microchip AT42QT1070-SSUR", lcsc="C461610")
U2 = part("U2", "TLV75533PDBVR", "Package_TO_SOT_SMD:SOT-23-5",
          [(1, "IN"), (2, "GND"), (3, "EN"), (4, "NC"), (5, "OUT")], mpn="TI TLV75533PDBVR", lcsc="C404027")
V5 += U2["IN"], U2["EN"]
GND += U2["GND"]
V3T += U2["OUT"]
C("C1", "1uF", V5, GND)
C("C2", "1uF", V3T, GND)
C("C3", "100nF", V3T, GND)                # at U1 pins 1/14
V3T += U1["VDD"], U1["RESET"]             # RESET tied high (datasheet 2.8.1)
GND += U1["VSS"], U1["MODE"]
SDA += U1["SDA"]
SCL += U1["SCL"]
CHANGE += U1["CHANGE"]
R("R1", "47k", CHANGE, V3T)
for k, name in enumerate(("ACTION", "MUTE", "VOL_DOWN", "VOL_UP")):
    pad = part(f"E{k + 1}", f"TOUCH_{name}", f"{LIB}:TouchPad_D12mm", [(1, "1")])
    node = Net(f"TOUCH_{name}")
    R(f"R{k + 2}", "10k", U1[f"KEY{k}"], node)
    node += pad["1"]

# Ambient light, top side, looking up through a window in the cover's back print / the top cap.
U3 = part("U3", "LTR-303ALS-01", "OptoDevice:Lite-On_LTR-303ALS-01",
          [(1, "VDD"), (2, "NC"), (3, "GND"), (4, "SCL"), (5, "INT"), (6, "SDA")],
          mpn="LTR-303ALS-01", lcsc="C364577")
V3 += U3["VDD"]
GND += U3["GND"]
SCL += U3["SCL"]
SDA += U3["SDA"]
C("C4", "1uF", V3, GND)
C("C5", "100nF", V3, GND)

# Locating holes at (+-12, +-12) mm for the cap's four 2 mm pins; the adhesive foam holds the disc (plain holes, no
# copper). Layout, as in the Fusion model: E1 ACTION at (0, 20), E2 MUTE at (0, -20), E3 VOL_DOWN at (-20, 0),
# E4 VOL_UP at (20, 0), under the cap's engraved icons; LTR-303 at the centre, under the cap's 2 mm light window.
for k in range(4):
    part(f"H{k + 1}", "Cover_post_M2", "MountingHole:MountingHole_2.2mm_M2", [])

if __name__ == "__main__":
    ERC()
    generate_netlist(file_="touchdisc.net")
