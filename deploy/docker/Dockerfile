# syntax=docker/dockerfile:1
#
# Multi-stage build for the `niles` binary. Built + pushed to GHCR by
# .github/workflows/image.yml; consumed by the kustomize manifests under
# deploy/kustomize/. See deploy/docker/README.md.

# ---- web bundle -------------------------------------------------------
# The config UI is bundled separately and embedded into the binary by the
# `ui` feature. Kept in its own stage so a Rust-only change doesn't
# reinstall node_modules, and so the Rust builder needs no JS toolchain.
FROM oven/bun:1 AS web
WORKDIR /web
COPY web/package.json web/bun.lock* ./
RUN bun install --frozen-lockfile
COPY web/ ./
RUN bun run build

# ---- builder ----------------------------------------------------------
FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
COPY --from=web /web/dist ./web/dist
# Cache the cargo registry + target dir across builds (BuildKit cache
# mounts). The binary must be copied OUT of the cache mount within the
# same RUN, since cache-mount contents aren't part of the image layer.
#
# niles-recognition depends on `ort` (onnxruntime) with the
# `download-binaries` feature, so this build pulls the ORT lib from the
# network. `ort` is `load-dynamic`, so the shared library it opens at
# runtime is not this one — the `deps` stage below fetches that.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p niles-bin --features ui \
 && cp target/release/niles /usr/local/bin/niles

# ---- onnx runtime + speaker model -------------------------------------
# Speaker recognition needs two files the Rust build does not produce.
#
# `ort` is built `load-dynamic`, which means it opens `libonnxruntime.so`
# at *runtime* and panics if it cannot. A runtime image without it does
# not degrade to "recognition off" — it dies the moment `[recognition]
# enabled` is true. Pinned to what ort 2.0-rc.12 asks for (`api-24` is
# onnxruntime 1.24.2); on a mismatch `GetApi(ORT_API_VERSION)` returns
# null and the assert behind it fires.
#
# The ECAPA model is optional at build time. Until somebody has exported
# one (scripts/export-ecapa-onnx.py) and attached it to a release there
# is nothing to fetch, and an image that refused to build for want of it
# would block every deploy that has nothing to do with voices.
FROM debian:bookworm-slim AS deps
ARG ORT_VERSION=1.24.2
ARG ORT_SHA256=43725474ba5663642e17684717946693850e2005efbd724ac72da278fead25e6
ARG ECAPA_URL=""
ARG ECAPA_SHA256=""
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /deps
RUN curl -fsSL -o ort.tgz \
      "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-${ORT_VERSION}.tgz" \
 && echo "${ORT_SHA256}  ort.tgz" | sha256sum -c - \
 && tar xzf ort.tgz \
 && install -D "onnxruntime-linux-x64-${ORT_VERSION}/lib/libonnxruntime.so.${ORT_VERSION}" \
      /out/lib/libonnxruntime.so
# The directory is created either way, so the COPY below works whether
# or not a model was asked for.
RUN mkdir -p /out/share \
 && if [ -n "${ECAPA_URL}" ]; then \
      curl -fsSL -o /out/share/ecapa.onnx "${ECAPA_URL}" \
   && echo "${ECAPA_SHA256}  /out/share/ecapa.onnx" | sha256sum -c -; \
    fi

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
# Named explicitly rather than left to the loader: the file is installed
# under its plain name, and ORT_DYLIB_PATH is the only thing `ort`
# consults before falling back to a system search that would not find it.
COPY --from=deps /out/lib/libonnxruntime.so /usr/local/lib/libonnxruntime.so
ENV ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so
# Empty unless the build was given a model. `[recognition] model_path`
# points at /usr/local/share/niles/ecapa.onnx when there is one.
COPY --from=deps /out/share/ /usr/local/share/niles/
USER niles
# 8080 = HTTP API + /healthz · 10300 = Wyoming voice (idle without a satellite).
EXPOSE 8080 10300
ENTRYPOINT ["niles"]
CMD ["serve", "--config", "/etc/niles/niles.toml"]
