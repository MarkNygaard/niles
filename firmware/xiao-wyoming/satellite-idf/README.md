# niles satellite firmware (ESP-IDF)

The voice-satellite firmware for the **XIAO ESP32-S3** (paired with the
reSpeaker **XVF3800** doing AEC / beamforming / noise suppression over I2S).
ESP-IDF, native — chosen for the on-device wake word + barge-in (see
[[satellite-transport-decision]] in memory / the design discussion).

This **replaces** the earlier Arduino VAD sketch (`../niles_satellite/`). To set
up a new satellite, go to [Set up a satellite](#set-up-a-satellite). It was
built up in de-risking stages:

- **Stage 1 (done):** microWakeWord "nyles" detection — prints the probability
  so we confirm + tune the model on real hardware.
- **Stage 2 (done):** on detection → open the Wyoming TCP stream to niles,
  capture + send the command.
- **Stage 3 (done):** play niles' spoken reply back over the same socket.
- **Niles calls in (done):** the satellite listens on `:10301`, so niles can
  push a timer chime or a notification to an idle satellite.
- **Barge-in (done, 2026-09-23):** I2S runs full duplex and playback
  has its own task, so the detector keeps running while the satellite talks.
  "Niles" over a chime or an answer cuts it off and starts a new command. The
  XVF3800's echo cancellation is what makes this work: on hardware a chime
  reads ~3000 peak for its first two seconds while AEC converges, then ~400,
  which is room level. The heartbeat's `play=1` marks seconds with the speaker on.
- **48 kHz bus (2026-10-03):** the XVF3800 now runs Seeed's
  `i2s_master_v1.0.9_48k` image and clocks the bus at 48 kHz as master; the
  ESP32 is the slave, decimates the microphone to 16 kHz (31-tap low-pass) and
  plays at 48 kHz.

The wake word is a custom microWakeWord v2 model ("nyles", spelled for the TTS
pronunciation of *Niles*) trained on microwakeword.com.

## Set up a satellite

What you need: a **reSpeaker XVF3800 with the XIAO ESP32-S3** (Seeed's
"ReSpeaker XVF3800 USB 4-Mic Array" with the XIAO on its back), a speaker on its
3.5 mm jack or speaker terminal, a USB-C data cable, and Niles running
somewhere on the same network.

The board has **two USB-C ports**. The one next to the 3.5 mm headphone jack
is the XMOS side (the microphone chip); the other is the XIAO's (the ESP32 that
runs this firmware). Each step below says which one.

### 1. Put the XVF3800 in I2S mode (once per board)

The reSpeaker ships with USB firmware, which doesn't send audio to the XIAO.
This firmware needs one specific Seeed image: **`respeaker_xvf3800_i2s_master_v1.0.9_48k.bin`**,
from `xmos_firmwares/i2s/` in Seeed's
[reSpeaker_XVF3800_USB_4MIC_ARRAY](https://github.com/respeaker/reSpeaker_XVF3800_USB_4MIC_ARRAY)
repository. In it the XVF3800 is the I2S **master at 48 kHz** and the XIAO the
slave; the firmware decimates the microphone to 16 kHz itself. Seeed's other I2S
images (the generic `i2s_dfu_firmware`, or anything at 16 kHz) don't clock the
bus this way, and the satellite will hear nothing with them.

Flash it once, through the **XMOS-side port** (next to the headphone jack),
with `dfu-util`:

```bash
dfu-util -l                      # should list [2886:001a] ... name="reSpeaker DFU Upgrade"
dfu-util -R -e -a 1 -D respeaker_xvf3800_i2s_master_v1.0.9_48k.bin
```

- On Linux, run both with `sudo`.
- On Windows, if `dfu-util -l` reports `LIBUSB_ERROR_NOT_SUPPORTED` or lists
  nothing, use [Zadig](https://zadig.akeo.ie/) to switch the "reSpeaker DFU"
  device to the WinUSB driver and try again.
- If the board doesn't show up at all (for instance because it already has
  another I2S image), start it in **safe mode** first: unplug it, hold **Mute**,
  plug it back in, and release when the red LED blinks. Then flash. Safe mode
  is also how to recover a board that misbehaves later.

Seeed's [XVF3800 with XIAO](https://wiki.seeedstudio.com/respeaker_xvf_3800_xiaozhi/)
guide has more on `dfu-util`. The firmware sets the XVF3800's echo-cancellation
profile itself at every boot (mic gain, `SYS_DELAY` −64, routes), so nothing
else needs configuring on the XMOS side.

### 2. Tell the firmware your Wi-Fi and where Niles is

Copy `main/secrets.example.h` to `main/secrets.h` (it is git-ignored, so your
password stays out of the repo) and fill it in:

```c
#define WIFI_SSID "your network"
#define WIFI_PASS "your password"
#define NILES_HOST "192.168.1.20"   // the machine running Niles
#define NILES_PORT 10300
```

- The ESP32-S3 only does **2.4 GHz** Wi-Fi. A network that is 5 GHz only
  won't connect.
- `NILES_HOST` must be an **IP address**, not a host name: the firmware
  doesn't look names up. On Kubernetes it is the external IP of the service
  that exposes Niles's port 10300.

### 3. Build and flash

Install **ESP-IDF 6.0.1**: that's the version the working satellites are built
with (with `esp-tflite-micro` 1.3.7, pinned in `main/idf_component.yml`). Older
IDF releases, 5.x included, aren't tested. The easiest way is the Espressif IDF
extension for VS Code, which installs the toolchain for you.

**VS Code:** open this `satellite-idf` folder, set the target to **esp32s3**
(status bar), then use the extension's **Build → Flash → Monitor** buttons.

**Command line:**
```bash
idf.py set-target esp32s3
idf.py build flash monitor      # Ctrl-] leaves the monitor
```

Flash through the **XIAO's port**, not the XMOS-side one (which doesn't show
up as a serial port while the XVF3800 runs the I2S firmware). If no serial
port appears, put the XIAO in its bootloader: hold **B**, tap **R**, release
**B**, then flash again. The first build downloads `esp-tflite-micro` (declared
in `main/idf_component.yml`), so give it a minute.

In the monitor you should see `wifi connecting to '…'`, then
`wifi connected, IP=…` (note that address for the next steps), then a heartbeat
about once a second (`peak=… featmax=… maxprob=… maxavg=…`).

### 4. Let the satellite and Niles reach each other

- Give the satellite a **fixed IP address**: a DHCP reservation in your router.
  Niles knows which room a satellite stands in by its address.
- The satellite connects to Niles on **TCP 10300** when it hears the wake word,
  and Niles connects to the satellite on **TCP 10301** to play timers and
  announcements. If the satellite sits on a separate network or VLAN, allow
  both directions through the firewall.

### 5. Add it in Niles

In the Niles app, open **Settings → Satellites**, give it a name (e.g.
`kitchen`), its IP address and its room, and restart Niles. A satellite that
isn't listed still works. It just can't tell which room "turn the lights off"
means.

### 6. Try it

Say **"Niles"**, then a command: *"Niles, what time is it?"* The ring lights up
while it listens, and the answer plays from the speaker. If it doesn't react,
watch `maxprob` in the monitor while you say "Niles" and see
[Tuning the wake word](#tuning-the-wake-word). The committed cutoff was tuned on
one satellite, and another room or voice may need a small change.

## Tuning the wake word

The firmware logs a ~1 s heartbeat: `peak=… featmax=… maxprob=… maxavg=…`.

- `featmax` jumps when you speak (confirms the mic + features work).
- `maxprob` is the highest single detector output that second, and `maxavg`
  the highest average over five outputs (~150 ms). **`maxavg` is what decides**:
  the satellite wakes when it reaches `AVG_CUTOFF` (0.75) in `main.cpp`.

The model was trained on 13,700 synthetic "nyles" plus real recordings from one
household and its television. In another home:

- **"Niles" is missed:** say it a few times at normal distance and read
  `maxavg`. If real ones land at, say, 0.65–0.80, lower `AVG_CUTOFF` to just
  under the weakest (0.6 here).
- **It wakes on its own** (a TV, a podcast): note `maxavg` on those false wakes
  and raise `AVG_CUTOFF` until it sits between them and your real ones. If the
  two overlap, the model needs retraining with that room's recordings (see
  below), not a threshold.

It logs `WAKE WORD DETECTED` and streams the command when it wakes. Rebuild
and flash after changing the cutoff.

## If it doesn't detect — iteration surface (in priority order)

1. **Build error** → likely an esp-tflite-micro API drift (e.g.
   `MicroResourceVariables::Create`) or an I2S v5 field name. Send me the error.
2. **`Invoke failed` / missing op / frozen prob** → the streaming op set
   (`MicroMutableOpResolver` in `main.cpp`) or `kNumResourceVars`. Reconcile
   against ESPHome's `micro_wake_word`.
3. **Random prob / no rise on "nyles"** → the feature transform
   (`FEATURE_SCALE`/`FEATURE_DIV`) or `FrontendConfig` constants.
4. **Audio garbage** → I2S slot format (try `I2S_STD_MSB_SLOT_DEFAULT_CONFIG`
   instead of `PHILIPS`), bit width, or the BCLK/WS/DIN pins.

## Files

- `main/main.cpp` — wake → stream → reply app.
- `main/nyles.tflite` — the wake-word model, embedded via `EMBED_FILES`.
- `main/nyles.json` — the model's microWakeWord metadata (cutoff, window).
- `main/idf_component.yml` — pulls `espressif/esp-tflite-micro`.
- `sdkconfig.defaults` — esp32s3, 8 MB flash, OPI PSRAM, custom partitions.
- `partitions.csv` — 3 MB app partition (room for TFLM + model).

Retraining the wake word: train a new v2 model on microwakeword.com, replace
`main/nyles.tflite` (+ `nyles.json`), retune `PROB_CUTOFF` in `main.cpp` from
the heartbeat (see "Tuning the wake word"), rebuild.
