# Ordering satellite v2

What to choose when ordering the mainboard (and, later, the top board), and what to buy besides.
Written 2026-10-08, updated 2026-10-09 for the round-9 mainboard (board v1, the first one made): DRC
0/0, parity clean, the four layout reviews clean, footprints and placement checked against JLC's own.
The files in `order/` are the ones to upload; re-export them after any board change.

## Where

**JLCPCB, PCB + PCBA.** The BOM is built on LCSC numbers, and JLCPCB assembles from the same
warehouse. Most placed passives are JLC *basic* parts (no feeder fee). About 50 of the 75 unique
part numbers are *extended*, at roughly $3 each per order: about $150 on top, worth it for the ICs and
the specific capacitors. PCBWay
turnkey is the fallback (SPEC §10 has its settings); nothing in the layout changes.

## PCB

| Setting | Choose | Why |
|---|---|---|
| Layers | 4 | |
| Dimensions | 103.8 × 102.2 mm (round, flat back with a small tab under J2) | Upload the Gerbers and the size fills itself in |
| Quantity | 5 | Assemble 3, keep 2 bare in case a fix is needed |
| Material | FR-4, TG155 (or higher) | |
| Thickness | 1.6 mm | J1, PS1 and J3 pin lengths assume it |
| Impedance control / stack-up | **JLC04161H-7628**, no impedance control | The USB and Ethernet lines are routed as plain 0.15–0.22 mm tracks, not coupled pairs, so on 7628 prepreg (≈0.21 mm L1–L2) they sit well off 90/100 Ω (Ethernet ≈ 140 Ω differential). At full-speed USB and 100BASE-TX over a few cm that doesn't matter, so impedance control would buy nothing |
| Outer copper | 1 oz | |
| Inner copper | **1 oz** (not the 0.5 oz default) | VIN area on L3, and the GND planes carry the amplifier current |
| Via covering | **Epoxy filled & capped** | Vias in the ESP32 (U1), the PD chip (U11) and the CO2 sensor (U9) exposed pads, and a few GND vias touching pad edges. Open vias there wick the solder away. **Add an order note:** "also fill and cap the 0.2 mm plated holes inside the U1 and U11 thermal pads" (they are footprint pad holes, so the drill file doesn't list them as vias) |
| Min via hole / diameter | 0.3 / 0.6 mm (routed vias); 0.2 mm holes exist inside U1/U11's pads | Pick the option that covers 0.2 mm holes |
| Min track / spacing | 0.15 / 0.15 mm (6/6 mil) | |
| Surface finish | **ENIG** | 0.4 mm-pitch PD chip, flat pads for the LEDs and QFNs |
| Solder mask / silk | Green / white | |
| Gold fingers, castellated holes | No | |
| Remove order number | Yes (no number) | There's no free spot marked for it on the silkscreen |
| Edge rails / fiducials | **Added by JLCPCB** | The board is round; the 3 board fiducials (FID1–3) stay |

## Assembly

| Setting | Choose |
|---|---|
| Type | Standard PCBA, **top side only** (nothing on the bottom) |
| Quantity | 3 |
| Tooling holes | Added by JLCPCB |
| J3 (Samtec ESQ) | Samtec's ESQ catalog lists its processing as lead wave only. Ask JLC whether their lead-free THT process is fine for ESQ-110-33-G-D; if not, consign J3 and hand-solder it |
| Through-hole parts | **Yes, solder them**: J1 magjack, J3 Samtec socket, J4 amp power, J6/J8 radar JST-ZH. **PS1 is left off** ("Do not place"): soldered by hand at home after each board passes its USB-C bring-up, see below |
| Not fitted | On purpose: R1/R2 (Ag5412 voltage trim), R70–R73 (I2S/MCLK swap links), C99 (spare VUSB bulk cap), C119 (second VBUS cap, see SPEC) |
| Confirm part placement | Yes. `cpl.csv` is already turned and shifted onto JLC's own footprints (see *Footprints checked against JLC* below), so in the preview every part should sit square on its pads with pin 1 on the board's pin-1 mark. Look hardest at the four 4-pad switches' row direction (SW1/SW2), J3 and PS1 (no JLC footprint to check against), and anything the preview shows off its pads: say so in the order note rather than accept a correction you don't understand |

### Not in JLC's stock: Global Sourcing

Ordered 2026-10-12 from DigiKey through Global Sourcing: J1 × 3, J2 × 5, J3 × 3, L5 × 4 (the spares cover machine pick-up losses on the SMD parts).

Order these through **Global Sourcing** (JLC buys them from DigiKey/Mouser, 9–15 days, can't be
cancelled once paid). Or buy them yourself and send them as consigned parts.

| Ref | Part | From |
|---|---|---|
| J1 | Bel Fuse 0826-1X1T-GJ-F (PoE+ magjack), JLC C6377355 | DigiKey |
| PS1 | Silvertel Ag5412 (PoE+ PD module, 12 V), JLC C40163463. **Not via JLC** (2026-10-12: no stock anywhere Global Sourcing searches): bought from Mouser (480-AG5412, back-ordered, expected 23/11/2026) and hand-soldered | Mouser / DigiKey / Farnell |
| L5 | Würth 74439346047, JLC C6827938 | DigiKey |
| J3 | Samtec ESQ-110-33-G-D, JLC C3322059 | Samtec / DigiKey |

The board's LCSC fields carry those JLC codes, so the BOM matches them; JLC shows little or no stock
for them and offers Global Sourcing at that step. Order 3 of each plus one spare J1 and PS1 for the bench.
Low stock to re-check on the day (2026-10-08, enough for 3 boards): C2 C178349 (104), L6 C2041388 (158),
J2 C3020560 (135 on 2026-10-09), U11 C22438973 (336), R14 C138009 (786).

## Files to upload

Generated 2026-10-09 from the real project into `order/` (`export_order.py`): 159 placed parts, every one with an LCSC number. Re-run it after any board change:

- `gerbers.zip`: F/B copper, In1/In2, masks, silks, paste, Edge.Cuts, and Excellon drills (PTH and NPTH separate)
- `bom.csv`: Comment, Designator, Footprint, LCSC Part #, fitted parts only
- `cpl.csv`: Designator, Mid X, Mid Y, Layer, Rotation, corrected onto JLC's footprints (`jlc_corrections.json`)

## Footprints checked against JLC (no parts on hand, so no 1:1 print test)

2026-10-09. JLC places each part from the EasyEDA footprint behind its LCSC number, so that footprint, not the
part in hand, is the reference (`jlc_fp_check.py`, footprints fetched with `easyeda2kicad`):

- **Pads:** 157 of 159 parts have a JLC footprint. Every one matches ours to ≤ 0.2 mm pad-centre to
  pad-centre after the best 90° turn, apart from three explained differences: U1's thermal pad is split into
  13 squares here and 9 at JLC; U11's power pins 20–22, 23–25, 32–33 and 34–35 are one wide pad here (TI's
  REF0038A land pattern) and single pins at JLC, each inside our wide pad and the same net; SW1/SW2 are
  numbered 1,1,2,2 here and 1–4 at JLC, with the internally joined pairs on the same rows in both. The custom
  Bel magjack footprint matches JLC's to 0.007 mm.
- **Pinouts:** all 23 multi-pin parts' pin names match JLC's symbols number for number (ESP32 module, TPS25730,
  W5500, TPS2121, bucks, LDO, opto, ESD, TVS, bridges, crystal, radar connectors, magjack).
- **Polarity:** C2 (+ on JLC's chamfered side), D1/D11 (cathode band) and D2 (cathode arrow) put JLC's mark on
  our pad 1, which is + / cathode in the circuit.
- **Placement:** 16 parts are drawn turned or with a different origin at JLC: D10, U3, U4, U10 by 90°; U2,
  U5, U6, U7, U12, J4, J5, J6 by 180° (J1 too, plus 8.6 mm); J2, J8, U1 shifted 1.3–3.6 mm. The raw KiCad
  positions would have put 19 parts off their pads. `jlc_cpl_verify.py` places JLC's footprint where the new
  `cpl.csv` says and finds every pad within 0.2 mm of ours (bar the three explained above).
- **Not checkable this way:** J3 (plain 2×10 socket, no polarity of its own) and PS1 (footprint checked
  against the Ag5400 datasheet earlier). They are through-hole and hand-placed by JLC.

## Hardware besides the boards

Per satellite (prototype case, Fusion "Niles v2"; SPEC §6 has the joints and the assembly order):

| Qty | Part | Where |
|---|---|---|
| 10 | M3 heat-set insert, **short (4 mm long at most)**, for a Ø4.0 mm hole | 4 in the lid's board bosses (the 5.5 mm holes can't be deeper without opening the chamber), 3 in the tops and 3 in the bottoms of the chamber's tie pillars |
| 4 | M3 × 25 pan head screw | Through the Flex, the bay's spacer tubes and the mainboard into the lid's inserts (this also clamps the bay) |
| 3 | M3 × 25 countersunk screw (ISO 10642 / DIN 7991) | Base to chamber, from underneath |
| 3 | M3 × 8 countersunk screw | Lid to chamber |
| 4 | M3 × 12 pan head screw + M3 nyloc nut | Speaker to the chamber floor |
| 3 | M2 × 6 self-tapping screw for plastic | LED ring board to its pads (Ø1.7 pilot holes) |
| 3 | M2 × 4 self-tapping screw for plastic | Mic array to the top cap (Ø1.7 pilot holes, 3 mm deep) |
| 1 | Die-cut 0.8 mm adhesive foam, Ø52 with a window over the LTR-303 and Ø2.2 holes at (±12, ±12) | Touch disc to the top cap |
| 4 | Mic gasket, OD 4 / ID 2 mm, cut from 0.8 mm closed-cell foam or silicone | Flex mics to the top cap |
| 1 | Neutral-cure silicone | Chamber-lid seam, speaker wire hole, under the speaker flange (or a 1 mm foam gasket there) |
| 1 | Glue for the band (CA or 2-part epoxy) | Diffuser band to the bay's top edge |
| 1 | 2 × 22 AWG speaker wire, about 150 mm, with a JST PH 2.0 2-pin plug | Speaker to the Flex's speaker connector, through the lid hole and the bay's wall channel |
| 1 | JST SH 6-pin cable, straight-through, about 100 mm | LED ring board J2 to the touch disc |
| 3 | Rubber feet, Ø8–10 | Under the base, over the screw heads |
| 1 | JST ZH 4-pin cable, wired **by function**: J6 pin 1 (silk "5V") to the module's contact marked 5V, then Rx, Tx, GND in order | J6 to the LD2450. Hi-Link's manual gives the socket order (5V, Rx, Tx, GND) but not which end is pin 1, so check the cable with a meter (5 V lands on 5 V, GND on GND) before the first power-up |
| 1 | JST ZH 5-pin to Dupont, straight-through | J8 to the LD2410C (solder pins on it, or buy the LD2410C-P) |
| 1 | JST SH 10-pin cable, **straight-through** | J5 to the top board. A reversed one is harmless for the rails but swaps LED data onto the button line |
