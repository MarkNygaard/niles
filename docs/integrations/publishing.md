# Publishing

> Interface 0.2, and 0.1. `niles plugin check` and installing, updating and
> rolling back from GitHub releases are built (Niles 0.2.0), as described
> here.

A plugin is published as a **GitHub release** on a public repo. There is no
registry to sign up for: anyone with the repo's URL can install it.

## Release layout

The release Niles installs is the repo's **latest release**: not a draft, not
marked pre-release. (Somebody can install another by pasting its address,
`https://github.com/you/your-repo/releases/tag/v0.2.0`.) Its assets must
include:

| Asset | Required | What it is |
|---|---|---|
| `niles-integration.toml` | yes | The [manifest](manifest.md), at most 64 KiB. Its `version` is the release tag, with or without a `v`. |
| `integration.wasm` | yes | The component, at most 64 MiB. |
| `logo.svg` | if the manifest names one | The logo, at most 32 KiB. The asset is found by the file name of the manifest's `logo`, so `logo = "assets/logo.svg"` is the asset `logo.svg`. |

Each cap holds whatever GitHub reports as the asset's size: a download is cut
off at it.

Other assets are ignored. The release notes (the release's body) are shown to
the household when they are offered the update, so write them for people:
what changed for them, not the commit log.

## The workflow

The template's `.github/workflows/release.yml` builds and publishes on a
version tag:

```yaml
name: release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write

env:
  CRATE: niles_spotprice   # the crate name, with _ for -

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip2

      - name: Tag matches the manifest
        run: |
          version=$(grep -m1 '^version' niles-integration.toml | cut -d '"' -f2)
          if [ "v$version" != "$GITHUB_REF_NAME" ]; then
            echo "Tag $GITHUB_REF_NAME but the manifest says $version" >&2
            exit 1
          fi

      - name: Build
        run: cargo build --release --target wasm32-wasip2

      - name: Collect
        run: cp "target/wasm32-wasip2/release/$CRATE.wasm" integration.wasm

      - name: Release
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          gh release create "$GITHUB_REF_NAME" \
            --title "$GITHUB_REF_NAME" \
            --generate-notes \
            integration.wasm niles-integration.toml logo.svg
```

Once `niles` publishes release binaries, a `niles plugin check .` step before
`Release` is worth adding.

For JavaScript, Python or Go, replace the toolchain and build steps with
`jco componentize`, `componentize-py` or `tinygo build`, and keep the rest.

## Versioning your plugin

Semver, judged by what the household sees:

- **Patch** (0.1.0 → 0.1.1): fixes. Same tools, devices, settings and
  permissions.
- **Minor** (0.1.1 → 0.2.0): new tools, devices, settings or surfaces;
  new permissions.
- **Major** (0.2.0 → 1.0.0): anything that breaks what the household built on
  it: a device renamed or removed (their rules name it), a setting renamed (its
  value is lost), a tool renamed.

Never rename a device id lightly: rules, scenes and the app refer to it. Never
change the plugin's `id`: that is a different plugin to Niles, installed beside
the old one, with none of its settings, secrets or storage.

## How updates reach people

- Niles asks each installed plugin's repo for its releases every 6 hours,
  first 2 minutes after it starts, and when someone presses **Check for
  updates** on its panel.
- The newest release whose tag is a higher semver version than the installed
  one shows as **Update available**, with its version, release notes and the
  change in permissions (what is added, what is removed). A pre-release is
  offered only to someone already on a pre-release, so mark betas as
  pre-releases on GitHub. Nothing updates by itself.
- Updating downloads the release and runs the same checks as an install, and
  the household sees the difference before they confirm. New required
  settings put the plugin in "Not set up" until they are filled in.
- The previous version is kept. If the new one traps, times out or will not
  load within 10 minutes of the update, Niles puts the previous one back by
  itself and says so on the panel; `init` returning an error does not count,
  since that is your plugin saying what is wrong. The household can also roll
  back by hand, once.

Storage and settings carry across updates. If your stored data's shape
changes, read the old shape too, or keep a version number in storage and
migrate in `init`.

## Pinning: why a release cannot change under people

When somebody previews an install or an update, Niles downloads your
assets, computes their sha256 and holds them for 15 minutes. Where GitHub
reports a digest for an asset, the two must agree. Installing takes exactly
those bytes, checked against the sha256 the preview showed, and keeps them in
Niles's own database; nothing is downloaded again, at install or at any
restart after. Replacing an asset on an existing release changes nothing for
anyone who installed it; publish a new release instead.

## Which Niles versions you support

- `interface = "0.2"` in the manifest and the WIT your component was built
  with say which interface you target, and they must agree: Niles reads the
  version your component's imports carry, and refuses a component built for
  0.2 under a manifest that says 0.1, or the reverse. A Niles that runs 0.2
  runs any `0.2.x` component; patch versions of the interface only add.
- `min_niles` is the oldest Niles your plugin needs: raise it when you use a
  function or UI node added in a later 0.2.x.

### Interface versions

| Interface | Niles | What changed |
|---|---|---|
| 0.2 | 0.2.0 | The `mqtt` interface and the exports `on-mqtt-message` and `on-mqtt-status`. Nothing else. |
| 0.1 | 0.2.0 | The first. |

Niles runs plugins built for the previous interface beside the current one
for at least six months, and their panel says "Built for an older Niles".
Moving a plugin from 0.1 to 0.2 takes a copy of the new
[WIT](../../schemas/niles-integration.wit), `interface = "0.2"`, and the two
new exports, empty if you use no MQTT:

```rust
fn on_mqtt_message(_conn: u32, _topic: String, _payload: Vec<u8>) {}
fn on_mqtt_status(_conn: u32, _status: ConnectionStatus) {}
```

The 0.1 interface is kept, frozen, as
[`niles-integration-0.1.wit`](../../schemas/niles-integration-0.1.wit).

## Private repos, and GitHub's limit

Not yet. Installing from a private repo needs a GitHub token in Niles,
which is planned but not specified. Without a token GitHub allows 60 API
requests an hour from one address: a preview takes one or two, an update check
one per plugin. When the limit is reached Niles says so, with the time it
resets.
