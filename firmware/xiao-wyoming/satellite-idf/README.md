# niles satellite firmware (ESP-IDF)

The voice-satellite firmware for the **XIAO ESP32-S3** (paired with the
reSpeaker **XVF3800** doing AEC / beamforming / noise suppression over I2S).
ESP-IDF, native — chosen for the on-device wake word + barge-in (see
[[satellite-transport-decision]] in memory / the design discussion).

This **replaces** the earlier Arduino VAD sketch (`../niles_satellite/`). It's
built up in de-risking stages:

- **Stage 1 (now):** microWakeWord "Hey Jarvis" **detection only** — prints the
  probability so we confirm + tune the model on real hardware.
- **Stage 2:** on detection → open the Wyoming TCP stream to niles, capture +
  send the command.
- **Stage 3:** run the detector *during* playback (XVF3800 AEC) → **barge-in**
  ("Hey Jarvis" interrupts a reply/chime).
- **Persistent connection** → niles can push chimes/alarms/notifications to an
  idle satellite.

## Build, flash, monitor

Requires **ESP-IDF v5.x** (you installed the Espressif IDF VS Code extension).

**VS Code:** open this folder, set target **esp32s3** (status bar), then use
the extension's **Build → Flash → Monitor** buttons. Flash over the **XIAO
module's own USB-C port** (not the XVF3800 carrier board's).

**CLI equivalent:**
```bash
idf.py set-target esp32s3
idf.py build flash monitor      # Ctrl-] to exit monitor
```

The first build downloads `esp-tflite-micro` via the component manager
(declared in `main/idf_component.yml`) — give it a minute.

## Stage 1 output

- Quiet room → nothing (we only log when the 5-frame average > 0.30).
- Speech → `prob=… avg=…` lines.
- "Hey Jarvis" → `avg` should climb toward 1.0 and log `WAKE WORD DETECTED`.

## If it doesn't detect — iteration surface (in priority order)

1. **Build error** → likely an esp-tflite-micro API drift (e.g.
   `MicroResourceVariables::Create`) or an I2S v5 field name. Send me the error.
2. **`Invoke failed` / missing op / frozen prob** → the streaming op set
   (`MicroMutableOpResolver` in `main.cpp`) or `kNumResourceVars`. Reconcile
   against ESPHome's `micro_wake_word`.
3. **Random prob / no rise on "Hey Jarvis"** → the feature transform
   (`FEATURE_SCALE`/`FEATURE_DIV`) or `FrontendConfig` constants.
4. **Audio garbage** → I2S slot format (try `I2S_STD_MSB_SLOT_DEFAULT_CONFIG`
   instead of `PHILIPS`), bit width, or the BCLK/WS/DIN pins.

## Files

- `main/main.cpp` — Stage 1 app.
- `main/hey_jarvis.tflite` — the model, embedded via `EMBED_FILES`.
- `main/idf_component.yml` — pulls `espressif/esp-tflite-micro`.
- `sdkconfig.defaults` — esp32s3, 8 MB flash, OPI PSRAM, custom partitions.
- `partitions.csv` — 3 MB app partition (room for TFLM + model).

Swapping the wake word later (custom "niles" v2 model): replace
`main/hey_jarvis.tflite`, update the constants in `main.cpp`, rebuild.
