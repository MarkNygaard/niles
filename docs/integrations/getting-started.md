# Getting started

> v0.1 of the interface. `niles plugin dev`, `niles plugin check` and
> installing from GitHub are built (Niles 0.2.0). Not yet: the generated
> setup panel (step 7 says what to do meanwhile), and the template repo,
> which will be published alongside it.

From nothing to a plugin installed on a Niles, in Rust. For JavaScript and
Python, see the snippets at the end of the [worked example](example-spotprice.md#the-same-tool-in-javascript-and-python).

## 1. What you need

- Rust (stable, 1.85 or later, for edition 2024) with the WebAssembly target:

  ```sh
  rustup target add wasm32-wasip2
  ```

- The `niles` binary, for `niles plugin dev` and `niles plugin check`. Either
  from a Niles release, or built from source:

  ```sh
  cargo install --locked --git https://github.com/MarkNygaard/niles niles-bin
  ```

- Optional: [`wasm-tools`](https://github.com/bytecodealliance/wasm-tools)
  (`cargo install wasm-tools`) to look inside a component:
  `wasm-tools component wit integration.wasm` prints what it imports and exports.

## 2. Start from the template

Create a repo from `niles-integration-template` ("Use this template" on GitHub).
It holds:

```
niles-integration.toml       # the manifest: id, name, settings, permissions
logo.svg                     # 24x24 mark shown on the Integrations page
wit/niles-integration.wit    # the interface, copied from Niles at v0.1.0
src/lib.rs                   # every export stubbed out
Cargo.toml                   # crate-type = ["cdylib"], wit-bindgen
dev-settings.toml            # setting values for local runs (gitignored)
.github/workflows/release.yml
```

Change the manifest's `id` (lowercase letters, digits and `_`, starting with a
letter: it becomes your device source, tool prefix and secret namespace), its
`name`, `description` and `authors`, and the crate name in `Cargo.toml`.

`src/lib.rs` starts like this:

```rust
wit_bindgen::generate!({
    world: "plugin",
    path: "wit",
});

struct MyPlugin;

export!(MyPlugin);

impl Guest for MyPlugin {
    fn init() -> Result<(), String> {
        Ok(())
    }
    fn tools() -> Vec<ToolDef> {
        Vec::new()
    }
    fn call_tool(name: String, _args_json: String) -> Result<String, String> {
        Err(format!("There is no tool called {name}."))
    }
    fn set_device(_id: String, _desired: DeviceState) -> Result<(), String> {
        Err("Nothing here can be switched.".into())
    }
    fn on_timer(_id: String) {}
    fn on_event(_event: Event) {}
    fn render(_surface_id: String, _context: RenderContext) -> Result<UiTree, String> {
        Err("No pages.".into())
    }
    fn on_action(
        _surface_id: String,
        _control_id: String,
        _value: ActionValue,
        _context: RenderContext,
    ) -> Result<ActionResult, String> {
        Err("No pages.".into())
    }
    fn stop() {}
}
```

`generate!` reads `wit/` and writes, inside your crate:

- `Guest`, the trait with one method per export, and `export!`, which wires
  your type to it;
- one module per import, under `niles::integration::`: `log`, `config`,
  `storage`, `http`, `devices`, `timers`, `events`, `notify`, `ui`;
- the types the exports use (`DeviceState`, `Event`, `ToolDef`,
  `RenderContext`, `UiTree`, `ActionValue`, `ActionResult`) at the crate root.
  Other types come from their interface, e.g. `niles::integration::types::Device`.

Every export must be there; return an empty list or an error from the ones you
do not use. Read the [host API](host-api.md) for what each one is for.

To see the generated code, `cargo doc --target wasm32-wasip2 --open`.

## 3. Build

```sh
cargo build --release --target wasm32-wasip2
```

The component is `target/wasm32-wasip2/release/<crate_name>.wasm`. Plain Rust
with `serde_json` comes out around 200 KB.

## 4. Run it locally

Put values for your settings in `dev-settings.toml`, keyed like the manifest:

```toml
area = "DK1"
cheap_hours = 4
# secrets too, for local runs only; never commit this file
api_key = "..."
```

Then:

```sh
niles plugin dev .
```

This builds nothing; it reads `niles-integration.toml` and the newest
`target/wasm32-wasip2/release/*.wasm` in the directory (or `integration.wasm`
if there is one, or the `.wasm` you name), and runs it in the same sandbox and
with the same limits as a real Niles, against a stand-in house:

- `init` runs, and log lines print as they happen;
- devices the plugin reports print as they change;
- storage is kept in `.niles-dev/storage.json`, so restarts keep it (add
  `.niles-dev/` to your `.gitignore`);
- timers fire on time;
- HTTP goes out for real, checked against your manifest's permissions, so a
  missing permission fails here exactly as it would at home.

At its prompt:

```
> tools                               list the tools as the model sees them
> tool prices {"day":"tomorrow"}      call one, print the result
> set system/lamp {"on":true}         call set-device
> event device_state office/door {"open":true}
> event presence away                 something happening elsewhere
> render electricity                  draw a page as text
> action electricity refresh          press a control
> restart                             stop, new instance, init
```

Settings on the command line go over `dev-settings.toml`, typed by your
manifest: `niles plugin dev . --settings area=DK2 --settings cheap_hours=6`.
For a script or CI, `--call prices '{"day":"today"}'` and `--render
electricity` do the same as the prompt once `init` has run, and `--dry-run`
exits after them instead of staying up for timers and the prompt.

Planned, not built yet: `niles plugin dev . --live https://niles.example.com
--token $NILES_API_TOKEN` would install it on a running Niles instead, as a
development plugin marked "Development" on its card, reinstall it each time the
`.wasm` changes, and stream its log to your terminal.

## 5. Check it

```sh
niles plugin check .
```

(or `niles plugin check you/your-repo` once it is released, to check the
release exactly as an install would download it) checks everything an install
would: the manifest (including its surfaces,
settings and permissions), the logo, that the component targets
`niles:integration/plugin@0.1.x` and exports every function, and its tools'
names, descriptions and schemas, asked for with `tools()` before any `init`,
and that there are keywords if there are tools. Fix whatever it reports before
you publish; it exits non-zero while there is anything to fix.

## 6. Release

Bump `version` in the manifest, commit, then tag the same version:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The template's workflow builds the component and creates a GitHub release with
`niles-integration.toml`, `integration.wasm` and `logo.svg` attached. Details
in [publishing](publishing.md).

## 7. Install

In the Niles app: Integrations → Available → **Add from GitHub** (at the end of
the page), paste `https://github.com/you/your-repo` and press **Preview**.
Niles downloads your latest release, runs every check `niles plugin check`
runs, and shows the name, authors, repo, version, description, logo and the
permissions in plain words, with the settings it will ask for. **Install**
installs exactly the files that preview downloaded, and the plugin starts. It
is on the Connected tab with a status line; its panel has your repo, the
version and install date, and buttons to turn it off, check for updates, roll
back an update and remove it.

Until the generated settings form is in the app, set settings through the
config API, which restarts the plugin with them:

```sh
curl -X PATCH https://niles.example.com/config \
  -H "authorization: Bearer $NILES_API_TOKEN" -H "content-type: application/json" \
  -d '{"plugins": {"spotprice": {"area": "DK2"}}}'
```

and secret settings through `PUT /secrets/plugins.<id>.<key>` with
`{"value": "…"}`, which accepts only keys your manifest declares as `secret`.

A preview is kept for 15 minutes. Installing needs Niles's database; without
one the preview says so. The same flow is reachable without the app:
`POST /plugins/install/preview` with `{"repo": "you/your-repo"}`, then
`POST /plugins/install` with the `token` (and `sha256`) it answered.
