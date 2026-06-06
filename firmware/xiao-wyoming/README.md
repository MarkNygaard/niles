# XIAO Wyoming satellite firmware

Custom firmware for the **reSpeaker XVF3800 + XIAO ESP32-S3** satellite:
on-device wake word + **direct TCP streaming of raw PCM frames to the
`niles-wyoming` server** (the Wyoming protocol). This is the settled
satellite architecture for niles.

## Why this approach (decided 2026-06-06)

The goal is low latency and high response quality. The fast pattern —
the same one Alexa/AVS uses — is **streaming small raw-PCM frames
continuously as the person speaks** (mono 16 kHz 16-bit LPCM, chunked,
on-device wake word, server-side endpointing). Recognition overlaps
with speech; that overlap is what makes it feel instant. `niles-wyoming`
already implements exactly this streaming-over-TCP model.

Alternatives were evaluated and rejected:

| Option | Why not |
| --- | --- |
| **ESPHome `voice_assistant`** | Connects **only to Home Assistant** (ESPHome native API); cannot target an arbitrary server. niles is a clean HA replacement — no HA in the path. See [`../esphome/`](../esphome/). |
| **MQTT (Seeed's example sends a WAV)** | Near worst-case latency: WAV-at-the-end is batch (must finish recording + encode + transfer *before* STT starts), plus an extra broker hop (satellite → Mosquitto → niles). MQTT stays on the device/Z2M layer, not audio. |
| **`wyoming-satellite`** | Deprecated since 2025 (replaced by "Linux Voice Assistant", ESPHome-protocol); Linux/Pi-only anyway. |
| **USB-mic mode** | Plugging the XVF3800 into the niles box as a plain USB mic is zero-firmware but tethers to one machine — incompatible with multiple wireless room satellites. |

## Architecture

```
XVF3800 (XMOS DSP)            XIAO ESP32-S3                    niles
─────────────────            ──────────────────────           ──────────────
beamforming / AEC   ──I2S──► capture (AudioTools)             niles-wyoming
noise suppression           ► wake word (microWakeWord)        (TCP :10300)
16 kHz, 2ch, 32-bit         ► on wake: downmix → mono           │
                              16 kHz 16-bit PCM                  ▼
                            ► TCP stream (Wyoming framing) ───► session accumulator
                                                                 ▼
                                                                Groq Whisper STT
                                                                 ▼
                                                                Tier 0 / Tier 1 → act
```

- **XMOS side:** flash the stock Seeed **I2S DFU firmware**
  (`respeaker_xvf3800_i2s_dfu_firmware_v1.0.x.bin`) → XVF3800 in
  INT-Device/I2S mode (2-channel, 32-bit, 16 kHz). Stock image, no
  custom dev.
- **XIAO side:** custom Arduino/ESP-IDF firmware. Capture base is
  Seeed's I2S test sketch (AudioTools `I2SStream`/`I2SConfig`). Wake
  word is **microWakeWord** (INT8 TFLite, custom "niles" model trained
  at microwakeword.com — the accessible path to a custom wake word;
  Espressif ESP-SR/WakeNet is more robust but custom wake words are
  gated behind a 500+ speaker corpus or a paid service).
- **niles side:** `niles-wyoming` already accepts the stream. The
  `[satellites]` peer-IP → room mapping tags which room a stream came
  from.

## Wyoming wire format (what the sketch must emit)

`niles-wyoming` reads newline-terminated JSON event headers, each
optionally followed by exactly `payload_length` bytes of binary PCM.
No JSON library is needed on the device — two of the three headers are
constant strings; only the chunk header varies (one `snprintf`).

```
{"type":"audio-start","data":{"rate":16000,"width":2,"channels":1}}\n
{"type":"audio-chunk","payload_length":1024}\n        ← then exactly 1024 bytes
   …repeat audio-chunk per frame…                        mono 16-bit LE PCM
{"type":"audio-stop"}\n
```

- `rate` 16000, `width` 2 (16-bit), `channels` 1 (mono).
- A 1024-byte payload = 512 samples = 32 ms/frame at 16 kHz.
- `audio-stop` is what makes niles finalize the session and transcribe.

## Audio conversion in the sketch

The XVF3800 I2S output is **2-channel 32-bit**; niles wants **mono
16-bit 16 kHz**. Per sample: take the processed-voice channel, keep the
top 16 bits (`>> 16`). The sample rate already matches (16 kHz), so no
resampling.

Two values to pull from
[Seeed's I2S example](https://wiki.seeedstudio.com/respeaker_xvf3800_xiao_i2s/)
rather than guess:
- the exact **I2S pin config** for the XIAO + XVF3800, and
- **which I2S channel** carries the processed/beamformed voice.

---

## Milestone 1 — dumbest possible end-to-end (no wake word)

**Goal:** prove the listen→act loop on real hardware. One satellite,
no wake word, no speak-back. This verifies the entire input chain
(transport → STT → intent → device control) for the first time.

**niles side — nothing to build, just config + run:**
- `[wyoming] bind_address = "0.0.0.0:10300"` (already the default).
- Optional: a `[satellites]` entry mapping the XIAO's LAN IP → a room.
- Run `niles serve`; watch logs for the transcript + intent + dispatch.

**XIAO sketch — the only thing built:** graft onto Seeed's I2S sketch.
On boot or button press:
1. Connect WiFi, open TCP to `<niles-ip>:10300`.
2. Send the `audio-start` header.
3. Loop for a fixed window (~5 s): read I2S → convert to mono 16-bit →
   send `audio-chunk` header + PCM bytes.
4. Send `audio-stop`.

**Success criterion:** speak *"turn on the office light"* during the
window → niles logs the transcript + Tier 0 intent + the MQTT
dispatch → the bulb turns on.

## Later layers (deferred, in order)

1. **On-device wake word** — microWakeWord "niles" model, so the
   satellite is silent until triggered (no streaming the whole house).
2. **Continuous / VAD-bounded capture** — replace the fixed window with
   wake-triggered start + silence-detected stop.
3. **Speak-back (output)** — Piper TTS → satellite speaker. Gated on a
   confirmed-working speaker (see the project notes; the XVF3800 mic
   arrived 2026-06-06, speaker still unverified).
4. **Multi-room** — replicate the proven satellite to other rooms;
   `[satellites]` already maps each peer IP to a room.

## References

- [Seeed XVF3800 + XIAO I2S test](https://wiki.seeedstudio.com/respeaker_xvf3800_xiao_i2s/)
- [Seeed XVF3800 getting started](https://wiki.seeedstudio.com/respeaker_xvf3800_xiao_getting_started/)
- [microWakeWord (custom wake-word training)](https://microwakeword.com/)
- [Espressif ESP-SR / WakeNet](https://github.com/espressif/esp-sr)
- `niles-wyoming` crate — the server side of the wire format above.
