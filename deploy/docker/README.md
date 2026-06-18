# Niles container image

Multi-stage [`Dockerfile`](Dockerfile) that builds the `niles` binary into a
slim Debian runtime image. The image contains **only the binary** — no
config. `niles.toml` is mounted at runtime (see "Configuration" below), so
config changes never require an image rebuild.

## Building + publishing

Built and pushed to **GHCR** by [`.github/workflows/image.yml`](../../.github/workflows/image.yml):

- **Tag a release** — push a git tag `v*` (e.g. `v0.1.0`) → publishes
  `ghcr.io/marknygaard/niles:0.1.0` + `:latest` + `:sha-<sha>`.
- **Manual** — run the `image` workflow (Actions → image → Run workflow) on
  `main` → publishes `:latest` + `:sha-<sha>`.

No local Docker needed — the build runs in CI.

### GHCR visibility

GHCR packages are **private by default**. Either:

- make the package public (GitHub → Packages → niles → Package settings →
  Change visibility → Public), or
- create an image pull secret in your namespace and reference it:
  ```sh
  kubectl create secret docker-registry ghcr \
    --docker-server=ghcr.io \
    --docker-username=<github-user> \
    --docker-password=<a PAT with read:packages> \
    -n <namespace>
  ```
  then add `imagePullSecrets: [{ name: ghcr }]` to the pod spec.

## Configuration (runtime, not baked in)

The deployment mounts a ConfigMap at `/etc/niles/niles.toml` and runs
`niles serve --config /etc/niles/niles.toml`. **Set all config there** — in
your kustomize ConfigMap or Helm chart — not in the image. Secrets
(`NILES_MQTT_USERNAME` / `NILES_MQTT_PASSWORD`, `GROQ_API_KEY`) come from a
`niles-secrets` Secret via env vars.

`serve` runs the full stack, but with no satellite connected the Wyoming
voice server just sits idle — the lighting curve, room light-switches, and
morning routine all run regardless. `serve` does require `GROQ_API_KEY` to
start (it builds the LLM client eagerly); the TTS/STT URLs are only exercised
once a satellite speaks, so their values don't matter until then.

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

# Needed for serve to start even while voice is idle.
[stt]
api_key_env = "GROQ_API_KEY"
language = "en"
[llm]
api_key_env = "GROQ_API_KEY"

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
