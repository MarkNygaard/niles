# Permissions

> Interface 0.2, and 0.1, which Niles still runs. Niles 0.2.0 runs them and
> installs them from GitHub: the runtime, `niles plugin check`, `niles plugin
> dev`, Add from GitHub with its approval screen, updates, rollback, turning
> off and removing, and in the app the generated setup panel, the plugin's
> log, and its pages and widgets.

A plugin declares in `[permissions]` what it needs to reach. Before install,
Niles shows the household each permission **in plain words** and installs
only on their approval. At run time, anything not granted is refused with
`not-permitted`. An update that changes the permissions is approved again,
shown as what was added and what was removed.

Ask for what you use, and nothing else. The approval screen is the first thing
anyone sees of your plugin, and a long list of grants for a small feature reads
as a reason not to install it.

Permissions are static: the manifest's, the same for every household, read
before anyone has filled in a setting. They cannot depend on settings. A
plugin that can announce something, but only when the household turns
announcements on in its settings, still asks for `notify`, and the approval
screen shows "Speak through your satellites" even to a household that will
leave announcements off. Say so in the setting's `help`, and in your README.

## What every plugin gets, and what none does

Without asking, every plugin can log, read its own settings and secrets, keep
1 MiB in its own storage, set timers, read the clock and get random numbers.

No plugin, whatever it asks for, can read Niles's database, other
integrations' secrets, conversations, memory, voice data or the people in the
house; open files, sockets or environment variables; or run code in the app.
Its network is HTTP and MQTT, to what the manifest names.
The approval screen always ends with that sentence: "It cannot see your
conversations, the people in your house, other integrations' passwords or
Niles's database."

## The permissions

### `http`

```toml
http = ["api.energidataservice.dk", "*.tibber.com"]
```

HTTPS requests to these hosts. `*.tibber.com` matches any subdomain
(`api.tibber.com`, `a.b.tibber.com`) but not `tibber.com` itself; list both if
you need both. No ports, paths or schemes, only host names. A host here that
resolves to a private address is still refused; that is `local_network`'s job.

Shown as: **"Connect to api.energidataservice.dk"**, **"Connect to any address
under tibber.com"**.

### `local_network`

```toml
local_network = ["setting:address"]
# or: ["192.168.1.50", "inverter.lan", "192.168.20.0/24"]
```

Requests to addresses on the home network: private ranges, link-local, `.lan`
and `.local`-style names that resolve to them. Plain `http` is allowed here.
Entries are:

- **`setting:<key>`**: whatever address the user types into that setting. The
  usual choice, since only the household knows its device's address.
- an IP address, or a host name that resolves to a local address;
- a range in CIDR, no wider than a `/24`.

Loopback (`127.0.0.1`, `::1`) and the cloud metadata address
(`169.254.169.254`) are refused whatever is declared. Niles's own database is
reachable only with credentials no plugin is given, but a `/24` that contains
other machines lets the plugin talk HTTP to all of them; prefer `setting:`.

Shown as: **"Reach the device at the address you enter under Address, on your
home network"**, **"Reach 192.168.1.50 on your home network"**, **"Reach any of
256 addresses on your home network (192.168.20.0/24)"**.

### `mqtt`

Interface 0.2.

```toml
mqtt = [
    { name = "printer", host = "setting:address", port = 8883, tls = "trust-device" },
    { name = "cloud", host = "broker.example.com" },
]
```

Connections to these MQTT brokers, each named for `mqtt.connect`. `host` is a
host name, an address, or `setting:<key>` for the address the household types
in; `port` is 8883 unless given (1883 with `tls = "none"`). `tls` says how the
broker's certificate is checked:

- **`verify`** (the default): against the public certificate authorities, for
  the host's name.
- **`trust-device`**: the device's own certificate, trusted the first time and
  pinned from then on; a different one stops the connection until the
  household accepts it. Home network only.
- **`none`**: no TLS. Home network only.

Every address is checked again after DNS: loopback and the cloud metadata
address never; `trust-device` and `none` only on the home network; a host
name written here for `verify` only on the internet, as an `http` host. See
[MQTT](mqtt.md#which-addresses-are-allowed).

Shown as: **"Connect to the MQTT broker at the address you enter under Printer
address, on your home network, trusting the certificate it shows the first
time"**, **"Connect to the MQTT broker at 192.168.1.20, on your home network,
unencrypted"**, **"Connect to the MQTT broker at broker.example.com"**.

### `devices`

```toml
devices = true
```

Add, update and remove the plugin's own devices, and receive commands for the
controllable ones. Never other sources' devices: to see those, use `events`.

Shown as: **"Add devices to your home. They appear in rooms, in rules and by
voice."**

### `tools`

```toml
tools = true
```

Offer tools to the voice model and the assistants. Needs `keywords` in the
manifest.

Shown as: **"Offer 2 tools to the assistant: prices, cheapest_window"**, with
each tool's description a tap away.

### `events`

```toml
events = ["device_state", "device_action", "presence"]
```

Subscribe to what happens elsewhere in the house. List only the kinds you use.

- `device_state`: every device's state changes, from any source. This is a
  lot: lights going on and off, doors opening, motion. It says when people are
  home, awake and in which rooms.
- `device_action`: button and wall-switch presses, and the actions other
  devices report, such as another plugin's printer finishing a print.
- `presence`: whether anyone is home, as one value for the house. Never who.

Shown as: **"See every device in your home change: lights, doors, motion"**,
**"See button presses and what other devices report"**, **"Know when the
house becomes occupied or empty"**.

### `notify`

```toml
notify = true        # low and normal priority
notify = "urgent"    # also urgent, which plays during quiet hours
```

Speak through the satellites via Niles's notifications, at most 6 an hour.

Shown as: **"Speak through your satellites"**, or **"Speak through your
satellites, even during quiet hours"**.

### `ui`

```toml
ui = true
```

Add the pages and widgets declared under `[[ui.menu]]` and `[[ui.widgets]]`,
and see the signed-in person's slug when drawing them.

Shown as one line per surface: **"Adds a page to the menu: Electricity"**,
**"Adds a card to the front page: Electricity now"**, **"Adds a card to the
Kitchen: Air quality"**, and for a widget placed by a setting, **"Adds a card
to the room its "Room" setting names: Printer"**.

### `memory_mb`

Not a permission, but shown with them when it is above the default 64:
**"Use up to 128 MB of memory"**.

## What the household is trusting you with

Be straightforward about this in your README; the approval screen is. The
sandbox limits what your plugin can **reach**. Within that, it is trusted: a
plugin holding an account's password can do whatever that account can; one
subscribed to `device_state` learns the house's daily rhythm and could send it
to any host it declared; a tool result can tell the model something untrue. The
household's protections are the approval screen, your repo being public, the
pinned component (what they approved cannot change under them), and re-approval
when your permissions change.
