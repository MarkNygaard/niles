# syntax=docker/dockerfile:1
#
# Multi-stage build for the `niles` binary. Built + pushed to GHCR by
# .github/workflows/image.yml; consumed by the kustomize manifests under
# deploy/kustomize/. See deploy/docker/README.md.

# ---- builder ----------------------------------------------------------
FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
# Cache the cargo registry + target dir across builds (BuildKit cache
# mounts). The binary must be copied OUT of the cache mount within the
# same RUN, since cache-mount contents aren't part of the image layer.
#
# niles-recognition depends on `ort` (onnxruntime) with the
# `download-binaries` feature, so this build pulls the ORT lib from the
# network. `ort` is `load-dynamic`, so the runtime image does NOT need
# the shared library unless speaker recognition is configured (it isn't
# for lighting / switch / voice-idle deployments).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p niles-bin \
 && cp target/release/niles /usr/local/bin/niles

# ---- runtime ----------------------------------------------------------
FROM debian:bookworm-slim AS runtime
# ca-certificates: TLS roots for outbound HTTPS (Groq, etc.).
# chrono-tz bundles the IANA database, so no tzdata package is needed for
# `home.timezone` resolution.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --user-group niles
COPY --from=builder /usr/local/bin/niles /usr/local/bin/niles
USER niles
# 8080 = HTTP API + /healthz · 10300 = Wyoming voice (idle without a satellite).
EXPOSE 8080 10300
ENTRYPOINT ["niles"]
CMD ["serve", "--config", "/etc/niles/niles.toml"]
