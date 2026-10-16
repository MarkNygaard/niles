"""Niles satellite v2: LED ring board (the light ring under the white band).

A ring round the mic array with 24 side-emitting LEDs facing the white band. One 10-pin
JST SH cable to the mainboard (same, mirror-symmetric pin order at both ends) and one 6-pin JST SH
to the touch disc (touchdisc.py: touch keys and light sensor, flat under the cover).

Run:  python topboard.py   ->  topboard.net
"""
from skidl import generate_netlist, ERC

from parts import Net, part, R, C, LIB, C0402, C0805, C1206

GND, V5, V3 = Net("GND"), Net("+5V"), Net("+3V3")
LED_DATA, BTN_ADC, SDA, SCL = Net("LED_DATA_5V"), Net("BTN_ADC"), Net("I2C_SDA"), Net("I2C_SCL")

J1 = part("J1", "Conn_Mainboard", "Connector_JST:JST_SH_BM10B-SRSS-TB_1x10-1MP_P1.00mm_Vertical",
          [(1, "GND"), (2, "+5V"), (3, "SDA"), (4, "LED_DATA"), (5, "+3V3"), (6, "+3V3_2"),
           (7, "BTN_ADC"), (8, "SCL"), (9, "+5V_2"), (10, "GND_2"), ("MP", "MP")],
          mpn="JST BM10B-SRSS-TB(LF)(SN)", lcsc="C160396")
# Same mirror-symmetric pinout as the mainboard's J5: a reversed cable is harmless.
V5 += J1["+5V"], J1["+5V_2"]
GND += J1["GND"], J1["GND_2"], J1["MP"]
V3 += J1["+3V3"], J1["+3V3_2"]
LED_DATA += J1["LED_DATA"]
BTN_ADC += J1["BTN_ADC"]
SDA += J1["SDA"]
SCL += J1["SCL"]

# LED supply: up to ~0.9 A at full white (firmware keeps it far lower).
C("C30", "47uF 10V", V5, GND, fp=C1206)
C("C31", "22uF 25V", V5, GND, fp=C0805)
C("C32", "22uF 25V", V5, GND, fp=C0805)

# 24 x SK6812SIDE-A in a chain round the ring.
din = Net("LED_DIN_1")
R("R10", "33R", LED_DATA, din)
for k in range(1, 25):
    d = part(f"D{k}", "SK6812SIDE-A", f"{LIB}:LED_SK6812SIDE-A_4020",
             [(1, "DIN"), (2, "VDD"), (3, "DOUT"), (4, "GND")], mpn="OPSCO SK6812SIDE-A", lcsc="C5378721")
    din += d["DIN"]
    V5 += d["VDD"]
    GND += d["GND"]
    C(f"C{k}", "100nF", V5, GND, fp=C0402)
    nxt = Net(f"LED_DIN_{k + 1}") if k < 24 else Net("LED_DOUT_END")
    nxt += d["DOUT"]
    din = nxt

# The touch keys and the light sensor are on the touch disc (touchdisc.py), flat under the cover; this
# ring stays level with the white band. One 6-pin JST-SH to the disc, straight-through, same order at both
# ends: 1 GND, 2 +5V, 3 +3V3, 4 SDA, 5 SCL, 6 TOUCH_CHANGE (the disc's AT42QT1070 interrupt, open drain).
J2 = part("J2", "Conn_TouchDisc", "Connector_JST:JST_SH_BM06B-SRSS-TB_1x06-1MP_P1.00mm_Vertical",
          [(1, "GND"), (2, "+5V"), (3, "+3V3"), (4, "SDA"), (5, "SCL"), (6, "CHANGE"), ("MP", "MP")],
          mpn="JST BM06B-SRSS-TB(LF)(SN)", lcsc="C160392")
GND += J2["GND"], J2["MP"]
V5 += J2["+5V"]
V3 += J2["+3V3"]
SDA += J2["SDA"]
SCL += J2["SCL"]
change = Net("TOUCH_CHANGE")
change += J2["CHANGE"]
# 1 k in series to the mainboard (J1 pin 7, IO1 there): a reversed cable puts the 5 V LED data on it
R("R1", "1k", change, BTN_ADC)
C("C33", "100nF", V3, GND)                # +3V3 for the disc, at J2

# Outline r 38-52.8 (Fusion "Niles v2": board z 118.4-120, top flush with the bay's top edge), the 24 LEDs at the
# outer edge with their light face at r 52.8. It sits on three printed pads: plain M2 holes at r 49.5, at 10, 130
# and 250 degrees (from +x, +y = front), M2 x 6 self-tapping screws.
for k in range(3):
    part(f"H{k + 1}", "Ring_mount_M2", "MountingHole:MountingHole_2.2mm_M2", [])

if __name__ == "__main__":
    ERC()
    generate_netlist(file_="topboard.net")
