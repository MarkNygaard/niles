# niles satellite firmware (ESP-IDF)

The voice-satellite firmware for the **XIAO ESP32-S3** (paired with the
reSpeaker **XVF3800** doing AEC / beamforming / noise suppression over I2S).
ESP-IDF, native — chosen for the on-device wake word + barge-in (see
[[satellite-transport-decision]] in memory / the design discussion).

This **replaces** the earlier Arduino VAD sketch (`../niles_satellite/`). It's
built up in de-risking stages:

- **Stage 1 (done):** microWakeWord "nyles" detection — prints the probability
  so we confirm + tune the model on real hardware.
- **Stage 2 (done):** on detection → open the Wyoming TCP stream to niles,
  capture + send the command.
- **Stage 3 (done):** play niles' spoken reply back over the same socket.
- **Barge-in (next):** run the detector *during* playback (XVF3800 AEC) so
  "nyles" interrupts a reply/chime.
- **Persistent connection** → niles can push chimes/alarms/notifications to an
  idle satellite.

The wake word is a custom microWakeWord v2 model ("nyles", spelled for the TTS
pronunciation of *Niles*) trained on microwakeword.com.

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

## Tuning the wake word

The firmware logs a ~1 s heartbeat: `peak=… featmax=… maxprob=…`.

- `featmax` jumps when you speak (confirms the mic + features work).
- `maxprob` jumps when you say **"nyles"** — this drives `PROB_CUTOFF`.

Say "nyles" a few times, read the `maxprob` peak, then set `PROB_CUTOFF` in
`main.cpp` to ~60-70% of it — comfortably above the `maxprob` you see for
ambient noise and look-alikes (miles/files). It logs `WAKE WORD DETECTED` and
streams the command once `maxprob` crosses the cutoff.

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
