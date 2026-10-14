# Host API

> v0.1 of the interface, as Niles 0.2.0 runs it. The examples compile
> against [`niles-integration.wit`](../../schemas/niles-integration.wit) with
> `wit-bindgen` 0.63; the WIT file is the authority.

A plugin **imports** functions Niles provides and **exports** functions Niles
calls. Examples are Rust; in JavaScript and Python the names are the same in
camelCase and snake_case (see the [worked example](example-spotprice.md#the-same-tool-in-javascript-and-python)).

Rules that hold for every call:

- **One call at a time.** Niles never calls into your plugin twice at once,
  and never while your plugin is inside a host call. Global state needs no
  locks; `thread_local!` with a `RefCell`, or a `static Mutex`, is fine.
- **Calls have deadlines**: 10 s for most exports, 30 s for `init`, 3 s for
  `render`, 2 s for `stop`. Time spent waiting in host calls counts. Past the
  deadline the instance is thrown away and restarted with backoff.
- **Your instance can disappear** after a trap, a timeout, a settings change
  or an update. Anything that must survive goes in [storage](#storage).

In Rust, records passed to imports are borrowed when they contain strings or
lists (`&Request`, `&Device`, and `&DeviceState`, whose `readings` is a list)
and passed by value when they do not. wit-bindgen does not derive `Default`;
keep an `empty_state()` helper for `DeviceState`, with `readings: Vec::new()`
(see the worked example).

## Exports: what you implement

### `init() -> result<_, string>`

Called once per instance: when the plugin starts, after every settings change,
after a restart. 30 seconds. Read your settings, register devices, set timers,
subscribe to events. Do not assume a fetch will succeed here; a device that is
off at boot should leave you "working, device unavailable", not "not working".

An `err` becomes the plugin's status line, so write it for the person reading
the card: `Err("The address is not set.".into())`. Niles retries when settings
are saved and every 10 minutes.

### `tools() -> list<tool-def>`

Read after each successful `init`. Return `Vec::new()` for none. See
[tools](tools.md).

### `call-tool(name, args-json) -> result<string, string>`

`name` is the bare name from your `tool-def` (`prices`, not `spotprice_prices`).
`args-json` is a JSON object the model produced against your schema; parse it,
and do not trust it further than that. `ok` is what the model reads; `err` is
told to the model as the tool's failure, so it can say "The bus service isn't
answering" rather than nothing.

### `set-device(id, desired) -> result<_, string>`

For your controllable devices only. `desired` holds just the fields to change.
Send the command, and once the device confirms, report the new state with
`devices::report_state`; the app shows the change as soon as it is asked for
and settles it when you report. See [devices](devices.md#controllable-devices).

### `on-timer(id)`

A timer you set fired. If the previous call for the same timer id is still
running, this one is skipped rather than queued, so a slow poll cannot pile up.

### `on-event(event)`

An event you subscribed to. See [events](#events).

```rust
fn on_event(event: Event) {
    match event {
        Event::DeviceState(change) if change.state.open == Some(true) => {
            // change.id is the full id, e.g. "z2m:office/door"
        }
        Event::DeviceAction(press) => { /* press.id, press.action */ }
        Event::Presence(HomeState::Away) => { /* nobody home */ }
        _ => {}
    }
}
```

### `render(surface-id, context) -> result<ui-tree, string>` and `on-action(...)`

Pages and widgets. See [pages and widgets](ui.md).

### `stop()`

The instance is about to go: Niles is stopping, or the plugin is being
updated, disabled, removed or restarted for new settings. Two seconds. Nothing
is guaranteed to run after it, and a crash skips it, so do not rely on it to
save anything important.

## Imports: what Niles provides

### `log`

```rust
use niles::integration::log;

log::log(log::Level::Info, "polling the inverter");
```

Levels `Debug`, `Info`, `Warn`, `Error`. Lines go to Niles's log, tagged with
your id, and to your card's log in the app (the last 1,000). At most 100 lines
per 10 s and 2 KiB each; beyond that lines are counted, not kept. Stdout and
stderr (`println!`, `eprintln!`, a panic message) also land here, as info and
warn.

### `config`

```rust
use niles::integration::config;

let address = match config::get("address") {
    Some(config::SettingValue::Text(address)) => address,
    _ => return Err("The address is not set.".into()),
};
let key: Option<String> = config::secret("api_key");
```

`get` returns what the user entered, else the manifest's `default`, else
`None`. `choice` and `room` settings come back as `Text`. `secret` returns
`secret` settings only; `get` returns `None` for them. Asking for a key the
manifest does not declare returns `None` and logs a warning, which is how
typos show up.

### `storage`

A private key-value store of strings, kept in Niles's database. Nobody else
can read it; removing the plugin deletes it.

```rust
use niles::integration::storage;

storage::set("cursor", "2026-10-14T12:00:00Z").map_err(|e| format!("{e:?}"))?;
let cursor: Option<String> = storage::get("cursor");
for key in storage::list_keys("day:") {
    storage::delete(&key);
}
```

Keys up to 256 bytes. All keys and values together up to 1 MiB; a `set` that
would go over fails with `HostError::Limit`. Store JSON with `serde_json` when
you need structure. `list_keys` returns keys in sorted order, so date-stamped
keys (`day:2026-10-14`) list oldest first.

### `http`

One request, the whole response.

```rust
use niles::integration::http;

let response = http::send(&http::Request {
    method: http::Method::Post,
    url: "https://api.example.com/v1/login".into(),
    headers: vec![("content-type".into(), "application/json".into())],
    body: Some(br#"{"user":"me"}"#.to_vec()),
    timeout_ms: None,
});
match response {
    Ok(r) if r.status == 200 => { /* r.body is Vec<u8> */ }
    Ok(r) => return Err(format!("The service answered {}.", r.status)),
    Err(http::HttpError::Timeout) => return Err("The service did not answer in time.".into()),
    Err(e) => return Err(format!("{e:?}")),
}
```

- The host must be in your manifest's `http` list, exactly or under a
  `*.example.com` wildcard, or covered by `local_network`. Otherwise:
  `HttpError::NotPermitted`.
- After DNS, the address is checked again. Private, loopback and link-local
  addresses (`192.168.x.x`, `10.x.x.x`, `127.0.0.1`, `169.254.x.x`, and their
  IPv6 kin) are refused unless `local_network` names them, whatever the host
  name says.
- Internet hosts must be `https`. Plain `http` is allowed to local-network
  addresses only.
- Redirects are followed, up to 5, each checked the same way.
- A 404 or 500 is an `Ok` response with that status; only failures to get an
  answer are errors: `Timeout`, `Connection` (DNS, refused, TLS),
  `TooLarge` (over 4 MiB), `InvalidRequest`.
- `timeout_ms` defaults to 5,000 and is cut to whatever is left of the
  current call's deadline.
- Niles adds `user-agent: niles-plugin/<id>/<version>` unless you set one.
  No cookies are kept between requests; if a service needs a session, keep
  its token in storage.

Language-native clients (`reqwest`, `fetch`, Python's `urllib.request`) do not
work: there are no sockets and no `wasi:http/outgoing-handler`. Use this.

### `devices`

Your plugin's devices. Needs `devices = true`. Ids are `room/name`. Full
treatment in [devices](devices.md).

```rust
use niles::integration::devices;
use niles::integration::types::{Device, DeviceClass, LightCapabilities};

devices::upsert(&Device {
    id: "living_room/floor_lamp".into(),
    class: DeviceClass::Light,
    light: LightCapabilities { color_temp: true, rgb: false },
    controllable: true,
    state: empty_state(),
})
.map_err(|e| format!("{e:?}"))?;

let mut state = empty_state();
state.on = Some(true);
state.brightness = Some(60);
devices::report_state("living_room/floor_lamp", &state);

devices::report_action("hall/button", "single");
devices::set_available("living_room/floor_lamp", false);
devices::remove("living_room/old_lamp");
let mine: Vec<String> = devices::registered();
```

### `timers`

```rust
use niles::integration::timers;

timers::every("poll", 60);    // on_timer("poll") every 60 s, first in 60 s
timers::after("retry", 30);   // once, in 30 s
timers::cancel("retry");
```

`every` is at least 5 s, `after` at least 1 s. At most 32 timers. A timer with
the same id replaces the old one. Timers do not survive a restart of the
instance, which always runs `init` again, so set them there. If you want a
first run straight away, call your handler at the end of `init`.

### `events`

Things happening elsewhere in the house, delivered to `on_event`. Each kind
must be listed under `events` in the manifest's permissions.

```rust
use niles::integration::events;
use niles::integration::types::EventKind;

events::subscribe(EventKind::DeviceState, &["office/*".to_string()]);
events::subscribe(EventKind::Presence, &[]);
events::unsubscribe(EventKind::DeviceAction);
```

- `DeviceState`: another device's state changed. You get its full id
  (`z2m:office/door`), class, and only the fields that changed — including
  only the [readings](devices.md#readings) that changed, so a plug's `power`
  arrives without its `energy`.
- `DeviceAction`: a button or wall switch was pressed. `action` uses
  Zigbee2MQTT's words: `single`, `double`, `hold`, `on_press`, `up_hold`…
- `Presence`: the house became `Home`, `Away` or `Unknown`. Never who.

The patterns narrow device events by `room/name`: `office/door`, `office/*`,
`*/door`; an empty list means every device. You never get events for your own
devices. At most 256 events wait for you; past that the oldest are dropped and
the count is shown on your card, so keep `on_event` quick.

### `notify`

Speak through the satellites, by way of Niles's notifications. Needs
`notify = true`; `Urgent` needs `notify = "urgent"`.

```rust
use niles::integration::notify;

notify::say("The washing machine is done.", notify::Priority::Normal, Some("kitchen"));
```

- `Low` waits out quiet hours and is replayed in the morning; `Normal` plays
  quietly during them; `Urgent` plays at normal volume whatever the hour.
- `room` picks the room; `None` lets Niles choose (normally the satellite
  last spoken to).
- One or two short sentences, the important part first. It is spoken by a
  text-to-speech voice: write numbers and units as words would be read
  ("twenty degrees"), skip symbols and URLs.
- At most 6 an hour. Remember what you said (in storage) so a restart does not
  say it again.

### `ui`

The node types for pages and widgets, and one function:

```rust
use niles::integration::ui;

ui::invalidate("electricity");
```

which asks every app showing that surface to render it again. Cheap; see
[pages and widgets](ui.md#refresh).

### WASI

Toolchains import parts of WASI 0.2 whether you use them or not, so Niles
provides them, empty. What works: the wall clock and monotonic clock
(`SystemTime::now()`, `Instant`), randomness, stdout and stderr (to your log).
What does not: files (no directories exist), environment variables (none),
sockets and DNS (refused), `std::process`, threads.

## Coming later

Not in v0.1, and sketched in
[ARCHITECTURE.md](../../ARCHITECTURE.md#the-interface) for 0.2:

- **WebSockets**, held by Niles, which reconnects them and delivers each
  message to a new export, for devices and services that push.
- **Local TCP and UDP**: one request, one reply, to an address under
  `local_network`, for devices that speak something other than HTTP.
- **`wasi:http/outgoing-handler`**, so that `fetch` and other native clients
  work under the same checks.
- **A page of your own**: your static HTML in a sandboxed iframe, under a
  separate permission.
- **Reading other devices' state on demand**, not only through events.
