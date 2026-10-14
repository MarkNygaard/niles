# The manifest

> v0.1 specification. Not implemented yet.

Every plugin has a `niles-integration.toml` at the root of its repo, attached
to each release. Niles reads it before downloading anything else: it decides
whether the plugin can be installed, what the household is asked to approve,
and what the setup panel looks like.

A complete example is in the [worked example](example-spotprice.md#the-manifest).

## Top-level fields

| Field | Required | What it is |
|---|---|---|
| `id` | yes | `[a-z0-9_]`, 2–32 characters, starting with a letter. Never changes once published. |
| `name` | yes | What the Integrations page calls it: "Spot prices". |
| `version` | yes | Semver, `0.1.0`. The release tag must be the same, with or without a leading `v`. |
| `description` | yes | One line, at most 120 characters. See below. |
| `authors` | yes | List of `"Name <email>"` or `"Name"`. |
| `homepage` | no | Usually the repo. Linked from the setup panel. |
| `min_niles` | yes | The oldest Niles version it runs on. The first Niles with plugins sets the floor; until it ships, use `"0.2.0"`. |
| `interface` | yes | The `niles:integration` version the component targets: `"0.1"`. |
| `group` | no | Where it sits on the Integrations page. See [Groups](#groups). |
| `logo` | no | Path of an SVG in the release, normally `"logo.svg"`. See [Logo](#logo). |
| `memory_mb` | no | Memory cap, 64 by default, at most 256. JavaScript and Python plugins want 128. |
| `keywords` | if it has tools | Words that make its tools relevant to a request. See [tools](tools.md#relevance-keywords). |

### `id`

The id ends up everywhere, so choose it once:

- device source `x_<id>`: devices are `x_spotprice:system/cheap_power` inside Niles;
- tool names `<id>_<tool>`: `spotprice_prices`;
- secrets `plugins.<id>.<key>` in Niles's secret store;
- menu entries `<id>.<entry>` in the house's `[menu]` settings.

It must not be the id of a built-in integration (`mqtt`, `groq`, `elevenlabs`,
`cerebras`, `sonos`, `spotify`, `lg_tv`, `unifi`, `tado`, `microsoft`, `google`,
`nemlig`, `claude_code`, `chatgpt`, `linear`, and any added later) or of another
plugin already installed. Prefer the product's name, short: `airgradient`, `easee`,
`rejseplanen`.

### `description`

One line saying **what the household gets**, not what the API is. The
built-in catalogue's blurbs are the model:

- "Switch and dim your Zigbee lights and WLED strips."
- "Set the heating in each room by voice."
- "Turn the shopping list into a filled basket."

So: "Hear what power costs hour by hour, and let rules run things when it is
cheapest." Not: "Integration with the Energi Data Service DayAheadPrices API."

### Groups

The Integrations page has two tabs, **Connected** and **Available**, each
grouped by what an integration does for the house. `group` is one of the
built-in group ids:

| `group` | Shown as | For |
|---|---|---|
| `essentials` | Essentials | What Niles needs to reach lights, hear and answer |
| `music` | Music & TV | Speakers, music, the television |
| `presence` | Presence & climate | Who is home, heating, air |
| `calendars` | Calendars | Calendars |
| `shopping` | Shopping | Groceries and deliveries |
| `assistants` | Assistants & tools | AI assistants and work tools |

A missing or unknown `group` puts the plugin in **Custom**, shown after the
built-in groups. These ids are part of the interface: Niles may add groups
later, and a plugin naming a group its Niles does not know yet lands in Custom
rather than failing, so naming a newer group is safe.

Wherever it sits, a plugin's row carries a **Custom** badge, and its setup panel
shows where it came from: your repo (linked), authors, version and install date.
**Add from GitHub** is at the end of the Available tab under its own heading.

### Logo

An SVG on a 24×24 view box, like the built-in marks: one or a few paths, a
brand colour or `currentColor` (which follows the text colour, so it works in
dark mode). Niles sanitises it at install and serves its own copy, so it
accepts only `svg`, `g`, `path`, `circle`, `ellipse`, `rect`, `line`,
`polyline` and `polygon`, with geometry, `fill`, `stroke` and `transform`
attributes. No `script`, `style`, `image`, `use`, `foreignObject`, links or
external references, and at most 32 KiB. Without a logo the card shows a
generic icon.

## `[permissions]`

What the plugin may reach. Everything not listed is refused at run time, and
the household approves this list in plain words before install. Each entry is
explained in [permissions](permissions.md).

```toml
[permissions]
http = ["api.example.com", "*.example.net"]
local_network = ["setting:address"]
devices = true
tools = true
events = ["device_state", "device_action", "presence"]
notify = true            # or "urgent"
ui = true
```

All are optional; leave out what you do not use.

## Settings

Each `[[settings]]` table is one field on the generated setup panel.

| Field | Required | What it is |
|---|---|---|
| `key` | yes | `[a-z0-9_]+`, what `config.get` and `config.secret` take. |
| `label` | yes | The field's label. |
| `type` | yes | `string`, `number`, `boolean`, `secret`, `choice` or `room`. |
| `required` | no | `false` by default. Until every required setting is filled in, the plugin is "Not set up" and does not start. |
| `help` | no | One or two sentences under the field: where to find the value. |
| `default` | no | Used until the user enters something. Not allowed for `secret`. |
| `options` | for `choice` | A list of `{ value, label }`. |
| `min`, `max` | no | For `number`. |

The types:

- **`string`**: one line of text. `config.get` returns `text`.
- **`number`**: `config.get` returns `number` (an `f64`).
- **`boolean`**: a switch. `config.get` returns `boolean`.
- **`secret`**: a password field. The value goes into Niles's encrypted secret
  store as `plugins.<id>.<key>`, is never shown again (the panel says only
  whether it is set), and reaches the plugin only through `config.secret`.
  `config.get` returns none for it.
- **`choice`**: a drop-down of `options`. `config.get` returns the chosen
  option's `value` as `text`.
- **`room`**: a drop-down of the house's rooms. `config.get` returns the room
  id, `living_room`, as `text`. Use it for "which room is this device in".

```toml
[[settings]]
key = "address"
label = "Address"
type = "string"
required = true
help = "The device's IP address or host name, such as 192.168.1.50. Give it a fixed address in your router."

[[settings]]
key = "room"
label = "Room"
type = "room"
required = true

[[settings]]
key = "api_key"
label = "API key"
type = "secret"
required = true
help = "Under Account → API in the vendor's app."

[[settings]]
key = "interval"
label = "Check every (seconds)"
type = "number"
default = 60
min = 15
max = 3600
```

Saving the panel restarts the plugin: `stop`, a fresh instance, `init`. Read
your settings in `init` and you never have to handle them changing.

## `[[ui.menu]]` and `[[ui.widgets]]`

Pages and widgets the plugin adds to the app. Need `ui = true`. Described in
[pages and widgets](ui.md#declaring-surfaces).

```toml
[[ui.menu]]
id = "electricity"
label = "Electricity"
icon = "zap"
refresh_every = 300

[[ui.widgets]]
id = "price_now"
title = "Electricity now"
room = "kitchen"
refresh_every = 300
```

## What Niles checks

At install and in `niles plugin check`: every required field is present and
well-formed; `version` matches the release tag; `interface` is one this Niles
runs and `min_niles` is not newer than it; `id` is free; settings keys are
unique and defaults match their types; `keywords` exist if the component
returns tools; surface ids are unique and `[a-z0-9_]+`; icons are Lucide names;
the logo passes sanitising.
