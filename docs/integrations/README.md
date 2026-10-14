# Writing a Niles integration

> **This is the v0.1 specification, not working software.** Niles cannot run
> plugins yet: the runtime, the "Add from GitHub" flow and the `niles plugin`
> commands described here are designed but not built. The interface
> ([`schemas/niles-integration.wit`](../../schemas/niles-integration.wit)) may
> still change before it ships. Everything here compiles against that interface
> (the worked example was built with `wasm32-wasip2`), but nothing will load it
> until the runtime lands.

Niles is an open-source home assistant, written in Rust, that you talk to. It
runs the lights, music, heating and calendars of one house, and it hands
questions it cannot answer from fixed phrases to a language model with tools.
Its built-in integrations live in its own repository. This guide is for
everything else: the thing in your house, or the service you use, that Niles
does not know about.

You write the integration in Rust, JavaScript, Go or Python, compile it to a
WebAssembly component, publish it as a GitHub release, and anyone running Niles
installs it by pasting your repo's URL into the app.

## The three terms you need

- **A WebAssembly component** is a compiled `.wasm` file that also says, in its
  own metadata, which functions it needs from its host and which functions it
  provides. It runs in a sandbox: it can do nothing the host does not hand it.
- **WIT** (WebAssembly Interface Type) is the language those declarations are
  written in. Niles's is one file,
  [`niles-integration.wit`](../../schemas/niles-integration.wit): package
  `niles:integration@0.1.0`, world `plugin`. A *world* is the full contract,
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

## What you cannot do

A plugin runs in a sandbox. It cannot read Niles's database, other
integrations' passwords, conversations, memory or the people in the house. It
has no filesystem, no environment variables and no sockets; the only network
access is HTTP to the hosts your manifest declares. It cannot put HTML or
script into the app. See [permissions](permissions.md).

## Where it shows up

Once installed, a plugin sits on the app's Integrations page among the
built-ins, in the group its manifest names (Music & TV, Presence & climate…),
or under **Custom** if it names none. Its row carries a **Custom** badge, and
its setup panel, generated from your manifest's settings, shows your repo,
version and authors, a status line ("Working", "Not working: …") and its log.
People find **Add from GitHub** at the end of the Available tab.

## Capabilities at a glance

| You want to | Use | Permission | Guide |
|---|---|---|---|
| Read your settings and secrets | `config.get`, `config.secret` | none | [manifest](manifest.md#settings) |
| Keep data between restarts | `storage` (1 MiB) | none | [host API](host-api.md#storage) |
| Call an API on the internet | `http.send` | `http = ["api.example.com"]` | [host API](host-api.md#http) |
| Call a device on the LAN | `http.send` | `local_network = ["setting:address"]` | [permissions](permissions.md#local_network) |
| Run something every N seconds | `timers.every` → `on-timer` | none | [host API](host-api.md#timers) |
| Add lights, plugs, sensors, covers | `devices.upsert`, `report-state` | `devices = true` | [devices](devices.md) |
| Be switched by voice and rules | export `set-device` | `devices = true` | [devices](devices.md#controllable-devices) |
| Answer questions, do things on request | exports `tools`, `call-tool` | `tools = true` | [tools](tools.md) |
| Add a page to the menu | `[[ui.menu]]`, exports `render`, `on-action` | `ui = true` | [UI](ui.md) |
| Add a widget to the front page or a room card | `[[ui.widgets]]`, same exports | `ui = true` | [UI](ui.md) |
| Speak through the satellites | `notify.say` | `notify = true` or `"urgent"` | [host API](host-api.md#notify) |
| React to other devices or presence | `events.subscribe` → `on-event` | `events = [...]` | [host API](host-api.md#events) |
| Log | `log.log` | none | [host API](host-api.md#log) |

Not in v0.1: raw TCP or UDP, WebSockets, and a page that runs your own HTML.
See [host API](host-api.md#coming-later).

## The guide

1. [Getting started](getting-started.md): template, build, run locally, release, install
2. [The manifest](manifest.md): every field of `niles-integration.toml`
3. [Host API](host-api.md): every import and export, with examples
4. [Devices](devices.md): classes, state fields and units, controllable devices, naming
5. [Tools](tools.md): designing tools the AI uses well
6. [Pages and widgets](ui.md): the declarative UI
7. [Permissions](permissions.md): what each one allows and how the household sees it
8. [Publishing](publishing.md): release layout, versioning, updates
9. [Worked example: spot prices](example-spotprice.md): a complete plugin in Rust,
   with the same tool in JavaScript and Python

The design and its reasoning are in
[ARCHITECTURE.md](../../ARCHITECTURE.md#third-party-integrations-plugins).
