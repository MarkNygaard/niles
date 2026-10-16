"""USB-C input: GCT USB4105 connector, ESD, and the TI TPS25730D PD sink.

Strapped for a 9-12 V contract: 12 V if the charger has it, else 9 V, never above.
A plain 5 V (or 5 V-only) source still gets passed through at 5 V (AlwaysEnableSink).
Values: TPS25730 datasheet SLVSGP9 and TI's EVM (SLVUCP4A). See research-notes.md.

Layout: DRAIN (pins 15, 30, EP 40) connects only to itself - no pours, no GND.
"""
from parts import Net, part, R, C, C0603, C0805, C1206


def build(*, GND, VUSB, V5, V3, USB_DP, USB_DN, CAP_MIS):
    vbus = Net("VBUS_RAW")
    cc1, cc2 = Net("USB_CC1"), Net("USB_CC2")

    J2 = part("J2", "USB_C_Receptacle", "Connector_USB:USB_C_Receptacle_GCT_USB4105-xx-A_16P_TopMnt_Horizontal",
              [("A1", "GND"), ("A4", "VBUS"), ("A5", "CC1"), ("A6", "D+"), ("A7", "D-"), ("A8", "SBU1"),
               ("A9", "VBUS_2"), ("A12", "GND_2"), ("B1", "GND_3"), ("B4", "VBUS_3"), ("B5", "CC2"),
               ("B6", "D+_2"), ("B7", "D-_2"), ("B8", "SBU2"), ("B9", "VBUS_4"), ("B12", "GND_4"), ("SH", "SH")],
              mpn="GCT USB4105-GF-A", lcsc="C3020560")
    vbus += J2["VBUS"], J2["VBUS_2"], J2["VBUS_3"], J2["VBUS_4"]
    GND += J2["GND"], J2["GND_2"], J2["GND_3"], J2["GND_4"], J2["SH"]
    cc1 += J2["CC1"]
    cc2 += J2["CC2"]
    USB_DP += J2["D+"], J2["D+_2"]
    USB_DN += J2["D-"], J2["D-_2"]

    # ESD on the data lines (flow-through: pins 1/6 and 3/4 are the same line).
    esd = part("U10", "USBLC6-2SC6", "Package_TO_SOT_SMD:SOT-23-6",
               [(1, "IO1"), (2, "GND"), (3, "IO2"), (4, "IO2_b"), (5, "VBUS"), (6, "IO1_b")],
               mpn="USBLC6-2SC6", lcsc="C7519")
    USB_DP += esd["IO1"], esd["IO1_b"]
    USB_DN += esd["IO2"], esd["IO2_b"]
    GND += esd["GND"]
    V5 += esd["VBUS"]        # its reference: our 5 V rail, never the 12 V VBUS
    C("C118", "100nF", V5, GND)    # at U10.5: the 5 V rail reaches it only after a long branch

    # VBUS surge protection: TI's flat-clamp TVS (22 V standoff, clamps under the PD
    # chip's limit, unlike an SMAJ), and a Schottky against ground surges on unplug (TI section 8.5).
    tvs = part("D10", "TVS2200DRVR", "Package_SON:WSON-6-1EP_2x2mm_P0.65mm_EP1x1.6mm",
               [(1, "GND"), (2, "GND_2"), (3, "GND_3"), (4, "IN"), (5, "IN_2"), (6, "IN_3"), (7, "EP")],
               mpn="TI TVS2200DRVR", lcsc="C523793", desc="VBUS flat-clamp TVS, 22 V")
    vbus += tvs["IN"], tvs["IN_2"], tvs["IN_3"]
    GND += tvs["GND"], tvs["GND_2"], tvs["GND_3"], tvs["EP"]
    sch = part("D11", "SS34", "Diode_SMD:D_SMA", [(1, "K"), (2, "A")], mpn="SS34", lcsc="C8678",
               desc="VBUS-GND Schottky, 40 V 3 A")
    vbus += sch["K"]
    GND += sch["A"]

    U11 = part("U11", "TPS25730DREFR", "Package_DFN_QFN:Texas_REF0038A_WQFN-38-2EP_6x4mm_P0.4",
               [(1, "LDO_3V3"), (2, "ADCIN1"), (3, "ADCIN2"), (4, "LDO_1V5"), (5, "ADCIN3"), (6, "CAP_MIS"),
                (7, "ADCIN4"), (8, "I2Ct_SDA"), (9, "I2Ct_SCL"), (10, "DBG_ACC"), (11, "GND"), (12, "GND_2"),
                (13, "PLUG_FLIP"), (14, "GND_3"), (15, "DRAIN"), (16, "GND_4"), (17, "GND_5"),
                (18, "FAULT_IN"), (19, "SINK_EN"), (20, "PPHV"), (23, "VBUS_IN"), (26, "RSVD1"),
                (27, "RSVD2"), (28, "CC1"), (29, "CC2"), (30, "DRAIN_2"), (31, "GND_6"), (32, "VBUS"),
                (34, "GND_7"), (36, "RSVD3"), (37, "PLUG_EVENT"), (38, "VIN_3V3"), (39, "GND_EP"),
                (40, "DRAIN_EP")],
               mpn="TPS25730DREFR", lcsc="C22438973")
    for p in ("GND", "GND_2", "GND_3", "GND_4", "GND_5", "GND_6", "GND_7", "GND_EP",
              "I2Ct_SDA", "I2Ct_SCL"):      # I2C target unused: tied to GND (datasheet)
        GND += U11[p]
    drain = Net("PD_DRAIN")                 # back-to-back FET drains: connect to nothing else
    drain += U11["DRAIN"], U11["DRAIN_2"], U11["DRAIN_EP"]
    vbus += U11["VBUS_IN"], U11["VBUS"]
    VUSB += U11["PPHV"]
    cc1 += U11["CC1"]
    cc2 += U11["CC2"]
    C("C90", "330pF C0G 50V", cc1, GND)              # right at the pins, same side, no via
    C("C91", "330pF C0G 50V", cc2, GND)
    # No 5.1 k Rd resistors: Rd is inside the TPS25730.

    ldo3, ldo15, vin33 = Net("PD_LDO_3V3"), Net("PD_LDO_1V5"), Net("PD_VIN_3V3")
    ldo3 += U11["LDO_3V3"]
    ldo15 += U11["LDO_1V5"]
    vin33 += U11["VIN_3V3"]
    C("C92", "10uF 10V X7R", ldo3, GND, fp=C0805)    # TI minimums are after DC bias
    C("C93", "10uF 10V X7R", ldo15, GND, fp=C0805)   # TI EVM C10
    C("C94", "10uF 6.3V", vin33, GND, fp=C0603)   # VIN_3V3 is otherwise unconnected (as on TI's EVM)

    # Strapping (100 k up from LDO_3V3, TI EVM ladder values down):
    #   ADCIN1 code 1 = min 9 V; ADCIN2 code 3 = max 12 V; ADCIN3/4 codes 2/1 = 2 A operating, 3 A max.
    for k, (pin, r_down) in enumerate((("ADCIN1", "5.10k 1%"), ("ADCIN2", "23.2k 1%"),
                                       ("ADCIN3", "12.1k 1%"), ("ADCIN4", "5.10k 1%"))):
        n = Net(f"PD_{pin}")
        n += U11[pin]
        R(f"R{90 + 2 * k}", "100k 1%", ldo3, n)
        R(f"R{91 + 2 * k}", r_down, n, GND)

    fault = Net("PD_FAULT_IN")
    fault += U11["FAULT_IN"]
    R("R98", "10k", ldo3, fault)             # must be high for normal operation
    for k, p in enumerate(("RSVD1", "RSVD2", "RSVD3")):
        n = Net(f"PD_{p}")
        n += U11[p]
        R(f"R{99 + k}", "10k", n, GND)

    # Capability mismatch (toggles on a 5 V or weak charger): to the ESP32.
    CAP_MIS += U11["CAP_MIS"]
    R("R102", "10k", CAP_MIS, V3)

    # Input and output capacitance. The 100 uF sink limit covers everything on the
    # USB path after a contract (TPS2121 is then a plain switch): VUSB + VIN + buck inputs.
    # CVBUS: 1 / 4.7 / 10 uF min / nom / max (SLVSGP9 6.4, the rendered table; its text extract runs the
    # CLDO_1V5 row's 4.5-12 uF into it). C95, 4.7 uF 50 V X7S, keeps about 3 uF at 12 V. C119 is a footprint
    # for a second one at the connector (TI's note 3 wants the high side if fast sink-disable is configured);
    # fitting both would exceed 10 uF at 0 V with tolerance.
    C("C95", "4.7uF 50V", vbus, GND, fp=C0805)
    C("C119", "DNP 4.7uF 50V", vbus, GND, fp=C0805, dnp=True)
    C("C96", "10nF 50V", vbus, GND)
    C("C97", "10nF 50V", vbus, GND)
    C("C98", "10uF 50V", VUSB, GND, fp=C1206, lcsc="C89632")
    C("C99", "DNP 10uF 50V", VUSB, GND, fp=C1206, dnp=True)
    C("C100", "100nF 50V", VUSB, GND, fp=C0603, lcsc="C14663")
    return U11
