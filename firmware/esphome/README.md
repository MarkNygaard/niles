# ESPHome satellite firmware — evaluated, not used

> **Decision 2026-06-06: ESPHome is NOT the niles satellite path.**
> Research confirmed ESPHome's `voice_assistant` component connects
> **only to Home Assistant** (over ESPHome's native API) — it cannot
> stream to an arbitrary Wyoming server. Since niles is a clean HA
> replacement, ESPHome would reintroduce Home Assistant as a bridge,
> which defeats the purpose.
>
> The settled approach is custom firmware that streams the Wyoming
> protocol directly to `niles-wyoming` — see
> [`../xiao-wyoming/`](../xiao-wyoming/).

This directory is kept as a marker of the evaluation. ESPHome remains
the easiest path *if you run Home Assistant* — but niles does not, so
it isn't applicable here.

What made ESPHome tempting (and what we give up by not using it):
- Stock, flashable firmware with great multi-device tooling (OTA,
  dashboard).
- Built-in on-device wake word (microWakeWord) — which the
  `xiao-wyoming` firmware reuses the *model* from, just running it in
  our own firmware instead of inside ESPHome.

The one-time cost of the chosen path (custom firmware) buys
HA-independence + direct, low-latency streaming to niles.
