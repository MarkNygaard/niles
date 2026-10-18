# Writing a Niles add-on

A third-party add-on is what Niles calls a plugin in the app: something
somebody else wrote, installed from GitHub. The technical names keep the old
word — the WIT package is `niles:integration`, the manifest
`niles-integration.toml`, the component `integration.wasm` and this folder
`docs/integrations/` — because plugins already built depend on them.

> **v0.4 of the interface, built.** Niles 0.2.0 runs plugins (the sandbox,
> every host interface, devices, tools, timers, events, pages, MQTT, and from
> Niles 0.3.0 accounts and webhooks) and installs
> them: **Add-on from GitHub** with its approval screen, update checks, updates
> that show what changes in permissions and roll back by themselves if the new
> version fails, turning off and removing, and the `niles plugin check` and
> `niles plugin dev` commands. The app draws everything a plugin declares: its
> setup panel generated from the manifest's settings, its log, its pages in
> the menu and its widgets on the front page and the rooms. Not built yet:
> `niles plugin dev --live`, which would run a plugin on a running Niles, and
> `--http`, which would serve its webhook locally. The interface
> ([`schemas/niles-integration.wit`](../../schemas/niles-integration.wit)) is
> v0.4: v0.1 plus MQTT (0.2), the house's time zone and lights kept out of
> their room's (0.2.1), connecting accounts and webhooks (0.3), and WebSockets (0.4). Niles still
> runs plugins built for 0.2 and 0.1. See
> [versioning](publishing.md#interface-versions) for what may change.

Niles is an open-source home assistant, written in Rust, that you talk to. It
runs the lights, music, heating and calendars of one house, and it hands
questions it cannot answer from fixed phrases to a language model with tools.
Its built-in add-ons live in its own repository. This guide is for
everything else: the thing in your house, or the service you use, that Niles
does not know about.

You write the add-on in Rust, JavaScript, Go or Python, compile it to a
WebAssembly component, publish it as a GitHub release, and anyone running Niles
installs it by pasting your repo's URL into the app.

## The three terms you need

- **A WebAssembly component** is a compiled `.wasm` file that also says, in its
  own metadata, which functions it needs from its host and which functions it
  provides. It runs in a sandbox: it can do nothing the host does not hand it.
- **WIT** (WebAssembly Interface Type) is the language those declarations are
  written in. Niles's is one file,
  [`niles-integration.wit`](../../schemas/niles-integration.wit): package
  `niles:integration@0.4.0`, world `plugin`. A *world* is the full contract,
  what you import and what you export. You do not write WIT; you read it, and a
  bindings generator turns it into types and functions in your language.
- **`cargo-component`** and **`wit-bindgen`** are the Rust tools that do that
  generation. `wit-bindgen`'s `generate!` macro reads the WIT and writes the
  Rust types, the functions you call (Niles's) and a trait you implement (yours).
  Rust's `wasm32-wasip2` target then builds a component directly.
  `cargo-component` wraps the same steps in a `cargo` subcommand; the template
  uses plain `cargo` with the target.

## What you can build

- **Devices from a cloud or local API.** An inverter, a heat pump, a pool pump,
  a smart plug with an HTTP API, a weather station on the LAN. They appear in
  their rooms in the app, can be switched by voice ("turn off the pool pump"),
  and rules can follow them ("when the garage door opens after sunset…").
- **Tools for the assistant.** "When is power cheapest tomorrow?", "When's the
  next bus?", "How full is the car's battery?" You describe a tool with a JSON
  Schema; the voice model and the Claude and ChatGPT assistants decide when to
  call it.
- **Pages and widgets in the app.** A page in the menu, or a card on the front
  page or a room, drawn from a small set of Niles's own components: readings,
  lists, charts, buttons, switches, sliders.
- **Spoken notifications.** "The washing machine is done", through the same
  chime, quiet hours and recall as Niles's own.
- **Reactions to the house.** Subscribe to device changes, button presses and
  whether anyone is home.
- **Devices that speak MQTT.** A 3D printer, Tasmota and Shelly plugs, the
  household's own broker: Niles holds the connection and hands your plugin
  each message.

## What you cannot do

A plugin runs in a sandbox. It cannot read Niles's database, other
add-ons' passwords, conversations, memory or the people in the house. It
has no filesystem, no environment variables and no sockets; the only network
access is HTTP to the hosts your manifest declares, and MQTT to the brokers it
names. It cannot put HTML or
script into the app. See [permissions](permissions.md).

## Where it shows up

Once installed, a plugin sits on the app's Add-ons page (Settings → Add-ons)
among the built-ins, in the group its manifest names (Music & TV, Presence & climate…),
or under **Custom** if it names none. Its row carries a **Custom** badge, and
its panel shows your repo, version, authors and install date, a status line
("Working", "Not set up: needs API key", "Not working: …", "Off", "Update
available: v0.2.0"), the settings form generated from your manifest, where
each of your widgets goes, the last lines of its log (yours, and what Niles
noted about it), and the buttons to turn it off, check for updates, update,
roll back and remove it. Your pages join the menu, where the household shows,
hides and moves them in Settings → Menu like the built-in ones. People find
**Add-on from GitHub** at the end of the Available tab.

## Capabilities at a glance

| You want to | Use | Permission | Guide |
|---|---|---|---|
| Read your settings and secrets | `config.get`, `config.secret` | none | [manifest](manifest.md#settings) |
| Know the house's time zone | `config.time-zone` (0.2.1) | none | [host API](host-api.md#config) |
| Keep data between restarts | `storage` (1 MiB) | none | [host API](host-api.md#storage) |
| Call an API on the internet | `http.send` | `http = ["api.example.com"]` | [host API](host-api.md#http) |
| Call a device on the LAN | `http.send` | `local_network = ["setting:address"]` | [permissions](permissions.md#local_network) |
| Run something every N seconds | `timers.every` → `on-timer` | none | [host API](host-api.md#timers) |
| Add lights, plugs, sensors, covers | `devices.upsert`, `report-state` | `devices = true` | [devices](devices.md) |
| Be switched by voice and rules | export `set-device` | `devices = true` | [devices](devices.md#controllable-devices) |
| Start rules when something happens on a device | `devices.report-action` | `devices = true` | [devices](devices.md#actions) |
| Keep a light out of its room's lights | `devices.set-room-light` (0.2.1) | `devices = true` | [devices](devices.md#lights-and-the-room) |
| Answer questions, do things on request | exports `tools`, `call-tool` | `tools = true` | [tools](tools.md) |
| Add a page to the menu | `[[ui.menu]]`, exports `render`, `on-action` | `ui = true` | [UI](ui.md) |
| Add a widget to the front page or a room card | `[[ui.widgets]]`, same exports | `ui = true` | [UI](ui.md) |
| Speak through the satellites | `notify.say` | `notify = true` or `"urgent"` | [host API](host-api.md#notify) |
| React to other devices or presence | `events.subscribe` → `on-event` | `events = [...]` | [host API](host-api.md#events) |
| Talk to an MQTT device or broker | `mqtt.connect`, `subscribe`, `publish` → `on-mqtt-message`, `on-mqtt-status` | `mqtt = [{ name, host, tls }]` (0.2) | [MQTT](mqtt.md) |
| Talk to a WebSocket endpoint | `websocket.connect`, `send`, `close` → WebSocket callbacks | `websocket = [{ name, host, tls }]` (0.4) | [WebSockets](websocket.md) |
| Connect an account through its sign-in page | exports `authorize-url`, `on-callback` | `callback = true` (0.3) | [accounts](accounts.md) |
| Keep a token you refreshed, encrypted | `secrets.get`, `set`, `delete` | none (0.3) | [host API](host-api.md#secrets) |
| Be called by a service that pushes | `inbound.hook-url` → export `on-webhook` | `webhook = true` (0.3) | [accounts](accounts.md#the-webhook) |
| Log | `log.log` | none | [host API](host-api.md#log) |

Not yet: raw TCP or UDP, and a page that runs your own HTML.
See [host API](host-api.md#coming-later).

## The guide

1. [Getting started](getting-started.md): template, build, run locally, release, install
2. [The manifest](manifest.md): every field of `niles-integration.toml`
3. [Host API](host-api.md): every import and export, with examples
4. [Devices](devices.md): classes, state fields and units, controllable devices, naming
5. [Tools](tools.md): designing tools the AI uses well
6. [Pages and widgets](ui.md): the declarative UI
7. [MQTT](mqtt.md): brokers, `trust-device` certificates, a printer as the example
8. [Accounts and webhooks](accounts.md): signing in to a service, keeping its tokens, being called by it
9. [Permissions](permissions.md): what each one allows and how the household sees it
10. [Publishing](publishing.md): release layout, versioning, updates
11. [Worked example: spot prices](example-spotprice.md): a complete plugin in Rust,
   with the same tool in JavaScript and Python

Why plugins are WebAssembly components, and what the sandbox does and does
not promise, are in ARCHITECTURE.md's
[Big decisions](../../ARCHITECTURE.md#big-decisions) and
[Security notes](../../ARCHITECTURE.md#security-notes).
