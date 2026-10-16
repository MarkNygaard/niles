# Niles satellite v2: PCB specification

The electronics for satellite v2, as custom boards assembled by JLCPCB (PCBWay is the fallback). They are designed around the
reSpeaker Flex (bought) and the case in the Fusion design **"Niles v2"**. The module-based parts list it
replaces is [`firmware/xiao-wyoming/satellite-v2.md`](../../firmware/xiao-wyoming/satellite-v2.md).

- KiCad project: [`niles-satellite-pcb-v2/`](niles-satellite-pcb-v2/) (KiCad 10).
- Circuit as code: [`circuit/`](circuit/) (`mainboard.py`, `usb_pd.py`, `topboard.py`, `touchdisc.py`). Datasheet notes behind every value: [`circuit/research-notes.md`](circuit/research-notes.md).
- Custom footprints: [`footprints.py`](footprints.py) → `niles_satellite.pretty/`.
- Order files and the scripts that make them: [`order/`](order/), [`scripts/`](scripts/) (see [ORDERING.md](ORDERING.md)).

Status (2026-10-12): **mainboard v1 ordered at JLCPCB** (198 parts; ORDERING.md). LED ring board (59 parts) and touch disc (22 parts): circuits complete and checked, not laid out yet.

**Not in this repository** (third-party CAD whose licence doesn't clearly allow re-publishing it, or files that are generated):

- Seeed's reSpeaker Flex CAD (STEP and DXF of the core board and the circular mic array), used for the case: from Seeed's [reSpeaker XVF3800 wiki](https://wiki.seeedstudio.com/respeaker_xvf3800_introduction/).
- Vendor 3D models the board refers to under `3d/`: Bel `0826-1X1T-GJ-F` (`Bel_0826-1X1T-GJ-F_box.wrl`, from Bel's STL), Samtec `ESQ-110-33-G-D`, JST `B4B-ZR` and `B5B-ZR`, Sensirion `SCD4x`. Download them from the makers and save them under those names; without them KiCad's 3D view just leaves those parts out. The `*_box.step` placeholders are ours and are included.
- The board STEP for the case model: `kicad-cli pcb export step --user-origin 100x100mm --no-dnp --subst-models -o niles-satellite-pcb-v2.step niles-satellite-pcb-v2.kicad_pcb` in `niles-satellite-pcb-v2/` (board underside at z 84 in Fusion).
- The loose Gerber files: they are what `order/gerbers.zip` holds, and `scripts/export_order.py` writes both.

---

## 1. Boards

| Board | Shape | Where | Job |
|---|---|---|---|
| **Mainboard** | Ø104 mm, straight edge at the back (y = −49.5), 4 layers, 1.6 mm | Floor of the electronics bay | MCU, Ethernet + PoE+, USB-C PD, power, sensors, connectors |
| **LED ring board** (`topboard.py`) | Ring Ø76–108 mm, 2 layers | Level with the white band, round the mic array | 24 side-emitting LEDs facing the band; 10-pin cable to the mainboard, 6-pin cable to the touch disc |
| **Touch disc** (`touchdisc.py`) | Ø≈52 mm, 2 layers, parts on both sides | Flat against the underside of the cover glass (final product) or the printed top cap (prototype), inside the mic ports | 4 touch keys (AT42QT1070), light sensor (LTR-303, top side) |
| reSpeaker Flex core board (Seeed) | 70.5 × 52.2 mm | **18.67 mm** above the mainboard, on a 2×10 socket and 4 M3 spacers | XVF3800 voice processing, 10 W amplifier |
| reSpeaker Flex mic array (Seeed) | Ring, Ø72.5 / Ø52.5 mm | Upside down against the top cap, foam gasket | 4 bottom-port IM69D130 mics |

The Flex sits at 18.67 mm, not 11, because the PoE module (14 mm) and the magjack (13.6 mm) are tall. The height is set by J3, a Samtec ESQ-110-33-G-D socket (16.13 mm tall, 2.3 mm tail) with the Flex's standard male header (2.54 mm body); there's no stock part for exactly 17 mm, and the next one down (16.13 mm) would leave the PoE module about 0.1 mm under the Flex's parts.

## 2. Block diagram

```
                        ┌──────────── mainboard ───────────────────────────────────────────────────┐
 RJ45 (PoE+) ─ Bel ─────┼─ data ─ W5500 ─ SPI ─────────────┐                                       │
   magjack      │       │                                   │                                       │
                └ CT ───┼─ 2× bridge ─ Ag5412 PoE+ ─ 12 V ─┐│                                       │
                        │                                  ├ TPS2121 ─ VIN 5–12 V ─┬─ J4 ─► Flex 12 V terminal ─► amp
 USB-C ─ TPS25730D ─────┼─ 9/12 V (5 V fallback) ──────────┘  mux    │              │
   │ D+/D− ─ USBLC6     │                                            │              ├─ TPS62933F 5 V ─┬─► Flex 5V_IN (pins 9, 10)
   └────────────────────┼─ USB ─► ESP32-S3-WROOM-1-N16R8 ◄───────────┘              │                ├─► J5 top board (LEDs)
                        │          │  │  │  │  │                                      │                ├─► J6 radar (LD2450)
                        │          │  │  │  │  │                                      │                └─► J8 radar (LD2410C)
                        │          │  │  │  │  └ ADC: VIN sense; touch interrupt        └─ TPS62933F 3.3 V ─► ESP32, W5500, sensors
                        │          │  │  │  └ IO38 ─ 74AHCT1G125 ─► LED data (5 V)
                        │          │  │  └ 2× UART ─ J6 radar, J8 radar
                        │          │  └ I2C ─ SHT40, SCD41, Flex (XVF3800), top board (LTR-303)
                        │          └ I2S (BCLK, LRCK, 2× data) ─ 2×10 socket ─► Flex
                        └──────────────────────────────────────────────────────────────────────────┘
```

Power and network are independent: the firmware uses **Ethernet whenever the link is up**, Wi-Fi otherwise.

| Plugged in | Powered by | Network |
|---|---|---|
| PoE+ cable | PoE (Ag5412) | Ethernet |
| USB-C only | USB-C (9/12 V PD, 5 V fallback) | Wi-Fi |
| USB-C + plain (non-PoE) network cable | USB-C | Ethernet |
| PoE+ cable + USB-C | PoE first (TPS2121), USB-C on standby, no back-feeding; switches to USB in about 5 µs if PoE goes (TPS2121 fast-switchover mode) | Ethernet; USB data still works for flashing and logs |

The PoE side is isolated, so a laptop on USB-C and a PoE switch never share a ground.

## 3. Components (mainboard)

| Function | Part | Why | Notes |
|---|---|---|---|
| MCU | **ESP32-S3-WROOM-1-N16R8** | Same chip family as v1 (firmware carries over), 16 MB flash, 8 MB PSRAM, PCB antenna | At a **side edge**, antenna pointing out, no copper under it |
| Ethernet | **WIZnet W5500** + 25 MHz crystal | Hardwired TCP/IP over SPI, ESP-IDF driver (`esp_eth`) | WIZnet's magjack reference circuit |
| RJ45 | **Bel 0826-1X1T-GJ-F** (magjack, 10/100, 802.3at) | PoE+-rated centre taps (720 mA) | Not at LCSC: DigiKey. Custom footprint, matches JLC's own for C6377355 to 0.007 mm (ORDERING.md) |
| PoE+ PD | **Silvertel Ag5412** (802.3at Class 4, 12 V, 30 W) + 2 × DF01S bridges, SMAJ58A | Certified module: detection, class, isolation and the converter done | 14 mm tall. Mouser. AT-detect → opto → IO40 |
| USB-C | **GCT USB4105-GF-A** | Power and firmware flashing (native USB on IO19/20) | Back flat, beside the RJ45 |
| USB PD sink | **TI TPS25730D** | Strapped by resistors to **9–12 V, never above**. Built-in switch and over-voltage cut-off. A 5 V charger still works | TVS2200 + Schottky on VBUS, USBLC6 on D+/D− |
| Power mux | **TI TPS2121** | Joins PoE 12 V and USB into VIN, PoE first. Cuts off either input above 13.1–14.3 V | Once on it's a plain switch, so everything on VIN counts against the USB PD 100 µF limit: VUSB + VIN carry ≈ 50 µF nominal, plus the Flex amplifier's own input caps |
| 5 V | **TI TPS62933F**, 3 A, 6.8 µH (forced PWM: no light-load pulse skipping near the audio) | Flex logic, LEDs, radar | Bridged solder jumper to isolate it |
| 3.3 V | **TI TPS62933F**, 3 A, 4.7 µH | ESP32 + W5500 peaks. Same part as the 5 V buck | Ferrite to a quiet 3V3A for the Ethernet PHY |
| LED level shift | SN74AHCT1G125 | 3.3 V data to the 5 V LEDs | 220 Ω series out (holds a reversed top cable's short to 23 mA) |
| Temperature/humidity | **Sensirion SHT40** | | At the left edge (x −46, y 16), far from the bucks, no copper on the inner layers or bottom under it (no slot) |
| CO₂ | **Sensirion SCD41** | Real CO₂ (photoacoustic), for ventilation reminders | Fitted from v1, by the front-right wall: the case needs a vent there |
| To the Flex | 2×10 female socket, 2.54 mm | Plugs onto a 2×10 male header soldered under the Flex | Position in §6 |
| Amp power to the Flex | J4, 2-pin JST-XH + cable to the Flex's 12 V screw terminal | | |
| To the top board | J5, 10-pin JST-SH: 1 GND, 2 +5V, 3 SDA, 4 LED data, 5–6 +3V3, 7 touch CHANGE (interrupt, active low), 8 SCL, 9 +5V, 10 GND | Mirror-symmetric power: a reversed cable shorts no rail, but puts the 5 V LED data on the touch chip's CHANGE pin (a 1 k series resistor on the top board limits that to about 1.4 mA), so use a straight-through cable | Same pin order at both ends |
| To the radar | J6, 4-pin JST-ZH: 5 V, RX, TX, GND (the LD2450's own order) | LD2450 module on the front wall: positions of up to 3 people | |
| To the second radar | J8, 5-pin JST-ZH in the LD2410C's own hole order: 1 TX, 2 RX, 3 OUT (unused), 4 GND, 5 5 V; UART2 (IO47/IO48) | LD2410C module beside the LD2450: still presence (someone sitting still) | The module has bare 2.54 mm holes: solder pins on (or buy the pinned LD2410C-P) and use a straight ZH-to-Dupont 5-pin cable |
| Debug UART | TP26 (TX) / TP27 (RX), 1 mm pads below the ESP32 (x 41.6, y −19.6 / −17.6, just outside its antenna keep-out); GND at TP7/TP8 | No expansion header (removed 2026-10-08): it had no spare pins left (IO16 is the PD flag, IO47/IO48 the second radar on J8), and its spot at the left edge pulled five signals along the back edge under J1 | |

**LED ring board:** 24 × **OPSCO SK6812SIDE-A** (4020 side-emitting) round the outer edge, facing the band, each with 100 nF.

**Touch disc:** 4 **capacitive touch keys** (12 mm copper pads under the solder mask) at r = 20 mm in a cross, under the icons on the cover glass or the printed top cap, read by a **Microchip AT42QT1070** (SOIC-14, LCSC C461610) over I2C at 0x1B. It runs on its own TLV75533 3.3 V LDO from +5 V (Microchip advise against sharing a logic rail), with 10 k in series with each pad, and signals key changes on its open-drain CHANGE line (47 k pull-up) to IO1 via the ring board (1 k in series there). It senses through up to 10 mm of glass or 5 mm of plastic. **Lite-On LTR-303ALS-01** light sensor on the top side, under a window in the cover's back print or the top cap. Everything else sits on the bottom side. A 0.8 mm die-cut adhesive foam with a window over the LTR-303 holds the pads flat against the cover. Keep the LED data line and the LED supply away from the key traces (datasheet layout note).

## 4. ESP32-S3 pin allocation

Avoided: GPIO26–37 (flash and octal PSRAM), 3/45/46 (strapping), 19/20 (USB), 43/44 (debug UART).

**I2C runs at 100 kHz:** the XVF3800 on the Flex (0x2C) supports 100 kbit/s only, and it shares the bus with the SHT40 (0x44), SCD41 (0x62) and LTR-303 (0x29).

**The N16R8 module is rated for 65 °C ambient** (WROOM-1 datasheet, R8 variants). The case holds two bucks, the Ag5412 and the amplifier: enable PSRAM ECC (85 °C, at the cost of 1/16 of the PSRAM) or measure the inside temperature.

| GPIO | Signal | GPIO | Signal |
|---|---|---|---|
| 0 | BOOT button | 13 | W5500 MISO |
| 1 | Touch CHANGE from the top board's AT42QT1070 (input, low = read the key status) | 14 | W5500 INT |
| 2 | VIN sense (ADC1_CH1, 100 k / 22 k) | 15 | Flex MCLK |
| 4 | I2S BCLK | 16 | USB PD capability mismatch |
| 5 | I2S LRCK | 17 | Radar RX (LD2450 TX) |
| 6 | I2S data in (Flex DATA1 → ESP) | 18 | Radar TX |
| 7 | I2S data out (ESP → Flex DATA0) | 21 | W5500 RST |
| 8 | I2C SDA | 38 | LED data (via 74AHCT1G125) |
| 9 | I2C SCL | 39 | Power source (TPS2121 ST: high = PoE, low = USB) |
| 10 | W5500 CS | 40 | PoE+ (Ag5412 AT-detect via opto) |
| 11 | W5500 MOSI | 41 | Status LED |
| 12 | W5500 SCLK | 42 | Flex X1D13 |
| 19/20 | USB D−/D+ (0 Ω) | 47/48 | Second radar RX/TX (UART2, J8). 3.3 V on the N16R8 |
| 43/44 | Debug UART TX/RX (pads TP26/TP27) | | |

XMOS's XVF3800 pinout: DATA0 (X1D00) is the reference input from the host, DATA1 (X1D22) the processed audio out to the host. Fitted: DATA1 → IO6, IO7 → DATA0. Both go through 0 Ω links that can be swapped. MCLK (R70) is not fitted: the Flex runs XMOS's I2S-master firmware at 48 kHz (as satellite v1 does), which makes its own clocks.

**Firmware notes from the board reviews (2026-10-08):**
- Drive IO15 (ESP_MCLK) and IO42 (ESP_X1D13) low as outputs while R70/R71 aren't fitted: each is a long unterminated track.
- Turn on the internal pull-ups on IO17 and IO47 (radar RX): with a radar unplugged they float.
- Touch keys: on IO1 going low, read the AT42QT1070 (I2C 0x1B) key status bytes, which releases CHANGE. Disable keys 4-6 (their pins are open), set the detect thresholds for the cover thickness, and trigger a calibration after start-up.
- Keep IO7 (I2S DOUT, to the Flex's DATA0) an input until the Flex runs the I2S firmware: XMOS's stock USB firmware drives DATA0 as an output.
- The Flex's TLV320AIC3104 codec answers at I2C 0x18 (beside the XVF3800 at 0x2C); set it up, it doesn't clash with anything.
- Power up the SCD41 (+3V3_CO2, via U12 from 5 V) before driving I2C: the 3.3 V pull-ups would otherwise feed it through its I2C pins. Normal start-up already does this.

## 5. The Flex 2×10 header (from Seeed's schematic and photo)

| Pin | Signal | Pin | Signal |
|---|---|---|---|
| 1 | MCLK | 20 | I2S BCLK |
| 2 | I2S LRCK | 19 | I2S DATA1 |
| 3 | I2S DATA0 | 18 | I2S DATA2 (test point) |
| 4 | GND | 17 | GND |
| 5 | I2C SDA (4.7 k pull-up on the Flex) | 16 | SPI CS_N (test point) |
| 6 | I2C SCL (4.7 k pull-up on the Flex) | 15 | SPI CLK (test point) |
| 7 | X1D13 | 14 | SPI MOSI (test point) |
| 8 | GND | 13 | SPI MISO (test point) |
| 9 | 5V_IN | 12 | GND |
| 10 | 5V_IN | 11 | VDDIO (output; do not drive) |

## 6. Mechanical (Fusion "Niles v2", mainboard centre = case axis)

Coordinates in mm, Fusion orientation: +Y is the front (radar side), −Y the back (sockets). KiCad: (100 + x, 100 − y).

| What | Where |
|---|---|
| Board outline | Circle r = 52, cut straight at y = −49.5 (31.9 mm flat) with a tab out to −50.1 between x 3.5 and 15.5 under J2's shield tabs (the case wall has a matching pocket); J1 and J2 fronts at y −51.4, flush with the case's port face; notch x ≥ 43.55, y −14.5 to 5.5 so the ESP32 antenna overhangs air |
| Bay wall | r 53 inside, 55.5 outside (2.5 mm) |
| Flex mounting holes (M3 spacers, **18.67 mm**: the Samtec socket's 16.13 mm body plus the Flex header's 2.54 mm plastic, so the header seats fully; the spacer tubes are part of the bay, z 85.6–104.3, on arms to the wall at z 100.5–104.1) | (±31.24, ±22.10) |
| 2×10 socket | Flex pin 1 at (−33.72, 11.92); pins 1–10 down x = −33.72, pins 20–11 down x = −31.18, 2.54 pitch. Socket pad 1 = Flex pin 20 |
| Flex footprint above (parts under it ≤ 15 mm) | x −35.3…35.3, y −26.1…26.1 |
| Socket alcove (RJ45 + USB-C) | Back, x −16.5…16.5, at the flat edge |
| Radar (LD2450) | Front wall, x −22…22, y 41.6…47.6 (cradle), z 98–113 (Fusion, board underside z 84) |
| Second radar (LD2410C, placeholder 22 × 16 × 4) | Front wall, 45° right of the front axis: centre (35.0, 35.0), antenna face 1.5 mm inside the 2.5 mm wall, z 97.5–113.5, in its own cradle (ledges with a 6 mm cable gap, side stops 0.2 mm clear, back lips at the top). Its inner end sits just past the LD2450 cradle's side stop. Cable to J8 about 30 mm |
| Top module, final product (2026-10-09) | Rim + cover glass + touch carrier bonded together (glass into the rim; carrier to the glass with 0.13–0.25 mm adhesive transfer tape such as 3M 468MP / 9473, die-cut with the mic ports; touch disc to the glass with its 0.8 mm adhesive foam). It locks to the diffuser band (glued to the bay's top edge) with a **bayonet**: 3 lugs under the carrier (0°, 120°, 240°, 10° wide, r 50–54, z 122.4–124.85) sit 0.2 mm under 3 tabs inside the band (12° wide, r 53–54.3, z 123.6–125). Twist 12° to lock or release. The LED ring (board z 118.4–120, LEDs up to 122) stays under the lugs (from z 122.4), so it needs no free sectors; the tabs throw three small shadows on the band |
| Port openings (2026-10-09, fitted to Bel's STL) | J1: x −16.15…2.15, z 84.40…99.44 (the real jack's shield is ±8.84 wide within the wall, and its front lip hangs 1 mm below the board top in front of the board edge). J2: x 4.71…14.29, z 85.28…89.14. Port face at y −51.7 (moved out 0.3 mm on 2026-10-09): the jack's front frame is flush (0.05 mm back), its top lip 0.56 mm proud; USB-C sits 0.3 mm back. Both openings are open down to the bay's bottom edge (J2's lower half squared off), because the board goes into the bay from below; a tongue on the chamber lid fills the slots under the board (z 80–83.9, flush with the port face) |
| Case parts and joints, prototype (2026-10-09) | Six printed parts, all printable without support except the bay's four spacer arms (tree supports under them). **Base** → **speaker chamber**: 3 × M3 × 25 countersunk from underneath, through the base posts into inserts in the chamber's three tie pillars (r 49.5 at 90°, 210°, 330°). **Chamber** (open cup, floor z 18–22, wall to z 77) → **chamber lid** (z 77–80, board bosses to 84): 3 × M3 × 8 countersunk down into inserts at the pillar tops, silicone bead on the wall top for the seal. **Bay** (open ring, z 80–120) sits on the lid and is clamped by the 4 Flex screws (M3 × 25) through its own spacer tubes, the mainboard and into the lid's inserts. **Diffuser band** glued to the bay's top edge. **Top cap** bayonets into the band. All inserts: M3 heat-set, at most 4 mm long, in Ø4.0 holes |
| Bayonet, prototype | Like the final product's, but tabs and lugs at 0°, 110°, 235° (not 120° apart), so the cap only locks fully the right way round (icons to the front). Lugs under the cap: stem r 50–52.8, foot r 52.7–54.1, z 122.4–123.4. Tabs inside the band: r 53–54.4, z 123.6–125, 12° wide, each with a stop rib at its clockwise end. Drop the cap in 12° anticlockwise of locked, twist clockwise until it stops |
| Speaker | SB65WBAC25-4 (64 × 64 mm frame, 3 mm flange, 4 × Ø3.8 holes on Ø72, basket Ø59.5, 38 mm deep) sits on the chamber floor inside the cup, cone down through the Ø56 hole: 4 × M3 × 12 pan head down through flange and floor, nyloc nuts underneath; a 1 mm foam gasket or a silicone bead under the flange. Air volume about 0.39 L after the pillars |
| Speaker wire | Through a Ø4 hole in the lid at 128°, r 48.5 (seal with silicone), out under the board edge (4 mm gap) into a 4 × 1.5 mm channel in the bay wall at 128° (z 80–92), up the wall, over the Flex's edge to its 2-pin speaker connector (JST PH, at x −20.9, y 23.1). About 150 mm of 2 × 22 AWG |
| LED ring board mount | Ring r 38–52.8 (Ø76–105.6), 1.6 mm, underside on three printed pads at z 118.4 (top flush with the bay's top edge, z 120). Three M2 holes at r 49.5, 10°/130°/250° (pads have Ø1.7 pilot holes, M2 × 6 self-tapping). 24 side LEDs at the outer edge, light face at r 52.8, 1.5 mm from the band |
| Touch disc mount | Ø52 disc flat under the top cap on its 0.8 mm adhesive foam (disc z 122.6–124.2), located by four Ø2 pins on the cap in its Ø2.2 holes at (±12, ±12). Pads under the engraved icons: action (0, 20), mute (0, −20), volume down (−20, 0), volume up (20, 0). LTR-303 at the disc centre, top side, under the cap's Ø2 light window |
| Assembly order | 1. Speaker wire down through the lid hole, out through the floor hole; solder, screw the speaker in. 2. Inserts; lid onto the chamber (silicone), 3 lid screws; silicone the wire hole. 3. Base onto the chamber, 3 screws from underneath. 4. Mainboard up into the bay from below (J1/J2 slide up their slots), set the bay on the lid. 5. Flex onto the header and the tubes, 4 × M3 × 25. 6. Radars into their cradles, cables. 7. LED ring onto its pads (3 × M2), 10-pin cable to J5. 8. Glue the band. 9. Mic array and touch disc onto the cap, 6-pin cable to the ring; bayonet the cap on |
| ESP32 | Side edge, \|x\| > 35.3, antenna pointing out, keep-out under the antenna on all layers |
| PD chip DRAIN pad | Own copper island only, no pour, no GND |

**As placed (2026-10-07, layout round 3):**

| Area | What |
|---|---|
| Back flat | Magjack J1 (x -16 to 2) and USB-C J2 (x 5 to 14), openings at the edge; USB ESD U10 beside J2 (flow-through); PD chip U11 turned 180 degrees so VBUS and CC face the connector |
| Left | Flex socket J3; the PoE input side (bridges with their own copper, TVS, C1, opto LED) in its own 2 mm isolated region; SHT40 U8 at the edge (x -46, y 16) |
| Middle, under the Flex | Ag5412 PS1 (body y -0.6 to 18.9 with the transformer tab). **Isolation barrier** (Silvertel Fig. 12, 15.3 to 42.1 mm from pin 1): no copper on any layer under the module there; under the module's primary half only PoE copper. Signals between the left and right halves go around the top (B.Cu above the bucks) |
| Below the module's tab | W5500 U3 at 0 degrees: XI/XO/SPI face the ESP32, the 25 MHz crystal at pins 30/31; terminations between U3 and the magjack |
| Right | ESP32 U1 (x 24 to 50, y -14 to 5), antenna keep-out from x 43.5 on all layers; power switch U4 cluster at (16, -4) with local caps on IN2 and OUT; VPOE plane from PS1 pin 8 to C2/C3 and U4 |
| Front band (y 24 to 34) | Both bucks laid out to TI's guide (5 V left, 3.3 V right; 100 nF across VIN/GND pins, GND vias at the pin); top-board J5, radars J6 and J8 behind the radar's volume (y <= 44.4), level shifter U7; CO2 U9 with its LDO U12 |
| Layers | L1 parts and signals, L2 solid GND (no traces), L3 GND + one VIN area (U4 to the buck inputs; no L4 signals cross it), L4 signals + GND fill |

## 7. Power budget (peak)

| Load | Rail | Peak |
|---|---|---|
| Flex amplifier, 10 W into 4 Ω | VIN | 10 W |
| Flex logic | 5 V | 1 W |
| ESP32-S3 Wi-Fi TX + W5500 | 3.3 V | 2 W |
| 24 LEDs, firmware-limited to about 30 % white | 5 V | 2 W |
| LD2450 + LD2410C radars | 5 V | 0.9 W |
| Sensors, PD, mux, bucks' losses | — | 1 W |
| **Total** | | **≈ 17 W** |

PoE+ gives 25.5 W at the device. USB-C contracts 12 V × 2 A (24 W) or 9 V × 2 A (18 W), both enough. On a 5 V-only charger the board runs, but the 5 V rail sits slightly under 5 V (the buck has no headroom), the amplifier is quiet, and the LEDs are dimmed. The firmware reads VIN, the power source and the PD mismatch flag, and shows them. **Firmware rule:** keep the LEDs and the amplifier off until VIN is above about 8 V, because on a 5 V-only charger the 5 V rail sits near 4.4 V (the LED level shifter needs 4.5 V) and the board may draw more than the port offers. The same on plain PoE (802.3af, about 13 W at the device): the Ag5412's AT-detect pulls low only after a two-event (802.3at, 30 W) classification, so IO40 is **low on PoE+ and high on plain PoE** (and high on USB power; read it only when PWR_SRC, IO39, says PoE). On plain PoE the firmware caps the amplifier volume and LED brightness so the switch doesn't cut the port. Passive PoE isn't supported.

## 8. Built in for the first revision

- **Test points** on every rail (VIN, 5 V, 3.3 V, PoE 12 V, VUSB) and on I2S, I2C, SPI and UART
- **Solder jumpers** (bridged) to disconnect each buck from its rail, and **0 Ω links** to swap the I2S data lines
- **USB flashing** with BOOT and RESET buttons, a debug UART, and a status LED
- **Unpopulated options:** the Ag5412 voltage trim, the I2S swap links
- **Fiducials** for assembly

## 9. Still to settle

- **Two 24 GHz radars side by side:** both are Hi-Link FMCW modules in the same band, so they can disturb each other. Bench-test an LD2450 and an LD2410C next to each other before the case is final; if the LD2410C sees ghosts, lower its far gates or angle the two modules apart in the front window.
- **Cables:** the top board's SH cable and the LD2410C's ZH-to-Dupont cable are straight-through (pin 1 to pin 1). The LD2450's ZH cable goes by function: J6 pin 1 (5V) to the module's 5V contact; Hi-Link doesn't say which end of its socket is pin 1, so meter the cable before the first power-up.
- **The Flex's stack height (J3):** decided 2026-10-06: 18.67 mm (ESQ-110-33-G-D). Samtec stocks it; LCSC lists it (C3322059) but had none, so PCBWay buys it from Samtec or DigiKey.

1. **The Flex's I2S direction:** set from XMOS's datasheet (§4); confirm on the bench. The swap links cover it if it's the other way.
2. **Sourcing:** decided 2026-10-08: JLCPCB, with J1, PS1, L5 and J3 through its Global Sourcing (or consigned). See `ORDERING.md`.
3. **The ring and disc heights:** the ring's LEDs must face the white band (z 120–125 in Niles v2), and the touch disc sits flat against the underside of the cover glass or top cap (no air gap beyond the 0.8 mm foam).
4. **The order the case parts go together around the boards:** cable lengths, which side the screws go in, and how the Flex and mainboard come out for service.
5. **The Flex amplifier's input capacitance** (12 V terminal): measure it, or ask Seeed. Together with our ≈ 50 µF it should stay near the USB PD 100 µF sink limit.
6. **The Fusion model** (after routing, from the board's STEP): boardStack 11 → 18.67 mm; the port opening reshaped to J1/J2 on the board (Bel's suggested RJ45 opening: **17.07 × 14.00 mm**, panel inside face 2.03 mm in front of the jack's face; the 3D placeholder in `3d/` has the plug cavity); remove the two old Ø2.6 port-screw holes in the back wall; a light window in the top cap for the LTR-303; vents beside U8 plus a second set opposite, each with a printed baffle 2–3 mm behind so you can't see in; split into an inner chassis (electronics, speaker seal, mic gasket, LEDs) and an outer skin, so the skin can later be wood veneer or a metal base.

**BOM (checked 2026-10-06):** every fitted part has a manufacturer part number and, where LCSC stocks it, an LCSC number (passives by value in `circuit/parts.py`, all Active, LCSC stock preferred). Not at LCSC, bought through JLCPCB's Global Sourcing (DigiKey/Mouser): J1 Bel magjack (DigiKey), PS1 Ag5412 (Mouser/DigiKey), L5 Würth 74439346047 (DigiKey). J3 Samtec ESQ-110-33-G-D (Samtec/DigiKey).

## 10. Order

JLCPCB PCB + PCBA (decided 2026-10-08, after the BOM moved to JLC basic parts): every setting, the Global Sourcing parts and the hardware list are in **`ORDERING.md`**. The PCBWay turnkey settings below (2026-10-04) stay as the fallback.


| Setting | Value |
|---|---|
| Service | Turnkey (they source everything, including the DigiKey/Mouser parts), single pieces, **top side only** |
| Quantity | 5 boards, **assemble 3** (the other 2 stay bare in case of a fix) |
| Alternatives made in China | Yes for passives; the BOM marks the ICs "no substitution" |
| Board | 4 layers, 104 × 104 mm, FR-4 **S1000H TG150**, 1.6 mm, 1 oz outer and inner |
| Limits | 4/4 mil track/space, 0.2 mm minimum hole, tented vias; **vias in the ESP32, CO2 sensor and PD chip exposed pads resin-filled and capped** |
| Finish | **ENIG 1U"** (0.4 mm-pitch PD chip, LEDs), green mask, white silkscreen |
| Silkscreen | Top: labels for connectors, ICs, modules, buttons, jumpers; what plugs in where (LD2450, LD2410C, TOP BOARD, AMP 12V, FLEX); test-point net names. Passive references hidden: the assembly drawing (`assembly-top.pdf`, F.Fab) has them all. Bottom: the Niles logo (butler, from `web/src/assets/butler.svg`) and "NILES satellite / v1  2026-10" (the first board made; "v2" is the product generation, not the board revision) |
| Layer order | L1 `F_Cu`, L2 `In1_Cu`, L3 `In2_Cu`, L4 `B_Cu` |
| Impedance control | Optional: USB 90 Ω diff (0.22 / 0.15 mm) and Ethernet 100 Ω diff (0.20 / 0.20 mm), on L1 over L2. Full-speed USB and a few cm of 100BASE-TX tolerate ±15 %, so it can be left off to save cost |

Stack-up (PCBWay standard): L1 | 7628 prepreg 0.196 mm, Dk 4.74 | L2 | core 0.96 mm, Dk 4.6 | L3 | 7628 prepreg 0.196 mm | L4.

## 11. How it gets built

1. **Circuit as code** (SKiDL): every part, footprint and connection in a reviewable Python file. `python mainboard.py` → `mainboard.net`; `python check_net.py mainboard.net` checks it.
2. **Netlist into KiCad** (File → Import → Netlist): all footprints appear with their connections. Each part's tag is its reference, so re-importing after a change keeps the placement.
3. **Placement** follows §6 (script plus hand). **Routing:** Ethernet, PoE, USB PD and the bucks by hand, with checklists; the rest by hand, then reviewed.
4. **DRC against PCBWay's 4-layer rules**, a 3D check in Fusion (STEP export), then **Gerbers, BOM and pick-and-place files** to PCBWay for 5 assembled sets.
