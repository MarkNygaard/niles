# Publishing

> v0.1 of the interface. `niles plugin check` is built; installing a
> release from GitHub is not yet.

A plugin is published as a **GitHub release** on a public repo. There is no
registry to sign up for: anyone with the repo's URL can install it.

## Release layout

The release Niles installs is the repo's **latest release**: not a draft, not
marked pre-release. Its assets must include:

| Asset | Required | What it is |
|---|---|---|
| `niles-integration.toml` | yes | The [manifest](manifest.md). Its `version` is the release tag, with or without a `v`. |
| `integration.wasm` | yes | The component, at most 64 MiB. |
| `logo.svg` | if the manifest names one | The logo, at most 32 KiB. |

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

- Niles checks each installed plugin's repo for a newer release once a day,
  and when someone presses **Check for updates** on its panel.
- A newer release shows as **Update available**, with its version and release
  notes. Nothing updates by itself.
- Updating runs the same checks as an install. If the permissions changed, the
  household sees the difference (added, removed) and approves again. New
  required settings put the plugin in "Not set up" until they are filled in.
- The previous version is kept, so a broken update can be rolled back once
  from the panel.

Storage and settings carry across updates. If your stored data's shape
changes, read the old shape too, or keep a version number in storage and
migrate in `init`.

## Pinning: why a release cannot change under people

When a plugin is installed or updated, Niles downloads `integration.wasm`,
computes its sha256 and keeps the bytes in its own database. Where GitHub
reports a digest for the asset, the two must agree. From then on Niles runs
that copy. Replacing an asset on an existing release changes nothing for
anyone who installed it; publish a new release instead.

## Which Niles versions you support

- `interface = "0.1"` in the manifest and the WIT your component was built
  with say which interface you target. A Niles that runs 0.1 runs any
  `0.1.x` component; patch versions of the interface only add.
- When interface 0.2 comes, Niles runs 0.1 plugins beside 0.2 ones for at
  least six months, marked "built for an older Niles". Rebuild against the new
  WIT within that time.
- `min_niles` is the oldest Niles your plugin needs: raise it when you use a
  function or UI node added in a later 0.1.x.

## Private repos

Not in v0.1. Installing from a private repo needs a GitHub token in Niles,
which is planned but not specified.
