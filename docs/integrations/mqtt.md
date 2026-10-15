# MQTT

> New in interface 0.2, run by Niles 0.2.0. A plugin targets 0.2 with
> `interface = "0.2"` in its manifest and the
> [WIT](../../schemas/niles-integration.wit) of 0.2; see
> [publishing](publishing.md#interface-versions).

Plenty of devices speak MQTT rather than HTTP, or push their state over it
instead of waiting to be asked: 3D printers, Tasmota and Shelly plugs, energy
meters, anything flashed with ESPHome's MQTT client, and the household's own
broker. The `mqtt` interface lets a plugin connect to brokers its manifest
names, subscribe, publish, and receive messages as they arrive.

**Niles holds the connection.** `connect` starts it and returns at once. A
task in Niles resolves the address, checks it, connects, does TLS, keeps the
connection alive with pings, and when it drops reconnects with a backoff (1 s,
2 s, 5 s, 10 s, 30 s, then every 60 s) and subscribes again to everything the
plugin subscribed to. Your plugin never waits on the network: it hears what
happens through two exports, `on-mqtt-status` and `on-mqtt-message`, called
one at a time like every other export.

## Declaring a broker

Each broker the plugin may connect to is one entry under `mqtt` in
`[permissions]`:

```toml
[permissions]
mqtt = [
    { name = "printer", host = "setting:address", port = 8883, tls = "trust-device" },
]
```

| Field | Required | What it is |
|---|---|---|
| `name` | yes | What `mqtt::connect` calls it. `[a-z0-9_]+`, at most 32 characters, unique. |
| `host` | yes | A host name, an IP address, or `setting:<key>` for the address the household types into a `string` setting. |
| `port` | no | 8883, or 1883 with `tls = "none"`. |
| `tls` | no | `"verify"` (the default), `"trust-device"` or `"none"`. |

### `tls`

- **`verify`**: TLS checked against the public certificate authorities for
  the host's name, as a browser does. For a cloud broker, or a home broker
  with a real certificate.
- **`trust-device`**: TLS with the certificate the device presents, signed by
  its maker or by nobody. **Home network only.** Niles trusts the certificate
  the first time it connects, and from then on only that one; see
  [below](#trust-device-trust-on-first-use).
- **`none`**: no TLS at all; everything, the password included, crosses the
  network as it is. **Home network only.** For devices that offer nothing
  better, such as most Tasmota and Shelly firmware.

### Which addresses are allowed

The address is checked when the plugin connects, and every address a name
resolves to is checked again, with the same rules as [`http`](host-api.md#http)
and [`local_network`](permissions.md#local_network). Niles connects to the
address it checked, so DNS cannot be pointed elsewhere in between.

- Loopback (`127.0.0.1`, `::1`) and the cloud metadata address are never
  reachable.
- `trust-device` and `none`: home-network addresses only (private ranges,
  link-local, unique-local), whether the host is written in the manifest or
  typed into a setting.
- `verify` with a host name in the manifest: internet addresses only, as an
  `http` host. A name that resolves into the house is refused.
- `verify` with an address in the manifest, or one the household typed in:
  either. The certificate has to be valid for it.

### How the household sees it

The approval screen says where and how, one line per broker:

- **"Connect to the MQTT broker at the address you enter under Printer
  address, on your home network, trusting the certificate it shows the first
  time"**
- **"Connect to the MQTT broker at 192.168.1.20, on your home network,
  unencrypted"**
- **"Connect to the MQTT broker at broker.example.com, port 8884"**

## Connecting

```rust
use niles::integration::mqtt::{self, ConnectOptions, Qos};

let conn = mqtt::connect(
    "printer",
    &ConnectOptions {
        client_id: None,                  // niles-<id>- and 8 random hex digits
        username: Some("bblp".into()),
        password: config::secret("access_code"),
        keep_alive: None,                 // 60 s
        clean_session: None,              // true
    },
)
.map_err(|e| format!("{e:?}"))?;
mqtt::subscribe(conn, &format!("device/{serial}/report"), Qos::AtMostOnce, false)
    .map_err(|e| format!("{e:?}"))?;
```

Connect in `init`. Connections, like timers, belong to the instance: when it
goes (a settings change, a restart, an update) they are closed, and the next
`init` opens new ones. Subscribe straight after `connect`; Niles subscribes as
soon as the connection is up, and again after every reconnect.

`connect` itself fails only for what is wrong on your side: a broker the
manifest does not name, an address the rules refuse, an empty address setting
(`not-permitted`), bad options (`invalid`), or a fifth connection (`limit`).
Everything about the network arrives later, as a status.

## Statuses

```rust
fn on_mqtt_status(conn: u32, status: ConnectionStatus) {
    match status {
        ConnectionStatus::Connected => { /* ask for a full report */ }
        ConnectionStatus::Disconnected(why) => { /* Niles is reconnecting */ }
        ConnectionStatus::Failed(why) => { /* Niles stopped trying */ }
        ConnectionStatus::CertificateChanged(change) => { /* ask the household */ }
    }
}
```

- **`connected`**: up, and every subscription applied again. A good moment to
  ask the device for its whole state.
- **`disconnected(why)`**: the connection dropped or could not be made, and
  Niles is trying again. Mark the device unavailable if it stays this way.
- **`failed(why)`**: Niles stopped trying, because trying again would not
  help: the broker refused the username or password, the address is not one
  the rules allow, or a `verify` certificate is not valid. `disconnect` the
  connection, and `connect` again when something changed (usually a setting,
  which restarts the plugin anyway).
- **`certificate-changed(change)`**: see below.

Each status is also written to the plugin's log ("MQTT printer: connected to
192.168.1.40 (192.168.1.40:8883)"), so the household sees it on the panel
without your help.

## Messages

```rust
fn on_mqtt_message(conn: u32, topic: String, payload: Vec<u8>) {
    let Ok(report) = serde_json::from_slice::<serde_json::Value>(&payload) else {
        return;
    };
    // merge it into what you know, report device state…
}
```

`on-mqtt-message` has the usual 10 seconds. Statuses and messages arrive in
the order they happened, so `connected` comes before the messages that follow
it.

Up to 256 messages wait for your plugin, 8 MiB in all. If it falls behind,
the oldest are dropped and the plugin's log says how many. A subscription
with **`latest-only`** keeps only the newest waiting message per topic: right
for topics where each message is the whole state (a Tasmota `tele/…/STATE`, a
sensor's reading), wrong where messages are partial updates that build on
each other, as most printers' reports are. A status is never dropped.

```rust
mqtt::publish(conn, "cmnd/plug/POWER", b"ON", Qos::AtLeastOnce, false)
    .map_err(|e| format!("{e:?}"))?;
```

`publish` returns at once; `ok` means Niles took it, not that the broker has
it. While the connection is down it fails with `unavailable`, so a tool can
tell the person the device is not reachable instead of pretending.

## `trust-device`: trust on first use

A device's own certificate cannot be checked against anything, because
nobody vouched for it. What can be checked is that it stays the same.

The first time a connection succeeds, Niles keeps the SHA-256 of the
certificate's **public key** in the plugin's storage, under a key the plugin
cannot read or write (keys starting with `.niles/` are Niles's; they count
towards the 1 MiB). It keeps it only once the broker has accepted the
username and password, so a device that merely answers on that address is not
trusted. Every later connection, reconnects included, must present the same
key. A device that renews an expiring certificate on the same key passes
without a word.

When the key differs, Niles stops before sending anything, the password
included, and tells the plugin `certificate-changed`, with the trusted and the
presented fingerprints. That happens for good reasons (a firmware update that
issued a new key, a factory reset, a replaced device at the same address) and
for bad ones (another machine answering at that address). Only the household
can tell which, so:

1. Show it: a badge on your page, the device unavailable, a line in your log.
2. Let them accept it: a button whose `on-action` calls
   `mqtt::accept_certificate(conn)`. Niles then trusts the key it was shown,
   and connects again.

Do not call `accept_certificate` on your own. A plugin that accepts every
change has no pinning at all, and the household has no way to see that it
did.

Changing the address setting starts afresh: the pin is kept per broker and
address, and a new address is trusted on its first connection.

## Limits

| | Limit |
|---|---|
| Connections | 4 per plugin |
| Subscriptions | 64 per connection |
| Message size | 256 KiB, each way. Larger messages are dropped and counted in the log; past 1 MiB the connection is dropped and made again. |
| Publishing | 20 a second per connection, in bursts of 50 |
| Waiting for the plugin | 256 messages, 8 MiB |
| Topics and filters | 1,024 bytes |
| Protocol | MQTT 3.1.1; QoS 0, 1 and 2 |

## A worked example: a printer with a self-signed certificate

A Bambu Lab printer is a good example of the hard case. In LAN mode it runs
its own MQTT broker on port 8883, with TLS and a certificate signed by Bambu's
own authority rather than a public one; the username is `bblp` and the
password is the LAN access code shown on the printer's screen. It publishes
reports on `device/<serial>/report` and takes commands, JSON, on
`device/<serial>/request`. This is a sketch of the parts that touch MQTT, not
a finished plugin.

The manifest asks for the address, the serial and the access code, and for
one broker at the address, trusted on first use:

```toml
interface = "0.2"

[permissions]
devices = true
ui = true
mqtt = [
    { name = "printer", host = "setting:address", port = 8883, tls = "trust-device" },
]

[[settings]]
key = "address"
label = "Printer address"
type = "string"
required = true
help = "The printer's IP address, under Settings → Network on its screen. Give it a fixed address in your router."

[[settings]]
key = "serial"
label = "Serial number"
type = "string"
required = true

[[settings]]
key = "access_code"
label = "LAN access code"
type = "secret"
required = true
help = "Under Settings → Network on the printer's screen."

[[ui.menu]]
id = "printer"
label = "Printer"
icon = "printer"
```

`init` connects and subscribes to the reports. Not `latest-only`: many
printers send partial reports, only what changed, and dropping one would lose
that change.

```rust
fn init() -> Result<(), String> {
    let serial = text("serial").ok_or("The serial number is not set.")?;
    let conn = mqtt::connect(
        "printer",
        &ConnectOptions {
            client_id: None,
            username: Some("bblp".into()),
            password: config::secret("access_code"),
            keep_alive: None,
            clean_session: None,
        },
    )
    .map_err(|e| format!("{e:?}"))?;
    mqtt::subscribe(conn, &format!("device/{serial}/report"), Qos::AtMostOnce, false)
        .map_err(|e| format!("{e:?}"))?;
    STATE.with(|s| s.borrow_mut().conn = Some(conn));
    devices::upsert(&printer_device()).map_err(|e| format!("{e:?}"))?;
    Ok(())
}
```

Once connected, it asks for a full report, so it does not wait for the next
change to know where things stand; a changed certificate becomes a badge and
a button on its page:

```rust
fn on_mqtt_status(conn: u32, status: ConnectionStatus) {
    match status {
        ConnectionStatus::Connected => {
            let serial = text("serial").unwrap_or_default();
            let pushall = r#"{"pushing":{"sequence_id":"0","command":"pushall"}}"#;
            let _ = mqtt::publish(
                conn,
                &format!("device/{serial}/request"),
                pushall.as_bytes(),
                Qos::AtMostOnce,
                false,
            );
            let _ = devices::set_available("office/printer", true);
        }
        ConnectionStatus::CertificateChanged(change) => {
            STATE.with(|s| s.borrow_mut().changed = Some(change.presented));
            let _ = devices::set_available("office/printer", false);
            ui::invalidate("printer");
        }
        _ => {
            let _ = devices::set_available("office/printer", false);
        }
    }
}

fn on_action(surface: String, control: String, _: ActionValue, _: RenderContext)
    -> Result<ActionResult, String>
{
    match (surface.as_str(), control.as_str()) {
        ("printer", "trust_new_certificate") => {
            let conn = STATE.with(|s| s.borrow().conn).ok_or("Not connected.")?;
            mqtt::accept_certificate(conn).map_err(|e| format!("{e:?}"))?;
            STATE.with(|s| s.borrow_mut().changed = None);
            Ok(ActionResult { toast: Some("Trusting the printer's new certificate".into()), rerender: true })
        }
        _ => Err("That does nothing.".into()),
    }
}
```

Each report is merged into what the plugin knows and reported as readings:
`nozzle` and `bed` in °C, `progress` in %, `remaining` in min, so rules can
follow them ("when the print is done, say so in the office").

Things worth knowing for a printer like this:

- A full report is tens of KiB; well within the 256 KiB a message may be.
- Printers accept few MQTT clients at once. Open one connection.
- A firmware update can give the printer a new certificate. Your page should
  say so and offer the button; it should not accept it for the household.
- The camera is not MQTT. Plugins have no raw TCP, so it is out of reach for
  now.

## Tasmota, Shelly and the household's broker

These devices usually talk to a broker rather than being one: the
household's Mosquitto, the one Zigbee2MQTT already uses. Ask for its address,
and a username and password if it has them, with `tls = "none"` for a broker
on the home network without TLS:

```toml
mqtt = [{ name = "house", host = "setting:broker", port = 1883, tls = "none" }]
```

Subscribe to the devices' state topics with `latest-only`, since each message
is the whole state (`tele/<topic>/STATE`, `shellies/<id>/relay/0`), and
publish commands to theirs (`cmnd/<topic>/POWER`, `shellies/<id>/relay/0/command`).
A plugin on the household's broker sees every topic it subscribes to there, so
subscribe to what it needs (`tele/kitchen_plug/#`), not to `#`.

## Trying it with `niles plugin dev`

`niles plugin dev` runs the plugin against the real broker, with the same
rules, and prints what happens on the connection:

```text
mqtt 1 printer: connecting to 192.168.1.40:8883
mqtt 1 + device/01S00A000000000/report
mqtt 1 printer: connected
mqtt 1 > device/01S00A000000000/request {"pushing":{"sequence_id":"0","command":"pushall"}} (54 B)
mqtt 1 < device/01S00A000000000/report {"print":{"nozzle_temper":24.6,"bed_temper":23.1,"mc_percent":0,…} (31.4 KiB)
```

The pin is kept in `.niles-dev/storage.json` beside the plugin; delete the
`.niles/mqtt-pin/` entry there to trust the device afresh.
