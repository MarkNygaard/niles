# Research notes for the circuit (datasheet-backed)

Collected while choosing parts. Items marked UNVERIFIED still need a check.

## ESP32-S3-WROOM-1-N16R8 (footprint `RF_Module:ESP32-S3-WROOM-1`)
Sources: WROOM datasheet v1.8, ESP32-S3 datasheet v2.2, Espressif hardware design guidelines.

- **Pad → GPIO map:**
  - Pads 1–14: GND, 3V3, EN, IO4, IO5, IO6, IO7, IO15, IO16, IO17, IO18, IO8, IO19 (USB D−), IO20 (USB D+)
  - Pads 15–26: IO3, IO46, IO9, IO10, IO11, IO12, IO13, IO14, IO21, IO47, IO48, IO45
  - Pads 27–41: IO0, IO35, IO36, IO37, IO38, IO39, IO40, IO41, IO42, RXD0 (GPIO44), TXD0 (GPIO43), IO2, IO1, GND, EPAD (GND)
- **Not usable on N16R8:** IO35/36/37 (octal PSRAM).
- **Strapping pins:**
  - GPIO0: pull-up (10 k) and a BOOT button to GND
  - GPIO46: never high at reset
  - GPIO45: **must be low or floating at reset** (high selects 1.8 V flash and breaks N16R8)
  - GPIO3: ignore
- **EN:** 10 k to 3V3 and 1 µF to GND; reset button to GND with 0.1 µF across it.
- **3V3:** 22 µF + 0.1 µF at the module, and ≥10 µF at the power entry. The supply must handle ≥0.5 A.
- **USB:** footprints for 22–33 Ω series resistors on D−/D+ near the module (fit 0 Ω), plus capacitor footprints left unfitted. Add USB ESD protection.
- **Debug UART:** 499 Ω in series on U0TXD.
- **Antenna:**
  - Must sit at or past the board edge. The footprint carries a 48 × 21 mm keepout zone (all copper) beyond the antenna end and ±15 mm to the sides.
  - Keep ≥15 mm clearance inside the enclosure.
  - Keep USB, UART and crystal traces away from it.
- **Thermal vias:** the EPAD vias are 0.2 mm drill. Check this against PCBWay's process.
- **Temperature:** rated −40 to 65 °C ambient.

## W5500 (footprint `Package_QFP:LQFP-48_7x7mm_P0.5mm`; symbol `Interface_Ethernet:W5500` has INTn mistyped)
Sources: datasheet v1.1.0, WIZnet reference "W5500-Ref-RJ45WithMag".

- **Pin connections:**
  - Pins 1/2 TXN/TXP, pins 5/6 RXN/RXP
  - Pin 10 EXRES1: 12.4 k 1% to GND
  - Pin 18 VBG: float. Pin 20 TOCAP: 4.7 µF. Pin 22 1V2O: 10 nF.
  - Pin 23 RSVD: GND (the datasheet says so, though the reference schematic draws it as NC)
  - Pins 24–27 LEDs (active low): LINKLED pin 25, ACTLED pin 27
  - Pin 28 VDD, pins 30/31 XI/XO
  - Pins 32–37: SCSn, SCLK, MISO, MOSI, INTn, RSTn
  - Pins 43–45 PMODE: float (auto-negotiation)
  - Floating: pins 7, 12, 13, 18, 46, 47. Pins 38–42: NC, or GND.
- **AVDD:** pins 4, 8, 11, 15, 17, 21 on 3V3A (3V3 through a 120 Ω/100 MHz bead). 6 × 0.1 µF plus 10 µF.
- **VDD:** pin 28 on 3V3D, with 10 µF + 0.1 µF.
- **Crystal:** 25 MHz, CL 18 pF, ±30 ppm. 18 pF load caps, 1 M across XI–XO, 0 Ω in series on XO. **2026-10-06:** the 0 Ω (R51) is removed (WIZnet's reference W5500-Ref-RJ45WithMag has it as R5 0 R, a place for a drive-limiting resistor; the W5500 drives the crystal at 59 µW, so it isn't needed and leaving it out keeps XO short); XO goes straight to the crystal. Layout: Y1, C61, C62 and R52 sit together right of the W5500, XI 13.9 mm (top layer, no vias), XO 14.9 mm (3 vias), routed first and locked.
  - Footprint `Crystal_SMD_3225-4Pin_3.2x2.5mm`: pads 1/3 are the crystal, pads 2/4 are GND.
- **TX side:** TXP/TXN each 49.9 Ω 1% to 3V3A. TX centre tap through 10 Ω to 3V3A, with 22 nF to GND.
- **RX side:** RXP/RXN through 6.8 nF series caps to RD+/RD−. On the chip side, 49.9 Ω from each to a common node; that node has 10 nF to GND and connects to the RX centre tap.
- **Optional:** 0 Ω series (33 Ω for EMI) on the four lines.
- **LEDs:** anodes to 3V3, cathodes through 330 Ω to LINKLED and ACTLED.
- **Shield:** to chassis ground, with 1 nF/2 kV to GND.
- **INTn:** add a 10 k pull-up. **RSTn:** hold ≥500 µs.
- **SPI pins:** IO10 CS, IO11 MOSI, IO12 SCLK, IO13 MISO (the FSPI IO_MUX pins).
- **Layout:** void the planes under the magnetics, and keep the pairs equal length.
- **Supply current:** 132 mA at 100 M. No auto-MDIX.

## Power mux: TI TPS2121RUXR (LCSC C485916; footprint `Package_DFN_QFN:Texas_VQFN-HR-12_2x2.5mm_P0.5mm`, the plain one, not ThermalVias)
Source: SLVSEA3F.

- **Pins:**
  - 1, 8 OUT
  - 2 IN2 (USB VBUS), 7 IN1 (PoE 12 V, the priority input)
  - 3 CP2: divider from IN2 (fast-switchover / XCOMP mode, see below; the first design tied it to GND)
  - 4 OV2, 5 OV1
  - 6 PR1: divider from IN1
  - 9 ST: open drain, low means IN2 is selected. Pull up 10 k to 3V3, to a GPIO.
  - 10 ILM: to GND through a resistor. 11 SS: to GND through a cap. 12 GND.
- **Values:**
  - PR1 divider (R8/R9): 24.3 k / 10.0 k from VPOE; CP2 divider (R14/R15): 30.1 k / 10.0 k from VUSB. PoE wins while VPOE > 0.855 × VUSB, and fast switchover (5 µs) stays on from VUSB ≈ 4.4 V up (current values, 2026-10-06; checked again 2026-10-08).
  - OV1 and OV2: 120 k / 10.0 k, **0.1 %**. Trips at 13.78 V typical, 13.13–14.30 V worst case.
  - ILM: 22.1 k gives a 4.0–5.0 A limit. SS: 100 nF.
- **Switching:** changeover takes 100 µs with CP2 at GND, 5 µs in the XCOMP mode used now. Add a TVS on USB VBUS; TI shows a TVS1800, LCSC C2649846 (UNVERIFIED).

## Bucks: TI TPS62933FDRLR (SOT-583-8, LCSC C5219272, the forced-PWM F variant; footprint `Package_TO_SOT_SMD:SOT-583-8`)
Source: SLUSEA4D.

- **Pins:** 1 RT (float for 500 kHz), 2 EN (float means enabled, max 5.5 V, never tie to VIN), 3 VIN, 4 GND, 5 SW, 6 BST (100 nF to SW), 7 SS (22 nF), 8 FB (0.800 V).
- **Common to both rails:** CIN 2 × 10 µF 50 V 1206 (C89632) + 100 nF 50 V (C1711).
- **3.3 V:**
  - FB divider: 75.0 k / 24.0 k
  - Inductor: 4.7 µH Würth 74439346047 (`L_Wuerth_XHMI-6060`); LCSC alternative Sunlord MWSA0603S-4R7MT, C408447
  - COUT: 3 × 22 µF 25 V 1206 (C12891)
- **5.0 V:**
  - FB divider: 53.6 k / 10.2 k
  - Inductor: 6.8 µH Würth 74439346068 (`L_Wuerth_XHMI-6060`, LCSC C2041388, low stock)
  - COUT: 2 × 22 µF 1206
- **Duty cycle:** maximum 98 %, not 100 %. On USB at 5 V before PD, the 5 V rail sags to about 4.4–4.85 V. That is OK until PD raises VIN to 9/12 V.
- **Forced PWM:** the TPS62933F (C5219272) is the forced-PWM variant, if pulse skipping proves audible.

## Level shifter: SN74AHCT1G125DBVR (SOT-23-5, LCSC C7484)
- **Pins:** 1 OE̅ (GND), 2 A, 3 GND, 4 Y, 5 VCC (4.5–5.5 V).

## LEDs: OPSCO SK6812SIDE-A, 4020 side-emitting (LCSC C5378721; NOT C2890037, which is reverse-mount)
Sources: OPSCO spec rev B/2 2025-11, Normand spec.

- **Pins:** 1 DIN, 2 VDD, 3 DOUT, 4 GND. 3.7–5.5 V supply. VIH ≥ 0.65·VDD, so the data line needs the 5 V level shifter. GRB order, 800 kbps, reset >200 µs (use 280 µs).
- **Package:** 4.0 × 2.0 (height) × 1.6 mm. The light leaves sideways, at 90° to the pad row, on the side away from the tabs.
- **Custom footprint (pad centres, sizes, origin at the pad group centre, KiCad +Y down):**
  - Pad 1: (−1.575, 0) 0.55 × 1.20, plus a tab at (−2.075, −0.075) 0.45 × 0.55
  - Pad 2: (−0.45, 0) 0.70 × 1.20
  - Pad 3: (+0.575, 0) 0.45 × 1.20
  - Pad 4: (+1.575, 0) 0.55 × 1.20, plus a tab at (+2.075, −0.075) 0.45 × 0.55
  - The light leaves toward +Y. The emitting face is at about y = +1.4 (UNVERIFIED).
- **Per LED:** 100 nF, 36 mA at full white (the datasheet says keep under 70 % grayscale). Put 33–100 Ω on the first DIN.

## Light sensor: Lite-On LTR-303ALS-01 instead of the VEML7700 (LCSC C364577)
- **Footprint:** `OptoDevice:Lite-On_LTR-303ALS-01`; KiCad symbol `Sensor_Optical:LTR-303ALS-01`.
- **Pins:** 1 VDD, 2 NC, 3 GND, 4 SCL, 5 INT, 6 SDA. I2C 0x29, 2.4–3.6 V, 1 µF on VDD. Looks up.
- **Placement:** on the top board, looking up through a small window in the top cap. Firmware blanks the LEDs while measuring.

## SHT40-AD1B (LCSC C2909890, UNVERIFIED)
- **Footprint:** `Sensor_Humidity:Sensirion_DFN-4_1.5x1.5mm_P0.8mm_SHT4x_NoCentralPad`.
- **Pins:** 1 SDA, 2 SCL, 3 VDD, 4 VSS. I2C 0x44, 100 nF.

## Switches: C&K PTS645SM43SMTR92 LFS (160 gf, actuator top 4.3 mm; LCSC C221880)
- **Footprint:** `Button_Switch_SMD:SW_SPST_PTS645Sx43SMTR92`. Pad 1 is pins 1+2, pad 2 is pins 3+4.

## Radar: HLK-LD2450
- **Size:** 44 × 15 mm, no mounting holes, 44 mm axis vertical, antennas face forward. No metal in front of it.
- **Connector:** 1.5 mm pitch 4-pin socket, order **5V, RX, TX, GND** (cable included). Also a 2 × 4 2.0 mm header.
- **Supply:** 5 V, >200 mA (120 mA average). UART 3.3 V, 256000 8N1.
- **On the mainboard:** a JST ZH 1.5 mm 4-pin connector (verify the cable's far end).

## PoE+: Silvertel Ag5412 (12 V, 2.5 A, 30 W continuous, 802.3at Class 4; Mouser 480-AG5412)
Source: Ag5400 datasheet V1.3 2023-11. Footprint `Converter_DCDC:Converter_DCDC_Silvertel_Ag54xx`, verified.

- **Pins:**
  - 1 VIN+, 2 VIN−
  - 3 AT-DET: opto LED from VIN+, cathode to AT-DET. Low means a 2-event PSE (30 W).
  - 4, 5, 6: IC, **do not connect**
  - 7, 10 −VDC; 8 +VDC
  - 9 ADJ: **leave open** for 12.0 V (11.5–12.5 V). Fit DNP pads for trimming; 68 k to +VDC gives 10.8 V, the minimum.
- **Size:** 57.3 × 19.5 mm footprint, **14 mm tall**.
- **Isolation:** add a copper keep-out zone (rule area) on all layers from about 15.3 to 42.1 mm from pin 1.
- **Input:** two **DF01S** bridges (1 A, 100 V). One takes the data-pair centre taps, the other the spare pairs; both paralleled to VIN+/VIN−. 10 nF 100 V across VIN. SMAJ58A optional (the module has an internal TVS).
- **Output:** 470 µF (220–1000 µF) ≥16 V, ESR ≤300 mΩ. Use **polymer** below 0 °C. Put it right at the pins.
- **Thermal:** copper planes on +VDC/−VDC on both outer layers. Derates above 70 °C ambient.
- **Note:** Cisco PSEs may give only 15.4 W without LLDP. The firmware could add LLDP later.

## PoE+ magjack: Bel 0826-1X1T-GJ-F (802.3at, 720 mA, −40 to 85 °C; DigiKey 2257875, about $8.70; not at LCSC)
Rejected: Würth 7499211121A (802.3af, 350 mA), Pulse JXD1-0007NL, Bel 0826-1X1T-57-F (15 W), HanRun parts.

- **Size:** 16.31 W × 13.61 H × **32.77 mm long**. No KiCad footprint, so a custom one is needed.
- **Pins:**
  - Cable-side data pairs: 11 TRD1+, 10 TRD1−, 12 TRCT1 (PD transmit, RJ45 1/2); 4 TRD2+, 5 TRD2−, 6 TRCT2 (PD receive, 3/6)
  - Cable centre taps: 13 VC12, 14 VC36 → data bridge AC inputs; 15 VC45, 16 VC78 → spare bridge AC inputs
  - LEDs: 17 yellow −, 18 yellow +; 19/20 bicolour (19+/20− orange, 19−/20+ green)
  - Shield ×2
- **Land pattern (mm; origin midway between the posts; +y toward the rear; component-side view):**
  - Posts (NPTH Ø3.25): (±6.35, 0)
  - Shield: (±8.065, −3.17), Ø1.57
  - Pin 15 (−5.59, 6.60), pin 16 (+5.59, 6.60), pin 13 (−5.59, 8.13), pin 14 (+5.59, 8.13), Ø0.89. Pads ≤1.25 mm here (1.53 mm spacing).
  - Pins 4/5/6: (+1.02 / +3.05 / +5.08, 15.24); pins 10/11/12: (+2.03 / +4.06 / +6.10, 17.78); Ø0.89, about 1.4 mm pads
  - LEDs 17/18/19/20: (−6.10 / −3.56 / +3.56 / +6.10, 20.32), Ø1.02, about 1.6 mm pads
  - Front face at y = −10.92, rear at about +21.85. No vias or pads from y −10.92 to −4.0.
  - **Print it 1:1 and check against the drawing before fab.**
- **To the W5500:**
  - TXP (2) → 11, TXN (1) → 10. 49.9 Ω each to 3V3A. TRCT1 (12) → 10 Ω to 3V3A, plus 22 nF.
  - RXP (6) → 6.8 nF → 4, RXN (5) → 6.8 nF → 5. 49.9 Ω to a common node, which goes to TRCT2 (6) and 10 nF.
  - LEDs: yellow 18 → 3V3, 17 → 330 Ω → LINKLED. Green 20 → 3V3, 19 → 330 Ω → ACTLED.
  - Shield → chassis, with 1 nF 2 kV to GND.

## Mechanical consequence
- **Flex height:** the Ag5412 (14 mm) and the magjack (13.6 mm) are too tall for the Flex at 11 mm, so **raise the Flex to boardStack = 17 mm** (clearance under it about 15 mm). **2026-10-06:** made 18.67 mm, the nearest stock socket height (Samtec ESQ-110-33-G-D, 16.13 mm + the Flex's 2.54 mm header body).
- **ESP32:** move it to a side edge (±X), antenna pointing out, where the Flex's shadow ends at |x| = 35.3.

## USB-C PD sink: decision TI TPS25730D (hardware-strapped 9–12 V, built-in 5 A switch, OVP; LCSC C22438973)
Footprint `Package_DFN_QFN:Texas_REF0038A_WQFN-38-2EP_6x4mm_P0.4`; symbol `TPS25730D`.

- **Rejected: STUSB4500.** Its factory NVM PDOs are 5/15/20 V and it negotiates on its own, so it would put 20 V on a 14.4 V amp.
- **Rejected: AP33772S.** It is safe (starts at 5 V and waits for the host), but the FA02 is out of stock at LCSC and DigiKey, it is firmware-dependent, and it needs an I2C level shifter.
- **Rejected: CH224K/Q.** Lowest choice with I2C is 9 V, and fallback is undocumented.
- **TPS25730D:**
  - ADCIN1/2 dividers set the voltage window.
  - If no PDO is in range it contracts at 5 V (AlwaysEnableSink).
  - D+/D− are not used, so they go straight to the ESP32 USB.
  - The TPS2121 OV2 (about 13.6 V) stays as a backstop.
- **Pads (merged on the footprint: 20, 23, 32, 34):**
  - 1 LDO_3V3 (10 µF; TI allows 5–25 µF), 4 LDO_1V5 (10 µF, about 9.5 µF at 1.5 V; TI allows 4.5–12 µF), 38 VIN_3V3 (10 µF only, as on TI's EVM).
  - 8/9 I2C target tied to GND. 10 DBG_ACC, 13 PLUG_FLIP, 19 /SINK_EN, 37 PLUG_EVENT not connected.
  - 15, 30 and EP 40 are DRAIN: one net of their own, **never GND, no pours**.
  - 18 /FAULT_IN 10 k up to LDO_3V3. 26, 27, 36 RESERVED, 10 k each to GND.
  - 23 + 32 VBUS from the connector; 20 PPHV is the switched output (VUSB).
  - 28/29 CC1/CC2 with 330 pF to GND at the pins. Rd is internal: no 5.1 k resistors.
  - 6 CAP_MIS, open drain, 10 k up to 3.3 V, to ESP32 IO16.
- **Strapping** (100 k up from LDO_3V3, TI EVM ladder values down): ADCIN1 5.10 k (min 9 V), ADCIN2 23.2 k (max 12 V), ADCIN3 12.1 k + ADCIN4 5.10 k (2 A operating, 3 A max).
- **Caps:** VBUS 4.7 µF 50 V X7S (C95; C119 a DNP second one) + 2 × 10 nF; TI's CVBUS range is 1–10 µF. PPHV 10 µF 50 V + 100 nF. The 100 µF sink limit applies to everything on the path after a contract: the TPS2121 soft-starts only once per valid input (SLVSEA3F §9.3.1), then it's a 56 mΩ switch. So VUSB + VIN + buck inputs ≈ 50 µF nominal; C6/C7 (VIN 22 µF) are DNP options. The Flex amplifier's input caps add to it.
- **Protection:** TVS2200DRVR on VBUS (flat clamp; an SMAJ22A clamps near 35 V, above the chip), SS34 Schottky VBUS → GND, USBLC6-2SC6 on D+/D− referenced to +5V.
- **Connector:** GCT USB4105-GF-A (LCSC C3020560). A4/A9/B4/B9 VBUS, A1/A12/B1/B12 + shell GND, A5 CC1, B5 CC2, A6/B6 D+, A7/B7 D−, SBU open.

## Changes after the independent review (2026-10-04)
Three reviewers (power, digital, footprints) checked the netlist against the datasheets. Applied:
- **Wrong LCSC parts on the PoE input:** D1 was an SMAJ28CA (now C10762, SMAJ58A); BR1/BR2 were MB10S in the wrong package (now C124477, onsemi DF01S).
- **C2:** a 470 µF 16 V polymer doesn't exist in that size. Now Panasonic 16SVPF560M, Ø8 × 11.9 mm, `CP_Elec_8x11.9`.
- **100 µF USB PD sink limit** covers everything after the TPS2121 (it soft-starts only once): C6/C7 not fitted, VUSB down to 10 µF; about 50 µF in total, plus the Flex amplifier's own caps.
- **Bucks → TPS62933F** (forced PWM, same pinout): no light-load pulse skipping near the audio.
- TPS2121 OV dividers 118k → **120k** (trip 13.1–14.3 V, clear of a 12.6 V PD source); TPS25730 LDO caps → 10 µF 10 V 0805; CC caps 330 pF C0G 50 V; three 100 nF caps' LCSC fixed to the 0603 part (C14663).
- **I2S direction from XMOS's pinout:** DATA0 is the XVF3800's reference input and DATA1 its output. Fitted R74/R75 (IO7 → DATA0, DATA1 → IO6); R72/R73 not fitted.
- **Top-board cable pinout is mirror-symmetric** (1 GND, 2 +5V, 3 SDA, 4 LED, 5–6 +3V3, 7 BTN, 8 SCL, 9 +5V, 10 GND), so a reversed SH cable is harmless.
- **R83 100 k pull-down** on LED_DATA (IO38 floats during boot). W5500 crystal load caps 18 → **27 pF**.
- Opto U2 → `SO-4_4.4x3.6mm_P2.54mm` (+ LCSC C136411); SW1/SW2 → E-Switch TL3342F160QG (C2886898); magjack pads 13–16 → 1.25 mm, and Bel's via keep-outs as real rule areas.
- During layout, BR1's AC pins were swapped (they're interchangeable) so the centre taps route without crossing.

## Independent review 2026-10-06 (4 reviewers: pinouts, power, layout, digital/BOM)

Circuit: no pinout, pad or net errors (553 pins diffed against datasheets for the exact packages). W5500 matches WIZnet's reference, J3 matches Seeed's header, TPS25730 straps decode to 9 V min / 12 V max / 2 A, buck dividers 3.300 / 5.004 V, ratings all OK.

Circuit fixes decided:
- **TPS2121 fast switchover (XCOMP):** with CP2 to GND the switchover is 100 µs (SLVSEA3F §7.5 tSW) and only ~24 µF holds VIN, so a PoE→USB failover under load browns out the rails. Use CP2 = divider from VUSB (CP2 ≥ VREF when USB is present) and PR1 > CP2 while PoE is good: tFSW 5 µs, dip ≈ 0.4 V.
- R3 (opto pull-up) 10 k → 47 k (AT-DET LED current is only ~1.6–2 mA through the module's internal 27 k).
- R1 value field "≥68k trim, DNP" (below 68 k damages the Ag5412).
- 1 M pull-up on BTN_ADC (floats without the top board).
- I2C pull-ups 10 k → 4.7 k (with the Flex's 4.7 k and the top-board cable the rise time is marginal at 400 kHz).
- Fit HF caps where B.Cu signals cross the In2 VIN/GND split (C6/C7 or 100 nF stitching caps).
- BOM: MPNs for U7, the 10 µF 50 V / 100 nF 50 V / 22 µF 25 V parts; top-board BOM regenerated; C1525 and USB4105 out of stock at LCSC (alternates).
- Open: the Flex gets VIN on its 12 V terminal and +5 V on header pins 9/10; if the Flex makes its own 5 V from 12 V they may fight. Measure: Flex on its 12 V terminal only, voltage on header pins 9/10. Option: DNP 0 R in series with J3 9/10.
- SPEC IO40 polarity fixed (low = PoE+ 802.3at granted).

Layout problems (the redo addresses these):
- PoE isolation: PS1 pins 4–6 (no net, primary side) had GND planes 0.25 mm away on all layers and I2S_DIN/ESP_MCLK between them; U2's local 0.2 mm clearance masked the PoE rule; CHASSIS 0.25 mm from GND.
- Hairline joints: tracks ending at pad corners (0.02–0.07 mm wide joints, e.g. the whole W5500 SPI bus at U1). DRC connection_width warnings must be treated as errors.
- Decoupling far from pins (W5500 VDD cap 27 mm / 4 vias; U11 LDO caps 16.5 mm), C2 14 mm from PS1, +3V3A spread on thin tracks.
- Power tracks 0.4 mm (want ≥ 0.8–1.5 mm, ≥ 3 vias per layer change).
- 182 mm of signals on In2 cutting the GND fill; B.Cu signals over the In2 VIN island crossing the split.
- Ethernet pairs not coupled/matched; terminations on 10–19 mm stubs.
- U5 GND pin isolated on F.Cu by I2C_SDA running under the buck; tracks under inductors.
- SHT40 not thermally isolated (no slot, planes under it); 37 % of segments < 0.1 mm (stair-steps).

## Re-review 2026-10-07 and layout round 3

The redo (round 2) was re-reviewed by the same four reviewers. Pinouts: clean. Digital/BOM: clean (its "out of stock" list came from LCSC's detail API; JLCPCB's parts search, the same warehouse, shows every part in stock, e.g. C1525 22 M, ESP32-S3-WROOM-1-N16R8 28 k, SHT40 C2909890 21 k). Power and layout: not clean. Main points and what round 3 does about them:

- **Ag5412 isolation barrier:** Silvertel Ag5400 §6/Fig. 12: "it is essential that the isolation barrier is not breached" (band 15.3–42.1 mm from pin 1). Both earlier layouts kept only F.Cu clear. Now a rule area on all layers over the module body and transformer tab in that band, and only PoE copper under the module's primary half. Signals between the left and right halves go around the top.
- **Power copper drawn by hand:** VPOE plane from PS1 pin 8 to C2/C3 and the TPS2121 (was 42 mm of 0.8 mm track through four single vias); GND copper at PS1 pins 7/10; buck inputs with the 100 nF (now 0402) straight across VIN/GND pins 3/4, VIN pour with four vias to the In2 VIN area, GND vias at pin 4, small SW pour; copper on the PoE bridge pins (they dissipate up to ~1 W at full PoE+).
- **In2:** one GND plane with a single VIN area (TPS2121 output → above the ESP32 → past PS1 pin 10, since In2 can't pass between the 2.54 mm pins → band under the buck inputs). No B.Cu signal crosses its edge (router keep-out).
- **Inner-plane thermals were 0/0:** the planes only touched through-hole pads (44 connection_width warnings). Now 0.3 mm gap / 0.5 mm spokes. 0402 GND pads get 0.25 mm spokes (tombstoning).
- **TPS2121:** 1 µF at IN2 (C115) and OUT (C116), C6/C7 (DNP bulk) removed; dividers at 1.6 mm pitch with each bottom resistor under its top resistor (OV2/CP2 nodes were 10–14 mm loops).
- **W5500 turned to 0°** so its crystal/SPI side faces the ESP32 instead of the barrier; Y1 at pins 30/31 (XO 3 mm, XI 6 mm, no vias), R52 in-line at the pins, load caps straight off the crystal pads.
- **Mechanical:** J5/J6/J8 moved out of the radar's volume (y ≤ 44.4); "5V" tags at J6 pin 1 and J8 pin 5, "F1" at the Flex's pin 1, "+" at J4.
- Also: C117 (1 µF) at the CO2 LDO output, USB ESD U10 turned for flow-through and C43/C44 (DNP) removed, FID2 4.5 mm in from the edge, silkscreen 1.0 mm / 0.15 mm stroke, J1 marked through-hole.
- Notes: I2C must run at 100 kHz (XVF3800); the N16R8 is rated 65 °C ambient (PSRAM ECC → 85 °C); a reversed top-board cable isn't fully harmless (LED data on the button ladder).

## Review of round 3 (2026-10-08) and round 4

Round 3 was reviewed by the same four reviewers. Pinouts: clean. Digital/BOM: stale hidden "DNP" fields on four fitted parts (C81, R74, R75, U9; the netlist sync now removes them) and four parts JLCPCB doesn't stock (PS1, J3, L5, J1), so those go through Global Sourcing or consignment. Layout and power, and what round 4 does:

- **PoE isolation in plan view:** the 2 mm rule compares copper on one layer only, so the inner GND planes ran up to (and 0.15 mm under) PoE tracks on F.Cu, with 0.2 mm of prepreg between. Every GND pour is now cut 2 mm around all PoE copper (1.5 mm around CHASSIS); the "PoE primary" rule area grew with it.
- **Antenna:** a notch in the outline under the antenna (x ≥ 43.55, y −14.5 to 5.5), per Espressif's "antenna past the board edge".
- **VPOE pour:** its two halves met across a 0.5 mm diagonal; now about 2 mm.
- **Power tracks** widened where clearance allows (236 tracks; a few 0.6/0.3 vias to 0.8/0.4). Still thin: VBUS_RAW at J2/U11 (0.42–0.55 mm over about 10 mm; fine for the 2 A the PD straps ask for) and a single 0.8/0.4 via into U4.7.
- **Routing:** ETH_MISO 104 → 40 mm (the W5500 and ESP32 SPI pin orders differ, so MISO and SCLK cross once); RADAR_RX out of the 3.3 V buck's input loop; signals kept 0.6 mm clear of the In2 VIN gap and about 3.7 mm from the mounting-hole centres.
- **Assembly:** thermal spokes on the 0402/0603 decoupling GND pads and on PS1.7/8/10 and J4.2 (solder fill); J1 marked through-hole.
- **Circuit:** R82 33 Ω → 220 Ω (a reversed top-board cable puts U7's output on the ACTION button, which is 0 Ω to GND); C5/C115/C116 value text corrected to 50 V.
- **Not done:** the Ethernet pairs are still not coupled pairs (R55–R62/C63/C64 need re-placing in line between U3 and J1); at 100 Mbit over about 20 mm this is emissions margin, not link function.


## Layout round 7 (2026-10-08): flush ports

The case got a flat port face at y −51.4, so J1 and J2 moved outward (J1 2.06 mm, J2 1.81 mm) until their fronts sit on it. The outline's back flat went from y −49.0 to −49.5, with a tab out to −50.1 under J2's shield tabs (the wall has a matching pocket).

- **What the move broke, and how it was found:** the hand-drawn "PoE primary: no pour" edge had sat 1.2 mm below J1's RX pin row; after the move the row was inside it and the router could not reach pins 4–6. The PoE centre-tap runs had only their J1 ends dragged, so VC45's B.Cu rail ran through J1.16 (VC78), and VC12's rail was 1.4 mm from the RX pins. The rule area was cut back and VC12/VC45/VC78 re-laid for the new pin positions. Lesson: after moving a part, check the hand rule areas and the locked tracks of its nets, not only its pads.
- **Back edge:** with J1 2 mm closer to the edge, the strip between its shield and the board edge was too narrow for the signals running left–right along it. Two changes made room:
  - J1's two shield tabs are one metal shell, so the footprint now marks same-numbered pads as connected (KiCad jumper pads) and the CHASSIS track between them, whose 1.5 mm halo filled the strip, is gone.
  - The expansion header J7 (never fitted) is removed: its pins were the debug UART (on TP26/TP27), IO16 (in use as the PD mismatch flag) and IO47/IO48 (the second radar, on J8). Its spot at the left edge was what pulled five signals along the back edge. TP26/TP27 now sit beside the ESP32 at (41.6, −19.6/−17.6), just outside its antenna keep-out (which runs from x 43.55 down to y −28.5).
- **Tooling fixes found on the way:** the router's PoE void rectangles stopped exactly at the rule area's edge, so a track could run 0.03 mm inside it (now padded 0.1 mm). The T-joint snap moved track ends without a clearance check, and its first revert didn't work because KiCad's Python returns a live reference to a track's end point (both fixed).
- **Ethernet pairs:** ETH_J_TXP is 29.0 mm with 3 vias against 13.7 mm with 2 vias on ETH_J_TXN (the direct path is blocked by the RX parts row and J1's LED and 3.3 V tracks). The skew is about 100 ps, under 2° of phase at 100BASE-TX's 31 MHz, so it stays. The other pairs match within 3.5 mm.
- **Result:** DRC 0 unconnected / 0 errors, parity 194 parts / 561 pins, every net one piece of copper. Board: kicad-work/r3/final7/board_r7 (LINEAGE.txt). The four reviews ran on that board.

## Review of round 7 (2026-10-08) and round 8

The four reviewers found no blockers on board_r7. The majors and what round 8 does about them:

- **Pinouts:** J1, PS1, L5 and J3 had no LCSC number, so JLC's matcher could have fitted a generic 8.5 mm 2×10 socket for the 16.13 mm Samtec. All four now carry JLC's codes (C6377355, C40163463, C6827938, C3322059).
- **Power:** VBUS_RAW (J2 → U11, up to 3 A from a 5 V charger) came out of the round-7 re-route at 0.35 mm with single vias. Round 8 rips the whole J2/U10/U11 corner and routes VBUS_RAW at 0.8 mm, necking only right at the pads, then adds vias wherever 12 V or USB power changes layer.
- **Digital/BOM (corrected 2026-10-09):** the round-7 reviewer read C95 (4.7 µF 35 V 0603) as under a 4.5 µF minimum after derating. That figure is the CLDO_1V5 row: the rendered SLVSGP9 §6.4 table gives **CVBUS 1 / 4.7 / 10 µF** (min / nom / max), which the text extract had run together. C95 is now a 4.7 µF 50 V X7S 0805 (about 3 µF at 12 V). C119, a second one by J2's VBUS pins, is a DNP option (TI's note 3: the high side if fast sink-disable is configured). Fitting both would pass 10 µF at 0 V with tolerance. A single 1206 had no legal spot near U11 anyway.
- **Layout:** RADAR2_RX/TX were still 210 mm, along the back edge under J1/J2 and up the left side. Round 8 keeps every signal that doesn't belong there out of the strip below y −36, and keeps all B.Cu copper out from under the SHT40.

Also in round 8:
- **Exact JLC basic parts:** R17 C21189, R66 C17927, and the 50 V 100 nF caps C307331.
- **New parts:** C118 (100 nF at the USB ESD diode's 5 V pin) and R76/R77 (22 Ω source resistors on the Flex's BCLK/LRCK, above J3).
- **R55/R56 swapped**, so the TX pair no longer crosses.
- **Dead VIN loop beside the crystal deleted.**
- **Buck switch-node pours** at 0.25 mm clearance to their BST pads (they had pinched to 0.30 mm).
- **Antenna end:** signals now stay out of the strip x 41.5–43.55 along the antenna end.

Written down rather than changed:
- **Firmware notes in SPEC §4:** drive the unfitted MCLK/X1D13 GPIOs low, pull-ups on the radar RX pins, IO7 an input until the Flex firmware is confirmed, and the AIC3104 codec at 0x18.
- **Ordering:** JLC's fill and cap for the footprint pad holes; the J3 lead-free solderability question; the LD2450 cable wired by function.
- **ORDERING.md impedance text:** these are plain tracks, not controlled-impedance pairs.

## Round-8 re-reviews (2026-10-09) and round 9

The four focused re-reviews of board_r8 found no blockers. Round 8's own fixes held:
- VBUS_RAW J2 → U11 is now 3.57 mΩ (15.7 in round 7), through a B.Cu pour.
- RADAR2 is down to 95 and 122 mm.
- The Ethernet TX skew is 1.3 mm.
- C118, R76/R77, the SHT40 area and the JLC codes are all in.

Two majors remained. Round 9:
- **I2S_BCLK on the back edge:** moving RADAR2 off the strip below y −36 had pushed BCLK onto it, under J1/J2 where ESD comes in. The strip is now closed to BCLK, LRCK and both radars' UARTs. BCLK runs 116 mm over the front band again. I2C (100 kHz) and the stubs to the unfitted MCLK/X1D13 links stay in the strip; the firmware drives those GPIOs low.
- **Board out of sync with the netlist:** the board is re-synced. C119 is DNP, and TI's CVBUS range is really 1 / 4.7 / 10 µF. C40/C84 are now 22 µF 25 V (C45783) and C56/C57 10 µF 25 V (C15850), both JLC basic parts. Value, LCSC and DNP are now compared against the netlist along with the pins.

Also in round 9, from the power review's resistive model:
- The rest of the dead VIN ring, the via drilled into C116's pad, and three dead stubs are gone.
- The three vias it proved with DRC first vanished again (finalize's prune pass drops power vias within 0.7 mm of a same-net via); the round-9 confirmation put them back at positions further apart, locked: VUSB, VPOE into U4.7, and a second VBUS_IN via at U11.23. Per-via currents at 3 A USB / 2.5 A PoE are now 1.7 A at most.
- The via-doubling pass keeps clear of SMD pads and same-net vias, and now also covers +3V3. Its VIN vias at PS1.8 and VPOE vias above U4 were removed again: they carried almost nothing, narrowed the VPOE pour and holed the inner planes.
- TP9 sits outside the Flex, where it can be probed.

Accepted: USB_CC2 changes layer between U11.29 and C91. TI asks not to, but fixing it means re-placing the CC caps and PD straps in the densest corner, around the VBUS pour. It was the same in round 7.

Two confirmation reviews of the final board (power and layout) came back with no blockers and no majors. Left as minors:
- U11.32 (the LDO's VBUS input) has one 0.6/0.3 via at 1.59 A worst case; no legal spot for a second.
- I2S BCLK/LRCK run about 6 mm on B.Cu under the buck inductors, with both GND planes between; check BCLK for 500 kHz ripple on the bench.
- U4's output-pin escapes neck to 0.33/0.38 mm (plus a 0.19 mm parallel segment) at the 0.4 mm pads.

The board went into the real project on 2026-10-09 (DRC 0/0 in place, parity 198/569/0). The previous live board is backed up in kicad-work/best/live_backup_2026-10-09.

## Touch keys instead of buttons (2026-10-09)

The final product has a glass top with printed icons, so the four buttons became capacitive touch keys.
- **Controller:** Microchip AT42QT1070-SSUR (JLC C461610), comms mode, I2C 0x1B, on its own TLV75533 LDO. Datasheet 9596C guidance: 4.7–20 k in series with each key, 47 k on CHANGE, 100 nF at the pins, no shared logic regulator, up to 10 mm glass / 5 mm plastic. CAP1206 isn't stocked at JLC and CAP1203 has only 3 channels.
- **Why a separate touch disc:** the top board was one plane, and its LED ring has to sit level with the white band (z 120–125). That left the centre disc 8.5 mm below the glass (5 mm below the printed top cap), too much air for touch. So it split into an LED ring board (`topboard.py`) and a touch disc (`touchdisc.py`, Ø≈52 mm, two-sided). The disc's top side carries the pads and the LTR-303; a 0.8 mm adhesive foam with a window bridges the sensor and holds the pads flat against the cover. A 6-pin JST-SH joins the disc to the ring.
- **Mainboard:** unchanged. Its BTN_ADC line (J5 pin 7, IO1) now carries the touch chip's CHANGE interrupt, with 1 k in series on the ring board against a reversed cable.
- **Final product (Fusion):** a printed "touch carrier" ring (z 125–128.2, r 26.4–51.3, Ø2 mic ports) fills the gap between the mic gaskets and the glass and holds the disc's edge on a shelf. Planned retention: the carrier is bonded to the glass, so rim, glass and carrier come out as one top module.
