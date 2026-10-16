"""Niles satellite v2: mainboard.

Generates `mainboard.net` for KiCad (PCB editor: File > Import > Netlist).
Every value and pin number comes from research-notes.md (datasheet-backed)
and SPEC.md. Sections follow the SPEC's block diagram.

Run:  python mainboard.py
"""
from skidl import generate_netlist, ERC

from parts import (Net, part, R, C, led, testpoint, jumper, LIB,
                   R0402, R0603, R1206, C0402, C0603, C0805, C1206)
import usb_pd

# ---------------------------------------------------------------------------
# Rails
# ---------------------------------------------------------------------------
GND = Net("GND")
CGND = Net("CHASSIS")          # RJ45 shield; tied to GND through 1 nF 2 kV and 1 M
VPOE = Net("VPOE_12V")         # Ag5412 output
VUSB = Net("VUSB")             # USB-C PD sink output (5, 9 or 12 V)
VIN = Net("VIN")               # mux output, 5-12 V: amplifier and both bucks
V5_BUCK, V5 = Net("+5V_BUCK"), Net("+5V")
V3_BUCK, V3 = Net("+3V3_BUCK"), Net("+3V3")
V3A = Net("+3V3A")             # W5500 analog supply

# ---------------------------------------------------------------------------
# PoE+: Bel 0826-1X1T-GJ-F magjack -> 2x DF01S -> Silvertel Ag5412 (12 V, 30 W)
# ---------------------------------------------------------------------------
J1 = part("J1", "0826-1X1T-GJ-F", f"{LIB}:Bel_0826-1X1T-GJ-F",
          [(4, "TRD2+"), (5, "TRD2-"), (6, "TRCT2"), (10, "TRD1-"), (11, "TRD1+"), (12, "TRCT1"),
           (13, "VC12"), (14, "VC36"), (15, "VC45"), (16, "VC78"),
           (17, "LED1-"), (18, "LED1+"), (19, "LED2a"), (20, "LED2b"), ("SH", "SH")],
          mpn="Bel 0826-1X1T-GJ-F", lcsc="C6377355", desc="RJ45 magjack 10/100, PoE+ 802.3at 720 mA (DigiKey 2257875)")
CGND += J1["SH"]

POE_P, POE_N = Net("POE_VIN+"), Net("POE_VIN-")   # isolated side: never touches GND
# AC pins are interchangeable; this order lets the centre taps route without crossing.
for ref, ac_a, ac_b in (("BR1", "VC36", "VC12"), ("BR2", "VC45", "VC78")):
    br = part(ref, "DF01S", "Diode_SMD:Diode_Bridge_OnSemi_SDIP-4L",
              [(1, "+"), (2, "-"), (3, "~1"), (4, "~2")], mpn="onsemi DF01S", lcsc="C124477",   # SDIP-4 (DF-S); orient by body marking
              desc="1 A 100 V bridge for PoE centre taps")
    n_a, n_b = Net(f"POE_{ac_a}"), Net(f"POE_{ac_b}")
    n_a += J1[ac_a], br["~1"]
    n_b += J1[ac_b], br["~2"]
    POE_P += br["+"]
    POE_N += br["-"]
C("C1", "10nF 100V", POE_P, POE_N, fp=C0805)
tvs = part("D1", "SMAJ58A", "Diode_SMD:D_SMA", [(1, "K"), (2, "A")], mpn="SMAJ58A", lcsc="C10762",
           desc="PoE input TVS")
POE_P += tvs["K"]
POE_N += tvs["A"]

PS1 = part("PS1", "Ag5412", "Converter_DCDC:Converter_DCDC_Silvertel_Ag54xx",
           [(1, "VIN+"), (2, "VIN-"), (3, "AT-DET"), (4, "IC1"), (5, "IC2"), (6, "IC3"),
            (7, "-VDC"), (8, "+VDC"), (9, "ADJ"), (10, "-VDC_2")],
           mpn="Silvertel Ag5412", lcsc="C40163463", desc="PoE+ PD module, 12 V 2.5 A, 802.3at (Mouser 480-AG5412)")
POE_P += PS1["VIN+"]
POE_N += PS1["VIN-"]
GND += PS1["-VDC"], PS1["-VDC_2"]
VPOE += PS1["+VDC"]
ADJ = Net("POE_ADJ")
ADJ += PS1["ADJ"]
R("R1", "DNP >=68k trim", ADJ, VPOE, fp=R0603, dnp=True)   # 68 k gives 10.8 V; below 68 k damages the Ag5412
R("R2", "DNP trim", ADJ, GND, fp=R0603, dnp=True)
cpoe = part("C2", "560uF 16V polymer", "Capacitor_SMD:CP_Elec_8x11.9", [(1, "+"), (2, "-")],
            mpn="Panasonic 16SVPF560M", lcsc="C178349", desc="Ag5412 output cap (220-1000 uF, ESR <= 300 mOhm), 8 x 11.9 mm")
VPOE += cpoe["+"]
GND += cpoe["-"]
C("C3", "10uF 25V", VPOE, GND, fp=C1206)

# AT-DET (2-event classification: 30 W granted) through an opto to a GPIO.
POE_AT = Net("POE_AT")
U2 = part("U2", "KPC357NT", "Package_SO:SO-4_4.4x3.6mm_P2.54mm",
          [(1, "A"), (2, "K"), (3, "E"), (4, "C")], mpn="Cosmo KPC357NT0BTLD", lcsc="C136411", desc="opto for Ag5412 AT-DET")
POE_P += U2["A"]
atdet = Net("POE_ATDET")
atdet += U2["K"], PS1["AT-DET"]
GND += U2["E"]
POE_AT += U2["C"]
R("R3", "47k", POE_AT, V3)      # AT-DET LED current is only ~1.6-2 mA (27 k inside the module)

# ---------------------------------------------------------------------------
# USB-C: connector, ESD and PD sink (in usb_pd.py)
# ---------------------------------------------------------------------------
USB_DP, USB_DN = Net("USB_D+"), Net("USB_D-")
USB_CAP_MIS = Net("USB_CAP_MIS")          # to ESP32 IO16
usb_pd.build(GND=GND, VUSB=VUSB, V5=V5, V3=V3, USB_DP=USB_DP, USB_DN=USB_DN, CAP_MIS=USB_CAP_MIS)

# ---------------------------------------------------------------------------
# Power mux: TI TPS2121 (PoE first; both inputs cut off above ~13.6 V)
# ---------------------------------------------------------------------------
U4 = part("U4", "TPS2121RUXR", "Package_DFN_QFN:Texas_VQFN-HR-12_2x2.5mm_P0.5mm",
          [(1, "OUT"), (2, "IN2"), (3, "CP2"), (4, "OV2"), (5, "OV1"), (6, "PR1"),
           (7, "IN1"), (8, "OUT_2"), (9, "ST"), (10, "ILM"), (11, "SS"), (12, "GND")],
          mpn="TPS2121RUXR", lcsc="C485916")
VPOE += U4["IN1"]
VUSB += U4["IN2"]
VIN += U4["OUT"], U4["OUT_2"]
GND += U4["GND"]
OV1, OV2, PR1, CP2 = Net("MUX_OV1"), Net("MUX_OV2"), Net("MUX_PR1"), Net("MUX_CP2")
CP2 += U4["CP2"]
OV1 += U4["OV1"]
OV2 += U4["OV2"]
PR1 += U4["PR1"]
R("R4", "120k 0.1%", VPOE, OV1)        # trips 13.1-14.3 V: clear of a 12.6 V PD source
R("R5", "10k 0.1%", OV1, GND)
R("R6", "120k 0.1%", VUSB, OV2)
R("R7", "10k 0.1%", OV2, GND)
# Fast switchover (XCOMP, SLVSEA3F 10.3): PR1 from PoE vs CP2 from USB, 5 us instead of 100 us.
# PoE wins while VPOE > 0.854 x VUSB (10.8 V at a 12.6 V PD contract; PoE is 11.5-12.5 V).
# CP2 >= VREF (1.10 V max) from VUSB 4.4 V up keeps fast mode on; without USB, PR1 vs VREF keeps PoE first.
R("R8", "24.3k 1%", VPOE, PR1)
R("R9", "10k 1%", PR1, GND)
R("R14", "30.1k 1%", VUSB, CP2)
R("R15", "10k 1%", CP2, GND)
ILM, SS = Net("MUX_ILM"), Net("MUX_SS")
ILM += U4["ILM"]
SS += U4["SS"]
R("R10", "22.1k 1%", ILM, GND)
C("C4", "100nF", SS, GND)
PWR_SRC = Net("PWR_SRC")       # high: PoE (IN1), low: USB (IN2)
PWR_SRC += U4["ST"]
R("R11", "10k", PWR_SRC, V3)
C("C5", "1uF 50V", VPOE, GND, fp=C0603)
# VIN bulk is the bucks' own input caps: everything on VIN counts against the
# 100 uF USB PD sink limit (TPS2121 is a plain switch once on), so no extra bulk here.
# HF return path where the busiest B.Cu bundle crosses the In2 VIN/GND edge (by U4).
C("C110", "100nF 50V", VIN, GND)
# Local bypass at the switch's pins (TI SLVSEA3F 11/12): 1 uF each is nothing against the 100 uF PD limit.
C("C115", "1uF 50V", VUSB, GND, fp=C0603)
C("C116", "1uF 50V", VIN, GND, fp=C0603)

# Amplifier supply to the Flex's 12 V screw terminal.
J4 = part("J4", "Conn_AMP_PWR", "Connector_JST:JST_XH_B2B-XH-A_1x02_P2.50mm_Vertical",
          [(1, "VIN"), (2, "GND")], mpn="JST B2B-XH-A(LF)(SN)", lcsc="C158012", desc="to Flex 12 V terminal")
VIN += J4["VIN"]
GND += J4["GND"]

# VIN sense: 100 k / 22 k -> 14 V reads 2.52 V.
VIN_SENSE = Net("VIN_SENSE")
R("R12", "100k 1%", VIN, VIN_SENSE)
R("R13", "22k 1%", VIN_SENSE, GND)
C("C8", "100nF", VIN_SENSE, GND)


# ---------------------------------------------------------------------------
# Bucks: 2x TI TPS62933 (3.3 V 2 A, 5.0 V 3 A), 500 kHz
# ---------------------------------------------------------------------------
def buck(ref, n, out, l_value, l_mpn, l_lcsc, fb_top, fb_bot, n_cout):
    global VIN, GND
    u = part(ref, "TPS62933FDRLR", "Package_TO_SOT_SMD:SOT-583-8",
             [(1, "RT"), (2, "EN"), (3, "VIN"), (4, "GND"), (5, "SW"), (6, "BST"), (7, "SS"), (8, "FB")],
             mpn="TI TPS62933FDRLR", lcsc="C5219272")   # F = forced PWM: no light-load pulse skipping near the audio
    VIN += u["VIN"]
    GND += u["GND"]
    sw, bst, ss, fb = Net(f"{ref}_SW"), Net(f"{ref}_BST"), Net(f"{ref}_SS"), Net(f"{ref}_FB")
    sw += u["SW"]
    bst += u["BST"]
    ss += u["SS"]
    fb += u["FB"]
    # RT and EN float: 500 kHz, enabled (EN has an internal pull-up; never tie it to VIN).
    C(f"C{n}", "10uF 50V", VIN, GND, fp=C1206, lcsc="C89632")
    C(f"C{n + 1}", "10uF 50V", VIN, GND, fp=C1206, lcsc="C89632")
    C(f"C{n + 2}", "100nF 50V", VIN, GND, fp=C0402)    # 0402: pads line up with VIN/GND pins 3/4
    C(f"C{n + 3}", "100nF 16V", bst, sw, fp=C0402)
    C(f"C{n + 4}", "22nF", ss, GND)
    ind = part(f"L{ref[1:]}", l_value, "Inductor_SMD:L_Wuerth_XHMI-6060", [(1, "1"), (2, "2")], mpn=l_mpn,
               lcsc=l_lcsc)
    sw += ind["1"]
    out += ind["2"]
    R(f"R{n}", fb_top, out, fb)
    R(f"R{n + 1}", fb_bot, fb, GND)
    for k in range(n_cout):
        C(f"C{n + 5 + k}", "22uF 25V", out, GND, fp=C1206, lcsc="C12891")
    return u


buck("U5", 20, V3_BUCK, "4.7uH", "Wurth 74439346047", "C6827938", "75.0k 1%", "24.0k 1%", 3)
buck("U6", 30, V5_BUCK, "6.8uH", "Wurth 74439346068", "C2041388", "53.6k 1%", "10.2k 1%", 2)
jumper("JP1", V3_BUCK, V3)     # cut to isolate the 3.3 V buck from its loads
jumper("JP2", V5_BUCK, V5)
fb1 = part("FB1", "120R@100MHz", "Inductor_SMD:L_0603_1608Metric", [(1, "1"), (2, "2")],
           mpn="Murata BLM18PG121SN1D", lcsc="C14709")
V3 += fb1["1"]
V3A += fb1["2"]

# ---------------------------------------------------------------------------
# ESP32-S3-WROOM-1-N16R8
# ---------------------------------------------------------------------------
U1 = part("U1", "ESP32-S3-WROOM-1-N16R8", "RF_Module:ESP32-S3-WROOM-1",
          [(1, "GND"), (2, "3V3"), (3, "EN"), (4, "IO4"), (5, "IO5"), (6, "IO6"), (7, "IO7"),
           (8, "IO15"), (9, "IO16"), (10, "IO17"), (11, "IO18"), (12, "IO8"), (13, "IO19"), (14, "IO20"),
           (15, "IO3"), (16, "IO46"), (17, "IO9"), (18, "IO10"), (19, "IO11"), (20, "IO12"),
           (21, "IO13"), (22, "IO14"), (23, "IO21"), (24, "IO47"), (25, "IO48"), (26, "IO45"),
           (27, "IO0"), (28, "IO35"), (29, "IO36"), (30, "IO37"), (31, "IO38"), (32, "IO39"),
           (33, "IO40"), (34, "IO41"), (35, "IO42"), (36, "RXD0"), (37, "TXD0"), (38, "IO2"),
           (39, "IO1"), (40, "GND_2"), (41, "EPAD")],
          mpn="ESP32-S3-WROOM-1-N16R8", lcsc="C2913202")
GND += U1["GND"], U1["GND_2"], U1["EPAD"]
V3 += U1["3V3"]
C("C40", "22uF 25V", V3, GND, fp=C0805)
C("C41", "100nF", V3, GND)
EN = Net("EN")
EN += U1["EN"]
R("R40", "10k", V3, EN)
C("C42", "1uF", EN, GND)
BOOT = Net("BOOT")
BOOT += U1["IO0"]
R("R41", "10k", V3, BOOT)
for ref, net, name in (("SW1", BOOT, "BOOT"), ("SW2", EN, "RESET")):
    sw = part(ref, name, "Button_Switch_SMD:SW_SPST_TL3342", [(1, "1"), (2, "2")], mpn="E-Switch TL3342F160QG", lcsc="C2886898")
    net += sw["1"]
    GND += sw["2"]

# Signals (see SPEC section 4). IO3, IO45 and IO46 are strapping pins: left unconnected.
I2S_BCLK, I2S_LRCK, I2S_DIN, I2S_DOUT = Net("I2S_BCLK"), Net("I2S_LRCK"), Net("I2S_DIN"), Net("I2S_DOUT")
SDA, SCL = Net("I2C_SDA"), Net("I2C_SCL")
ETH_CS, ETH_MOSI, ETH_SCLK, ETH_MISO = Net("ETH_CS"), Net("ETH_MOSI"), Net("ETH_SCLK"), Net("ETH_MISO")
ETH_INT, ETH_RST = Net("ETH_INT"), Net("ETH_RST")
RADAR_RX, RADAR_TX = Net("RADAR_RX"), Net("RADAR_TX")     # RX: from the radar's TX
RADAR2_RX, RADAR2_TX = Net("RADAR2_RX"), Net("RADAR2_TX")  # UART2, the still-presence radar
LED_DATA, BTN_ADC, STATUS = Net("LED_DATA"), Net("BTN_ADC"), Net("STATUS_LED")
ESP_MCLK, ESP_X1D13 = Net("ESP_MCLK"), Net("ESP_X1D13")
UART_TX, UART_RX = Net("UART0_TX"), Net("UART0_RX")
EXP16 = USB_CAP_MIS   # IO16 reads the PD mismatch flag
for pin, net in (("IO4", I2S_BCLK), ("IO5", I2S_LRCK), ("IO6", I2S_DIN), ("IO7", I2S_DOUT),
                 ("IO15", ESP_MCLK), ("IO16", EXP16), ("IO17", RADAR_RX), ("IO18", RADAR_TX),
                 ("IO8", SDA), ("IO9", SCL),
                 ("IO10", ETH_CS), ("IO11", ETH_MOSI), ("IO12", ETH_SCLK), ("IO13", ETH_MISO),
                 ("IO14", ETH_INT), ("IO21", ETH_RST), ("IO47", RADAR2_RX), ("IO48", RADAR2_TX),
                 ("IO38", LED_DATA), ("IO39", PWR_SRC), ("IO40", POE_AT), ("IO41", STATUS),
                 ("IO42", ESP_X1D13), ("RXD0", UART_RX), ("IO2", VIN_SENSE), ("IO1", BTN_ADC)):
    net += U1[pin]
TXD0 = Net("ESP_TXD0")
TXD0 += U1["TXD0"]
R("R42", "499R", TXD0, UART_TX)
# USB (Full-Speed only on the S3): series resistors just after the ESD part (0 R fitted, 22-33 R
# possible); the space at the module's USB pins is taken by its other top-edge pins.
for ref, pin, net in (("R43", "IO19", USB_DN), ("R44", "IO20", USB_DP)):
    esp_side = Net("USB_ESP_D-" if pin == "IO19" else "USB_ESP_D+")
    esp_side += U1[pin]
    R(ref, "0R", esp_side, net)
# Status LED.
st_a = Net("STATUS_LED_A")
R("R45", "1k", STATUS, st_a)
led("D2", st_a, GND, "green")

# ---------------------------------------------------------------------------
# Ethernet: WIZnet W5500 (WIZnet reference "W5500-Ref-RJ45WithMag")
# ---------------------------------------------------------------------------
U3 = part("U3", "W5500", "Package_QFP:LQFP-48_7x7mm_P0.5mm",
          [(1, "TXN"), (2, "TXP"), (3, "AGND"), (4, "AVDD"), (5, "RXN"), (6, "RXP"), (7, "DNC"),
           (8, "AVDD_2"), (9, "AGND_2"), (10, "EXRES1"), (11, "AVDD_3"), (12, "NC1"), (13, "NC2"),
           (14, "AGND_3"), (15, "AVDD_4"), (16, "AGND_4"), (17, "AVDD_5"), (18, "VBG"), (19, "AGND_5"),
           (20, "TOCAP"), (21, "AVDD_6"), (22, "1V2O"), (23, "RSVD"), (24, "SPDLED"), (25, "LINKLED"),
           (26, "DUPLED"), (27, "ACTLED"), (28, "VDD"), (29, "GND"), (30, "XI"), (31, "XO"),
           (32, "SCSn"), (33, "SCLK"), (34, "MISO"), (35, "MOSI"), (36, "INTn"), (37, "RSTn"),
           (38, "RSVD2"), (39, "RSVD3"), (40, "RSVD4"), (41, "RSVD5"), (42, "RSVD6"),
           (43, "PMODE2"), (44, "PMODE1"), (45, "PMODE0"), (46, "NC3"), (47, "NC4"), (48, "AGND_6")],
          mpn="W5500", lcsc="C32843")
for p in ("AGND", "AGND_2", "AGND_3", "AGND_4", "AGND_5", "AGND_6", "GND", "RSVD",
          "RSVD2", "RSVD3", "RSVD4", "RSVD5", "RSVD6"):
    GND += U3[p]
for k, p in enumerate(("AVDD", "AVDD_2", "AVDD_3", "AVDD_4", "AVDD_5", "AVDD_6")):
    V3A += U3[p]
    C(f"C{50 + k}", "100nF", V3A, GND)
C("C56", "10uF 25V", V3A, GND, fp=C0805)
V3 += U3["VDD"]
C("C57", "10uF 25V", V3, GND, fp=C0805)
C("C58", "100nF", V3, GND)
exres, tocap, v12 = Net("ETH_EXRES1"), Net("ETH_TOCAP"), Net("ETH_1V2O")
exres += U3["EXRES1"]
tocap += U3["TOCAP"]
v12 += U3["1V2O"]
R("R50", "12.4k 1%", exres, GND)
C("C59", "4.7uF", tocap, GND, fp=C0603)
C("C60", "10nF", v12, GND)
# Crystal: 25 MHz, CL 18 pF; pads 1/3 crystal, 2/4 GND.
xi, xo = Net("ETH_XI"), Net("ETH_XO")
xi += U3["XI"]
xo += U3["XO"]
Y1 = part("Y1", "25MHz CL18pF", "Crystal:Crystal_SMD_3225-4Pin_3.2x2.5mm",
          [(1, "X1"), (2, "GND1"), (3, "X2"), (4, "GND2")], mpn="YXC X322525MRB4SI", lcsc="C70593")   # 25 MHz, CL 18 pF, +-10/+-20 ppm
xi += Y1["X1"]
xo += Y1["X2"]      # WIZnet's reference has a 0 R on XO; left out to keep XO short (drive level 59 uW, no limiting needed)
GND += Y1["GND1"], Y1["GND2"]
R("R52", "1M", xi, xo)
# 27 pF each + ~4 pF stray = the crystal's 18 pF load (18 pF caps would pull it ~+45 ppm).
C("C61", "27pF C0G", xi, GND)
C("C62", "27pF C0G", xo, GND)
# SPI and control.
for pin, net in (("SCSn", ETH_CS), ("SCLK", ETH_SCLK), ("MISO", ETH_MISO), ("MOSI", ETH_MOSI),
                 ("INTn", ETH_INT), ("RSTn", ETH_RST)):
    net += U3[pin]
R("R53", "10k", ETH_INT, V3)
R("R54", "10k", ETH_RST, V3)
# Line side: 0 R damping (33 R for EMI if needed), 49.9 R terminations, centre taps.
txp, txn, rxp, rxn = Net("ETH_TXP"), Net("ETH_TXN"), Net("ETH_RXP"), Net("ETH_RXN")
# P/N last so KiCad pairs them for differential routing.
txp_j, txn_j, rxp_c, rxn_c = Net("ETH_J_TXP"), Net("ETH_J_TXN"), Net("ETH_C_RXP"), Net("ETH_C_RXN")
rxp_j, rxn_j, txct, rxct = Net("ETH_J_RXP"), Net("ETH_J_RXN"), Net("ETH_TXCT"), Net("ETH_RXCT")
txp += U3["TXP"]
txn += U3["TXN"]
rxp += U3["RXP"]
rxn += U3["RXN"]
R("R55", "0R", txp, txp_j)
R("R56", "0R", txn, txn_j)
R("R57", "0R", rxp, rxp_c)
R("R58", "0R", rxn, rxn_c)
R("R59", "49.9R 1%", txp_j, V3A)
R("R60", "49.9R 1%", txn_j, V3A)
R("R61", "49.9R 1%", rxp_c, rxct)
R("R62", "49.9R 1%", rxn_c, rxct)
C("C63", "6.8nF", rxp_c, rxp_j)
C("C64", "6.8nF", rxn_c, rxn_j)
R("R63", "10R 1%", txct, V3A)
C("C65", "22nF", txct, GND)
C("C66", "10nF", rxct, GND)
txp_j += J1["TRD1+"]
txn_j += J1["TRD1-"]
txct += J1["TRCT1"]
rxp_j += J1["TRD2+"]
rxn_j += J1["TRD2-"]
rxct += J1["TRCT2"]
# Jack LEDs: yellow = link, green = activity (both active low).
link_k, act_k = Net("ETH_LINK_K"), Net("ETH_ACT_K")
V3 += J1["LED1+"], J1["LED2b"]           # 18 yellow anode, 20 green anode
link_k += J1["LED1-"]
act_k += J1["LED2a"]                      # 19 is the green cathode when 20 is the anode
lnk, act = Net("ETH_LINKLED"), Net("ETH_ACTLED")
lnk += U3["LINKLED"]
act += U3["ACTLED"]
R("R64", "330R", link_k, lnk)
R("R65", "330R", act_k, act)
C("C67", "1nF 2kV", CGND, GND, fp=C1206)
R("R66", "1M", CGND, GND, fp=R1206)     # across the shield gap: 1206 for the creepage and 200 V

# ---------------------------------------------------------------------------
# reSpeaker Flex: 2x10 socket (Flex pins 1-10 down one column, 20-11 down the other)
# ---------------------------------------------------------------------------
flex_pins = {1: "MCLK", 2: "LRCK", 3: "DATA0", 4: "GND", 5: "SDA", 6: "SCL", 7: "X1D13", 8: "GND",
             9: "5V_IN", 10: "5V_IN", 11: "VDDIO", 12: "GND", 13: "SPI_MISO", 14: "SPI_MOSI",
             15: "SPI_CLK", 16: "SPI_CS_N", 17: "GND", 18: "DATA2", 19: "DATA1", 20: "BCLK"}


def socket_pad(flex_pin):
    """Flex pin -> 2x10 socket pad.

    KiCad's socket footprint puts pad 2 to the LEFT of pad 1 (x = -2.54), while seen from
    above the Flex's pin 20 is to the RIGHT of its pin 1. So pad 1 sits under Flex pin 20:
    odd pads carry Flex 20..11, even pads Flex 1..10. Place the footprint unrotated with
    pad 1 at Flex pin 20 (Fusion x -31.18, y 11.92).
    """
    return 2 * flex_pin if flex_pin <= 10 else 41 - 2 * flex_pin


J3 = part("J3", "Flex_2x10", "Connector_PinSocket_2.54mm:PinSocket_2x10_P2.54mm_Vertical",
          [(socket_pad(p), f"F{p}_{n}") for p, n in flex_pins.items()],
          mpn="Samtec ESQ-110-33-G-D", lcsc="C3322059", desc="to reSpeaker Flex 2x10 header; 16.13 mm socket, 18.67 mm stack")
fp = {p: J3[f"F{p}_{n}"] for p, n in flex_pins.items()}
for p in (4, 8, 12, 17):
    GND += fp[p]
# The Flex may also make its own 5 V from its 12 V terminal: R17 lets the two be separated.
FLEX_5V = Net("FLEX_5V")
FLEX_5V += fp[9], fp[10]
R("R17", "0R", V5, FLEX_5V, fp=R0603)
FLEX_DATA0, FLEX_DATA1 = Net("FLEX_DATA0"), Net("FLEX_DATA1")
# The Flex drives both clocks over 115-130 mm of track: source series resistors at J3 against ringing
# (a double-clocked BCLK edge); 0 R if the edges turn out too slow.
FLEX_BCLK, FLEX_LRCK = Net("FLEX_BCLK"), Net("FLEX_LRCK")
FLEX_BCLK += fp[20]
FLEX_LRCK += fp[2]
R("R76", "22R", FLEX_BCLK, I2S_BCLK)
R("R77", "22R", FLEX_LRCK, I2S_LRCK)
FLEX_DATA0 += fp[3]
FLEX_DATA1 += fp[19]
SDA += fp[5]
SCL += fp[6]
flex_mclk, flex_x1d13, vddio = Net("FLEX_MCLK"), Net("FLEX_X1D13"), Net("FLEX_VDDIO")
flex_mclk += fp[1]
flex_x1d13 += fp[7]
vddio += fp[11]
R("R70", "DNP 0R", ESP_MCLK, flex_mclk, dnp=True)
R("R71", "DNP 0R", ESP_X1D13, flex_x1d13, dnp=True)
# Data direction (XMOS XVF3800 datasheet, device pinout): DATA0 (X1D00) is the reference
# input from the host, DATA1 (X1D22) the processed audio out to the host. So fitted:
# DATA1 -> ESP in (IO6), ESP out (IO7) -> DATA0. Swap by moving the 0 R links.
R("R72", "DNP 0R", FLEX_DATA0, I2S_DIN, dnp=True)
R("R73", "DNP 0R", FLEX_DATA1, I2S_DOUT, dnp=True)
R("R74", "0R", FLEX_DATA0, I2S_DOUT)
R("R75", "0R", FLEX_DATA1, I2S_DIN)
# Flex SPI (XVF3800 flash/boot) and DATA2 to test points for later.
for k, p in enumerate((13, 14, 15, 16, 18)):
    n = Net(f"FLEX_{flex_pins[p]}")
    n += fp[p]
    testpoint(f"TP{20 + k}", n)
testpoint("TP25", vddio)
# Flex mounting standoffs (M3, at the Flex's holes): plated, to GND.
for k in range(4):
    h = part(f"H{k + 1}", "Flex_standoff_M3", "MountingHole:MountingHole_3.2mm_M3_Pad_Via", [(1, "1")])
    GND += h["1"]

# ---------------------------------------------------------------------------
# I2C: SHT40 here, LTR-303 on the top board, XVF3800 on the Flex (4.7 k pull-ups there too)
# ---------------------------------------------------------------------------
R("R80", "4.7k", SDA, V3)      # with the Flex's 4.7 k: ~2.4 k, for the top-board cable at 400 kHz
R("R81", "4.7k", SCL, V3)
U8 = part("U8", "SHT40-AD1B", "Sensor_Humidity:Sensirion_DFN-4_1.5x1.5mm_P0.8mm_SHT4x_NoCentralPad",
          [(1, "SDA"), (2, "SCL"), (3, "VDD"), (4, "VSS")], mpn="SHT40-AD1B-R2", lcsc="C2909890")
SDA += U8["SDA"]
SCL += U8["SCL"]
V3 += U8["VDD"]
GND += U8["VSS"]
C("C80", "100nF", V3, GND)
U9 = part("U9", "SCD41", "Sensor:Sensirion_SCD4x-1EP_10.1x10.1mm_P1.25mm_EP4.8x4.8mm",
          [(6, "GND"), (7, "VDD"), (9, "SCL"), (10, "SDA"), (19, "VDD_2"), (20, "GND_2"), (21, "EP")],
          mpn="Sensirion SCD41-D-R2", lcsc="C3659294", desc="CO2 sensor, fitted from v1 (by the front-right vent)")
GND += U9["GND"], U9["GND_2"], U9["EP"]
# Own LDO from 5 V: Sensirion wants < 30 mV supply variation, which the Wi-Fi bursts on 3V3 break.
V3_CO2 = Net("+3V3_CO2")
U12 = part("U12", "TLV75533PDBVR", "Package_TO_SOT_SMD:SOT-23-5",
           [(1, "IN"), (2, "GND"), (3, "EN"), (4, "NC"), (5, "OUT")], mpn="TI TLV75533PDBVR", lcsc="C404027")
V5 += U12["IN"], U12["EN"]
GND += U12["GND"]
V3_CO2 += U12["OUT"]
C("C114", "1uF", V5, GND)
V3_CO2 += U9["VDD"], U9["VDD_2"]
SCL += U9["SCL"]
SDA += U9["SDA"]
C("C81", "4.7uF 10V", V3_CO2, GND, fp=C0603)   # Sensirion: 4.7 uF at VDD; also the LDO's output cap
C("C117", "1uF", V3_CO2, GND)      # at U12's OUT pin: C81 sits by the sensor, 9 mm away

# ---------------------------------------------------------------------------
# LED data level shift (3.3 V -> 5 V) and the top board connector
# ---------------------------------------------------------------------------
U7 = part("U7", "SN74AHCT1G125DBVR", "Package_TO_SOT_SMD:SOT-23-5",
          [(1, "OE"), (2, "A"), (3, "GND"), (4, "Y"), (5, "VCC")], mpn="TI SN74AHCT1G125DBVR", lcsc="C7484")
GND += U7["OE"], U7["GND"]
LED_DATA += U7["A"]
R("R83", "100k", LED_DATA, GND)      # IO38 floats until firmware sets it: keep the ring dark
V5 += U7["VCC"]
C("C82", "100nF", V5, GND)
led_y, LED_5V = Net("LED_DATA_Y"), Net("LED_DATA_5V")
led_y += U7["Y"]
# 220R, not 33R: a reversed top-board cable puts this output on the button ladder, where ACTION is
# 0R to GND; 220R holds the short to 23 mA (AHCT125: 25 mA) and still gives ~3 ns edges into one LED
R("R82", "220R", led_y, LED_5V)
J5 = part("J5", "Conn_TopBoard", "Connector_JST:JST_SH_BM10B-SRSS-TB_1x10-1MP_P1.00mm_Vertical",
          [(1, "GND"), (2, "+5V"), (3, "SDA"), (4, "LED_DATA"), (5, "+3V3"), (6, "+3V3_2"),
           (7, "BTN_ADC"), (8, "SCL"), (9, "+5V_2"), (10, "GND_2"), ("MP", "MP")],
          mpn="JST BM10B-SRSS-TB(LF)(SN)", lcsc="C160396")
# Mirror-symmetric pinout: a reversed cable (pin k to 11-k) only swaps SDA/SCL and
# LED/button, never 5 V onto GND or 3.3 V onto a signal.
V5 += J5["+5V"], J5["+5V_2"]
GND += J5["GND"], J5["GND_2"], J5["MP"]
V3 += J5["+3V3"], J5["+3V3_2"]
LED_5V += J5["LED_DATA"]
BTN_ADC += J5["BTN_ADC"]
SDA += J5["SDA"]
SCL += J5["SCL"]
C("C83", "100nF", BTN_ADC, GND)
R("R16", "1M", BTN_ADC, V3)     # idle high without the top board; on it, the touch chip's CHANGE (open drain) pulls low
C("C84", "22uF 25V", V5, GND, fp=C0805)

# ---------------------------------------------------------------------------
# Radar (HLK-LD2450): JST ZH 1.5 mm, order 5V, RX, TX, GND as on the module
# ---------------------------------------------------------------------------
J6 = part("J6", "Conn_Radar", "Connector_JST:JST_ZH_B4B-ZR_1x04_P1.50mm_Vertical",
          [(1, "5V"), (2, "RX"), (3, "TX"), (4, "GND")], mpn="JST B4B-ZR(LF)(SN)", lcsc="C157997")
V5 += J6["5V"]
RADAR_TX += J6["RX"]      # our TX into the module's RX
RADAR_RX += J6["TX"]
GND += J6["GND"]
C("C85", "10uF 10V", V5, GND, fp=C0805)

# Second radar (HLK-LD2410C) for still presence; the LD2450 above tracks positions. 5-pin ZH in the
# module's own hole order (TX, RX, OUT, GND, VCC), so a straight ZH-to-2.54 mm cable fits. OUT is
# unused (the UART reports the same). IO47/IO48 are in the VDD_SPI domain, 3.3 V on the N16R8.
J8 = part("J8", "Conn_Radar2", "Connector_JST:JST_ZH_B5B-ZR_1x05_P1.50mm_Vertical",
          [(1, "TX"), (2, "RX"), (3, "OUT"), (4, "GND"), (5, "5V")], mpn="JST B5B-ZR(LF)(SN)", lcsc="C157990")
RADAR2_RX += J8["TX"]     # the module's TX into our RX
RADAR2_TX += J8["RX"]
GND += J8["GND"]
V5 += J8["5V"]
C("C86", "10uF 10V", V5, GND, fp=C0805)

# ---------------------------------------------------------------------------
# Expansion / debug header and test points
# ---------------------------------------------------------------------------
# No expansion header: its pins were the debug UART (TP26/TP27 below), IO16 (in use: PD mismatch) and
# IO47/IO48 (the second radar's UART, on J8), and its spot at the left edge pulled five signals across
# the board along the back edge under J1.
for k, net in enumerate((VIN, VPOE, VUSB, V5, V3, V3A, GND, GND, I2S_BCLK, I2S_LRCK, I2S_DIN, I2S_DOUT,
                         SDA, SCL)):
    testpoint(f"TP{1 + k}", net)
testpoint("TP26", UART_TX)       # debug UART for logging, beside the ESP32 (GND at TP7/TP8)
testpoint("TP27", UART_RX)

# Fiducials for assembly.
for k in range(3):
    part(f"FID{k + 1}", "Fiducial", "Fiducial:Fiducial_1mm_Mask2mm", [])

if __name__ == "__main__":
    ERC()
    generate_netlist(file_="mainboard.net")
