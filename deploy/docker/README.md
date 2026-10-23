# Niles container image

Multi-stage [`Dockerfile`](Dockerfile) that builds the `niles` binary into a
slim Debian runtime image. The image contains **only the binary** — no
config. Settings live in Postgres and are changed from the app (see
"Configuration" below), so config changes never require an image rebuild.

## Building + publishing

Built and pushed to **GHCR** by [`.github/workflows/image.yml`](../../.github/workflows/image.yml):

- **A merge into `main`** publishes `:latest` and `:sha-<sha>`, and the
  version (`:0.4.0`) when what was merged is a release.
- **Manual**: run the `image` workflow (Actions → image → Run workflow),
  which publishes `:sha-<sha>`, and `:latest` from `main`.

The image is `linux/amd64` only. CI's `compose (up + healthz)` job also
builds it on every pull request and starts it with
[`deploy/compose`](../compose/).

## Speaker recognition

Two files the Rust build does not produce.

**ONNX Runtime** is baked in already. `ort` is built `load-dynamic`, so it
opens `libonnxruntime.so` at *runtime* and panics if it cannot — a runtime
image without it does not quietly run with recognition off, it dies the
moment `[recognition] enabled` is true. The version is pinned to what `ort`
asks for; changing the `ort` dependency means checking `ORT_VERSION` and
`ORT_SHA256` against it.

**The ECAPA model** is not, because it does not exist until somebody makes
it. Once:

```sh
pip install torch speechbrain onnx onnxscript
python scripts/export-ecapa-onnx.py -o ecapa.onnx
```

It takes a few minutes and produces one file of about **80 MB**. `torch`
writes the weights to a sidecar `.onnx.data` when they are large; the
script folds them back in, because `model_path` names a file and a graph
whose weights went missing fails at load with nothing pointing at the
cause.

The script prints the size and sha256. Attach `ecapa.onnx` to a GitHub
release, then set two **repository variables** (Settings → Secrets and
variables → Actions → Variables):

| Variable | Value |
|---|---|
| `ECAPA_URL` | the release asset's download URL |
| `ECAPA_SHA256` | the checksum the script printed |

Variables rather than secrets: neither is secret, and a secret would be
masked out of the build log at exactly the moment somebody is working out
why a build failed.

The next image then carries the model at
`/usr/local/share/niles/ecapa.onnx`. Point config at it:

```toml
[recognition]
enabled = true
model_path = "/usr/local/share/niles/ecapa.onnx"
```

With the variables unset — the normal case — the build is unchanged and the
image ships without a model.

### GHCR visibility

The package `ghcr.io/marknygaard/niles` is public, so pulling it needs no
login.

## Configuration (runtime, not baked in)

The image runs `niles serve` with no config file. Give it
`NILES_DATABASE_URL` (a Postgres connection string) and `NILES_SECRET_KEY`
(base64 of 32 bytes, which seals the keys typed into the app). Most of the
rest is set from the app's Settings and kept in the database: integrations,
the lighting curve, the providers.

A fresh install also needs, before its first start, the MQTT broker's
address and login and the keys for hearing (ElevenLabs) and thinking
(Cerebras): `serve` connects to the broker and builds both clients before
the API is up, and exits without them. Mount a small `niles.toml` with the
broker's address and the `*_env` names of the variables holding the rest,
and run `niles serve --config /etc/niles/niles.toml`.
[`deploy/compose/niles.toml`](../compose/niles.toml) is one. What the app
saves is layered on top of the file.

With no satellite connected the Wyoming voice server sits idle, and the
lighting curve, room light-switches and morning routine all run regardless.
Without a reachable broker, `serve` exits at its first curve tick, a minute
after starting.

### Sample config — circadian + switches + morning (no voice yet)

```toml
[home]
name = "Aarhus"
latitude = 56.1572
longitude = 10.2107
timezone = "Europe/Copenhagen"
locale = "da_DK"

[mqtt]
host = "mosquitto.home-automation.svc.cluster.local"
port = 1883
username_env = "NILES_MQTT_USERNAME"
password_env = "NILES_MQTT_PASSWORD"

[api]
bind_address = "0.0.0.0:8080"

# Needed for serve to start even while voice is idle, unless the keys
# are set in the app instead.
[stt]
api_key_env = "ELEVENLABS_API_KEY"
language = "en"
[llm]
api_key_env = "CEREBRAS_API_KEY"

# WLED strips are declared in config (not MQTT-discovered). Each entry's
# `topic` is that strip's base MQTT topic. Required for the curve + switches
# to drive them.
[[wled.devices]]
name = "living_room/ceiling"
topic = "wled/living_room_ceiling"
# [[wled.devices]]
# name = "living_room/tv_light"
# topic = "wled/living_room_tv"

[lighting]
morning_start = "05:45"
morning_end   = "06:30"
sunset_start  = "21:30"
sunset_end    = "23:00"
night_floor_brightness = 15
daytime_brightness     = 100

# Morning routine: turn these lights ON at morning_start on these days,
# then hand them to the curve. Device ids are <room>/<device>; prefix WLED
# strips with `wled:`.
[lighting.morning_routine]
fire_days = ["mon", "tue", "wed", "thu", "fri"]
target_devices = ["wled:living_room/ceiling"]

[[lighting.color_temp_anchors]]
time = "00:00"
kelvin = 2000
[[lighting.color_temp_anchors]]
time = "06:30"
kelvin = 2700
[[lighting.color_temp_anchors]]
time = "12:00"
kelvin = 4500
[[lighting.color_temp_anchors]]
time = "17:00"
kelvin = 3500
[[lighting.color_temp_anchors]]
time = "23:00"
kelvin = 2000
```

Notes:
- **Switches need no config** — any Zigbee button (Hue dimmer or 1-button
  Smart Button) controls the lights in its own room by convention.
- The curve only adjusts lights that are already **on**, and skips any in
  manual mode or listed under `[ambient_lights]`.
