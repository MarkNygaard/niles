# Niles config UI

Vite + React + shadcn front end for Niles's `/config` API. Bundled into
the `niles` binary by the `ui` cargo feature; the container image builds
it in its own stage.

## Development

```bash
bun install
bun run dev        # http://localhost:5174
```

`vite.config.ts` proxies `/config`, `/devices` and `/healthz` to
`http://localhost:8080`, so run Niles alongside it:

```bash
cargo run -p niles-bin -- serve --config niles.toml
```

## Build

```bash
bun run build      # typecheck + bundle into web/dist
```

`web/dist` is gitignored — CI and the image build it from source. To run
the embedded UI locally:

```bash
cargo run -p niles-bin --features ui -- serve --config niles.toml
```

## Keeping in step with the API

`src/lib/api.ts` mirrors `crates/niles-api/src/config.rs` by hand. The
shapes are small enough that a generator would cost more than it saves,
but they do have to be changed together.
