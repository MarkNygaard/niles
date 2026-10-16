# Niles satellite v2: build list

The next satellite. It has the same XVF3800 voice chip as v1, with four improvements:

- **A real speaker.** A 2.5″ full-range driver in its own sealed chamber, on a 10 W amplifier.
- **Presence radar.** An mmWave sensor that tells Niles who is in the room and where.
- **Mics separated from the speaker.** The mic array sits at the top of the case and the speaker at the bottom, connected by a flex cable. This is what makes echo cancellation and barge-in work best.
- **Two ways to connect:** **USB-C power + Wi-Fi**, or **PoE+ with wired Ethernet**. Never PoE with Wi-Fi: when there is a cable, the data goes over it.

It uses the same firmware approach as v1: on-device wake word, raw PCM streamed over TCP to `niles-wyoming` ([README](README.md)).
The concept model is the Fusion design **"Niles v2"**.

> **Electronics update (2026-10-04):** v2 is now built on a custom PCB assembled by JLCPCB instead of the
> modules below. The board spec, parts and status live in [`hardware/satellite-v2/SPEC.md`](../../hardware/satellite-v2/SPEC.md).
> This file still holds the case, speaker, radar and Flex details.

Written for buying from Denmark. Prices are from October 2026 and include VAT unless noted.
EU shops come first. UK shops (The Pi Hut, etc.) add customs and import VAT after Brexit.
AliExpress links are search links. Pick a seller with good ratings and "Choice" shipping.

---

## How it fits together

```
 USB-C charger ─► PD trigger (15 V) ─► buck 15→12 V ─┐
                                                     ├─► 12 V ─► reSpeaker Flex (amp 10 W) ─► SB65 speaker
 PoE+ switch ─► PoE+ splitter ── 12 V ───────────────┘     │
                     │                                     └─► buck 12→5 V ─► ESP32-S3-ETH, LED ring, radar
                     └── data ─► patch cable ─► ESP32-S3-ETH RJ45 (W5500)

 ESP32-S3-ETH ◄─ I2S + I2C ─► reSpeaker Flex core ◄─ FPC ─► mic array (top)
              ◄─ UART ─ LD2450 radar · ─ data ─► WS2812 ring · ─ I2C ─ sensors · ◄─ 4 buttons (1 ADC pin)
```

Each satellite is fitted with **one** of the two inputs: the USB-C trigger or the PoE splitter. Everything after the 12 V point is identical, and so is the firmware.
The firmware uses Ethernet when a cable is connected and Wi-Fi otherwise.

---

## 1. Voice: reSpeaker Flex XVF3800 (circular), **without** XIAO

| | |
|---|---|
| What | XVF3800 processing board + separate **circular 4-mic array** (Ø73 mm, 44 mm mic spacing) on a 20 cm FPC cable, **TPA3139D2 class-D amp (10 W into 4 Ω)**, 3.5 mm jack, 2-pin speaker connector, 12 V terminal or 5 V USB-C/JST input, I2S + I2C header |
| Why | Same voice chip and processing as v1 (AEC, beamforming, DoA, noise suppression), but the mics can go at the top of the case, away from the speaker, and there's a real amplifier |
| Buy (EU) | [OpenELAB: Flex circular **without** XIAO](https://openelab.io/nl/products/seeed-studio-respeaker-flex-xvf3800-circular-4-without-xiao-esp32s3) (EU warehouse, duties covered) |
| Docs | [Seeed wiki: reSpeaker Flex](https://wiki.seeedstudio.com/respeaker_flex_introduction/) |

Get the **circular** array (360° pickup for a room). The linear version is for front-facing devices.
The version without XIAO is right here: the controller is the Ethernet board below.

## 2. Controller: Waveshare ESP32-S3-ETH

| | |
|---|---|
| What | ESP32-S3R8 (8 MB PSRAM, 16 MB flash), **W5500 Ethernet with RJ45**, Wi-Fi, USB-C, Pico-style 2×20 pin header |
| Why | Same chip family as v1's XIAO, so the firmware carries over, with Ethernet built in and enough pins for the Flex audio and configuration lines, radar, LED ring and buttons |
| Buy (EU) | [Electrokit (SE), 319 SEK with PoE module](https://www.electrokit.com/en/development-board-esp32-s3-ethernet-poe) (currently out of stock) · [Waveshare](https://www.waveshare.com/esp32-s3-eth.htm) · [AliExpress](https://www.aliexpress.com/w/wholesale-waveshare-esp32-s3-eth.html) |
| Docs | [Waveshare wiki: ESP32-S3-ETH](https://waveshare.com/wiki/ESP32-S3-ETH) |

Buy it **without** Waveshare's PoE module. That module is 802.3af and powers only the board at 5 V, while the amplifier needs 12 V. The PoE+ splitter in section 6 does the job instead.

## 3. Speaker

| Part | Where | Price |
|---|---|---|
| **SB Acoustics SB65WBAC25-4**: 2.5″ full-range, 4 Ω, aluminium cone | [Sound Imports (NL)](https://www.soundimports.eu/de/sb-acoustics-sb65wbac25-4.html) · [Tout le Haut-Parleur (FR)](https://en.toutlehautparleur.com/brands-page/sb-acoustics/by-type/wideband.html) | about €33 |
| Polyfill damping for the sealed chamber (one bag covers many satellites) | [Sound Imports: Acousta-Fill 1, 450 g](https://www.soundimports.eu/en/acousta-fill-1.html) | about €10 |
| Thin foam gasket tape (1–2 mm EPDM/Poron), to seal the driver flange and the mic ports | [AliExpress: EPDM foam tape 1mm](https://www.aliexpress.com/w/wholesale-epdm-foam-tape-1mm.html) | about €3 |

SB65 data: Vas 0.43 L, Fs 115 Hz, Qts 0.68, Xmax 2.65 mm, 4 Ω, 20 W, 38 mm deep, four Ø3.3 mounting holes.
The model's sealed chamber is **about 0.40 L of speaker air**, which gives Qtc ≈ 0.95 and a natural roll-off around 160 Hz. The amplifier's EQ can extend that.
4 Ω suits the Flex amplifier: 12 V into 4 Ω gives the full 10 W. A grille can be printed into the case, so no cloth is needed.

## 4. Presence radar

| Part | Where | Price |
|---|---|---|
| **HLK-LD2450**: 24 GHz radar that tracks up to 3 people with X/Y position | [SmartHomeShop.io (NL)](https://smarthomeshop.io/en/products/hlk-ld2450-24ghz-mmwave) · [AliExpress](https://www.aliexpress.com/w/wholesale-ld2450.html) | about €9.50 (NL), less on AliExpress |
| Optional: **HLK-LD2410C** for detecting people who are sitting still (the LD2450 tracks movement) | [AliExpress](https://www.aliexpress.com/w/wholesale-ld2410c.html) | about €3 |

The LD2450 uses 5 V power and 3.3 V UART at 256000 baud, on a spare UART of the ESP32 board. It looks through the front wall, so leave that wall plain plastic: no paint, no metal.
Niles benefits most from **position**: the XVF3800 reports which direction a voice came from (DoA), and combining that with the radar's positions tells Niles *who* spoke.

## 5. Light ring and buttons

The Flex has no LED ring like v1's board, so the listening and thinking lights need their own ring, driven by the ESP32 board.
On top there are **four buttons, like the Echo Dot**: volume up and down on the sides, **action** at the front and **mute** at the back. They sit **in a cross 20 mm from the centre**, like the Echo Dot. That is possible because the Flex's mic array is a **ring** (Ø72.5 mm outside, Ø52.5 mm hole): the buttons go through the hole, on a small round button board under the top cap. The array itself is mounted upside down against the cap, because its IM69D130 mics hear through holes in the board. The buttons are **recessed**: each sits in a shallow round dip, its top 0.5 mm below the surface, with a raised symbol. Each is a printed cap through the top cap, with a small flange underneath, pressing a tactile switch.

| Part | Where | Price |
|---|---|---|
| **WS2812B 24-LED ring** (about 66 mm outside diameter), a few mm behind the white light band | [AliExpress: WS2812B 24 ring](https://www.aliexpress.com/w/wholesale-ws2812b-24-led-ring.html) | about €2–4 |
| 74AHCT125 level shifter (only needed if the ring flickers on 3.3 V data) | [AliExpress](https://www.aliexpress.com/w/wholesale-74ahct125.html) | about €2 for 10 |
| **4 tactile switches, 6×6 mm, about 4.3 mm tall** (one per button; buy a mixed-height bag and pick what fits) | [AliExpress: 6x6 tactile](https://www.aliexpress.com/w/wholesale-6x6-tactile-switch.html) | <€2 for a bag |
| Resistors for a **button ladder** (e.g. 1 k, 2.2 k, 4.7 k, 10 k): all four buttons on **one** ADC pin, which saves three pins | from a resistor kit | — |

## 6. Power and network: pick one input per satellite

The socket for either option sits **low in the back of the electronics bay** (about 86 mm above the table), in a flat alcove set 5 mm into the round wall.
It is a **panel-mount socket on a short cable**, screwed to a thickened mounting plate, so the case takes the strain of plugging in, not the boards. Inside, it sits right next to the PoE splitter or USB-C trigger.
The cable hangs down the back to the table. Satellites stand with their back to a wall, so it isn't seen, and the speaker chamber stays a plain sealed box (about 0.40 L of speaker air, measured on the model).

### Option A: USB-C + Wi-Fi

| Part | Where | Price |
|---|---|---|
| **USB-C PD trigger board, set to 15 V** (asks the charger for 15 V) | [AliExpress: USB-C PD trigger](https://www.aliexpress.com/w/wholesale-usb-c-pd-trigger-15v.html) | about €2 |
| **Buck converter 15 V → 12 V**, 2 A or more (for example a Mini560 with fixed 12 V output) | [AliExpress: Mini560 12V](https://www.aliexpress.com/w/wholesale-mini560-12v.html) | about €2 |
| USB-C charger, **30 W or more** (any that offers 15 V, which all 30 W+ PD chargers do) | anything you have, or local | — |
| **USB-C panel-mount extension, 15–20 cm**: female socket with a 2-screw flange, cable to a male USB-C at the PD trigger | [AliExpress: USB-C panel mount screw](https://www.aliexpress.com/w/wholesale-usb-c-panel-mount-extension-screw.html) | about €3 |

Why 15 V and not 12 V straight from the charger: USB-C PD chargers always offer 5/9/15/20 V, but 12 V is optional and many don't have it. The amplifier's limit is about 14.4 V, so 15 V can't go to it directly.

### Option B: PoE+ with wired Ethernet

| Part | Where | Price |
|---|---|---|
| **PoE+ splitter module, IEEE 802.3at, 12 V ≥ 2 A output, data passed through** (choose a bare PCB version, about 50×30 mm, not the 100 mm long boxes) | [AliExpress: PoE splitter module 12V 802.3at](https://www.aliexpress.com/w/wholesale-poe-splitter-module-12v-802.3at.html) | about €8–12 |
| **RJ45 panel-mount extension, 15–20 cm**: female socket with a 2-screw flange, cable to a male RJ45 into the splitter | [AliExpress: RJ45 panel mount screw](https://www.aliexpress.com/w/wholesale-rj45-panel-mount-extension-cable-screw.html) | about €3 |
| Short flat RJ45 patch cable (10–15 cm), splitter → ESP32 board | [AliExpress: flat RJ45 15cm](https://www.aliexpress.com/w/wholesale-flat-ethernet-cable-15cm.html) | about €2 |
| A **PoE+ (802.3at)** port on the switch, or a PoE+ injector | your UniFi switch, or about €15 | — |

Plain PoE (802.3af) gives about 13 W, which barely covers the 10 W amplifier plus the electronics. **PoE+ gives 25 W.**

### Both options

| Part | Where | Price |
|---|---|---|
| **Buck converter 12 V → 5 V, 3 A** (powers the ESP32 board, LED ring and radar) | [AliExpress: Mini560 5V](https://www.aliexpress.com/w/wholesale-mini560-5v.html) | about €2 |
| Optional: ideal-diode OR board, if a satellite could ever have both inputs connected at once | [AliExpress: ideal diode module](https://www.aliexpress.com/w/wholesale-ideal-diode-module.html) | about €3 |

## 7. Optional environment sensors (I2C, on the ESP32 board)

Keep these **away from the amplifier and the ESP32**, near the side vents low in the electronics bay, or they will read warm.

| Part | Where | Price |
|---|---|---|
| VEML7700 ambient light (dims the LEDs at night, and tells the lighting curve when a room is actually dark) | [DigiKey DK: DFRobot SEN0228](https://www.digikey.dk/en/products/detail/dfrobot/SEN0228/7682232) · [AliExpress](https://www.aliexpress.com/w/wholesale-veml7700.html) | about 63 kr (DigiKey) |
| SHT40 temperature and humidity | [DigiKey DK search](https://www.digikey.dk/en/products/result?keywords=SHT40%20breakout) · [AliExpress](https://www.aliexpress.com/w/wholesale-sht40-module.html) | about €3–8 |
| SCD41 CO₂ (real photoacoustic CO₂, not estimated) | [DigiKey DK search](https://www.digikey.dk/en/products/result?keywords=SCD41) · [AliExpress](https://www.aliexpress.com/w/wholesale-scd41.html) | about €20–45 |

## 8. Case hardware

You already have these from the v1 case:

- M2/M3 heat-set inserts and screws
- **Black ABS** for the case and **white ABS** for the light band, printed with the same 100.47% X/Y shrink compensation as v1. Check the white once: colours can shrink slightly differently.
- The light band is **1.2 mm thick** (`diffuserThickness`): two or three perimeters, no infill, so it glows evenly instead of showing 24 dots.
- Silicone, for sealing the wire pass-through between the speaker chamber and the electronics

**Vents:** slots low in the electronics bay beside the SHT40, one set on each side for cross-flow, each with a
printed baffle 2–3 mm behind it so you see black plastic, not the board.

**Later materials (ideas, 2026-10-04).** The case gets split into an inner chassis (electronics, speaker seal,
mic gasket, LEDs) and an outer skin, so the skin can change without touching the electronics:
- **Base in metal:** CNC-turned aluminium (6061/6082), brushed or bead-blasted, then electrolytically coloured
  bronze anodising ("eloxering"); Cerakote bronze or clear anodise as fallbacks. A heavy base helps the bass
  and the stability. Rubber or felt feet; screwed into inserts; kept isolated from the electronics or grounded.
- **Wood:** veneer wrapped over the printed shell (solid wood moves and would let the speaker chamber leak), and a
  CNC'd wooden top with counterbored mic holes. Thin maple veneer (about 0.5 mm) over the light band glows when lit.
- **Keep metal away from the ESP32 antenna side** of the electronics bay, or Wi-Fi range suffers.
- Molded versions later: vacuum casting (10–50 units) or MJF nylon; injection molding only pays off at hundreds.

---

## Cost per satellite

| | € | DKK (≈ 7.46) |
|---|---|---|
| reSpeaker Flex without XIAO (check the price at OpenELAB) | about 45–50 | about 350 |
| Waveshare ESP32-S3-ETH | about 25 | about 190 |
| SB65 speaker | 33 | 250 |
| LD2450 radar | 10 | 75 |
| LED ring + 4 buttons + level shifter | 5 | 40 |
| 12 V → 5 V buck | 2 | 15 |
| Foam tape, share of polyfill, filament, patch wires | 8 | 60 |
| **Option A input** (PD trigger + 15→12 V buck + panel socket) | 7 | 50 |
| **Option B input** (PoE+ splitter + patch cable + panel socket) | 15 | 110 |
| **Total, option A (USB-C + Wi-Fi)** | **≈ 138** | **≈ 1,030** |
| **Total, option B (PoE+ + Ethernet)** | **≈ 146** | **≈ 1,090** |
| + light / temperature / CO₂ sensors | +10 to 55 | +75 to 410 |

Shipping comes on top. A combined Sound Imports + SmartHomeShop order covers the speaker side for several satellites at once.

---

## Check before ordering

Seeed's and Waveshare's documentation don't answer these. Check them in the wikis, on the boards, or with support:

1. **48 kHz I2S firmware for the Flex.** The current Niles speech path, with barge-in, runs the bus at 48 kHz. The Flex ships with "I2S firmware" at v3.2.1. One shop lists 16 kHz max, while the wiki mentions 16 and 48 kHz variants. v1's 48 kHz `.bin` is built for the USB 4-mic board and may not suit the Flex.
2. **The Flex's I2S/I2C header** on the version without XIAO: which pins, which voltage levels, and whether it is the I2S master (on v1 the XVF3800 is master).
3. **What the Flex's 12 V terminal feeds**: the amplifier only, or the board's own 3.3 V too. Either way the ESP32 board gets its own 5 V from the buck.
4. **Free pins on the ESP32-S3-ETH** once the W5500 has its SPI pins: they need to cover I2S (4), I2C (2), the radar UART (1–2), the LED data line (1) and the buttons (1 ADC pin with the resistor ladder, or 4 plain pins).
5. **The size of the PoE+ splitter module.** The model reserves about 50×30×16 mm at the back of the upper layer.
   The panel socket's body takes the back of the lower layer, so the 12 V → 5 V buck sits beside it.
6. **The panel-mount sockets' flange and screw spacing.** The model's port alcove in the electronics bay has a 16×14 mm opening and two screw holes 28 mm apart (`portWindowWidth`, `portWindowHeight`, `portScrewSpacing`). Measure the RJ45 and USB-C sockets and set those parameters. The USB-C socket may want its own opening size.
7. **Onboard status LEDs on the Flex**, and whether XVF GPO pins are broken out. v1 drives its light ring through the XVF.

## Niles work this needs

- **Firmware port:** from the XIAO to the ESP32-S3-ETH, with a new pin map. Re-tune echo cancellation (`SYS_DELAY`, mic gain, reference gain) for the new amp and speaker. Drive the WS2812 ring from the ESP32 instead of the XVF's LED ring.
- **Ethernet:** the W5500 driver (ESP-IDF's `esp_eth`) and the rule "Ethernet when the link is up, Wi-Fi otherwise". The audio and the Niles protocol are unchanged.
- **Radar:** an LD2450 UART parser on the ESP32. Send presence and target positions to Niles. Add a satellite presence source in `niles-presence`, so each room with a satellite reports who's there.
- **Voice + radar:** read DoA from the XVF3800 and match it to radar targets, to make speaker identification more reliable.
- **Buttons:** volume up/down change the satellite's volume locally and tell Niles. **Mute** mutes the mics in the XVF3800 (over I2C) and turns the light ring red until pressed again. **Action** starts listening without the wake word, or stops a ringing timer or alarm.
- **Case:** turn the "Niles v2" concept into printable parts: joints between the sections (tongue/groove and heat-set inserts as in v4), board mounting posts, a carrier for the four button switches under the top cap, internal cable routing and fillets, using measurements from the real parts.

## Build order

1. Order **one** set, with both power options' small parts, and verify the "Check before ordering" items on the real boards.
2. Get speech in and out at 48 kHz with barge-in (firmware port), on USB-C + Wi-Fi first.
3. Add Ethernet, then try the PoE+ splitter.
4. Add the LED ring and the radar.
5. Finish and print the case, then tune echo cancellation in the finished case. The case changes the acoustics.
6. Order the rest once the first unit works.
