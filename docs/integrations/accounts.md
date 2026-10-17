# Accounts and webhooks

> New in interface 0.3. A plugin targets 0.3 with `interface = "0.3"` in its
> manifest and the [WIT](../../schemas/niles-integration.wit) of 0.3; see
> [publishing](publishing.md#interface-versions).

Some services need more than an API key. They have the person sign in on
their own page and hand back OAuth tokens, and they push what happens to an
address you give them instead of waiting to be asked. Interface 0.3 has
three pieces for that, each usable without the others:

- **Connecting an account** (the `callback` permission). Niles runs the
  browser's round trip and checks it. You build the service's sign-in
  address and exchange the code it sends back.
- **[`secrets`](host-api.md#secrets)** (no permission): an encrypted store
  you write, for the tokens.
- **A webhook** (the `webhook` permission): one secret address the internet
  may call, answered by your `on-webhook`.

All of it is built on Niles's **public address**, which the household sets
under Settings → API access: `https://home.example.dk`. Until it is set,
there is nothing to connect and no webhook. Making that address reachable
from the internet is the household's own doing (a tunnel, an ingress rule);
only `/plugins/*/hook/*` has to be.

## The manifest

```toml
id = "withings"
interface = "0.3"
min_niles = "0.3.0"

[permissions]
http = ["account.withings.com", "wbsapi.withings.net"]
devices = true
callback = true
webhook = true

[[settings]]
key = "client_id"
label = "Client ID"
type = "string"
required = true

[[settings]]
key = "client_secret"
label = "Client secret"
type = "secret"
required = true
```

`callback` needs at least one `http` host: the sign-in page has to be on one.
Both are refused under `interface = "0.1"` or `"0.2"`.

## Connecting an account

The household sees your plugin's **callback address** on its panel, to
register with the service (in Withings's developer dashboard, the Callback
URI), and a **Connect account** button. Pressing it:

1. Niles makes a single-use `state`, bound to that browser and that person,
   and calls **`authorize-url(callback-url, state)`**. Answer with the
   service's sign-in page, carrying both as the service names them:

   ```rust
   fn authorize_url(callback_url: String, state: String) -> Result<String, String> {
       let client_id = match config::get("client_id") {
           Some(config::SettingValue::Text(id)) => id,
           _ => return Err("Set the client ID first.".into()),
       };
       Ok(format!(
           "https://account.withings.com/oauth2_user/authorize2?response_type=code\
            &client_id={client_id}&scope=user.sleepevents&redirect_uri={}&state={state}",
           urlencode(&callback_url),
       ))
   }
   ```

   Niles sends the browser there only if it is `https` on a host your `http`
   permission declares. Anything else goes back to the panel with the reason.

2. The service sends the browser back to the callback address. Niles checks
   the `state` against the browser and the person, takes it out of the
   query, and calls **`on-callback(callback-url, query, person)`** with the
   rest as the service sent it: `code=…`, or `error=access_denied`.
   **Exchange the code there and then**: codes are often good for seconds
   (Withings's for 30). Keep the tokens with `secrets.set`:

   ```rust
   fn on_callback(
       callback_url: String,
       query: Vec<(String, String)>,
       person: Option<String>,
   ) -> Result<String, String> {
       let code = value(&query, "code").ok_or("Withings was not given access.")?;
       let tokens = exchange(code, &callback_url)?; // http::send to the token endpoint
       secrets::set("token", &serde_json::to_string(&tokens).map_err(|e| e.to_string())?)
           .map_err(|e| format!("{e:?}"))?;
       Ok(format!("Connected to Withings as {}", tokens.user_name))
   }
   ```

   Do not check `state` yourself: you never see it, and it was checked.
   `person` is who is signed in, when Niles knows. You have the usual 10 s.

3. What you return is shown on the panel: `ok` as it went, `err` as what
   went wrong.

**Refreshing.** A refreshed token is yours to keep. Many services (Withings
among them) issue a new refresh token each time and retire the old one, so
save the new pair with `secrets.set` before you use it. `set` replaces the
value whole or not at all: keep the access token, the refresh token and
when it expires together in one JSON value.

## The webhook

```rust
fn init() -> Result<(), String> {
    let Some(address) = inbound::hook_url() else {
        return Err("Set Niles's public address under Settings → API access.".into());
    };
    subscribe(&address)?; // give it to the service, once per address
    Ok(())
}
```

`inbound.hook-url()` is `<public address>/plugins/<id>/hook/<token>`. The
token in it is the only thing that authenticates a request. It survives
restarts, updates and turning the plugin off. **New address** on the panel
makes a new one, retires the old one and restarts your plugin, so `init`
hands the service the new address. Subscribing every `init` is the simple
way to stay right; most services accept the same address twice.

What arrives:

- **`HEAD`** is answered `200` by Niles without calling you: services check
  an address before registering it. A `HEAD` on your callback address gets
  the same, for the same reason.
- **`GET` and `POST`** call **`on-webhook(request)`**: the method, the
  query's pairs, the headers (lowercase names, no `cookie`) and the body,
  at most 64 KiB. Your `inbound-response` goes back as you made it: a
  status from 200 to 599, an optional content type, and a body of at most
  64 KiB. Echo a challenge if the service sends one.
- **Anything else** is a `405`, a wrong token a `404`.

```rust
fn on_webhook(request: InboundRequest) -> InboundResponse {
    let body = String::from_utf8_lossy(&request.body);
    match field(&body, "appli") {
        Some("50") => report("bed_in"),
        Some("51") => report("bed_out"),
        _ => return answer(400, "not a bed event"),
    }
    answer(200, "ok")
}
```

**Answer quickly.** You have 5 seconds, and the service is waiting.
Report what happened, answer, and do anything slow from a timer. Past the
deadline the caller gets a `503` and your plugin is restarted, as for any
call that overruns.

**Nothing is acknowledged that you did not answer.** While your plugin is
starting, restarting, turned off or failing, Niles answers `503`, so the
service keeps the event and tries again. A `5xx` of your own does the same.
Past 60 requests a minute (20 at once) Niles answers `429`; past 8 waiting
for you, `503`.

**What arrives is not to be trusted.** The token says only that the caller
knows the address. Most webhooks are not signed (Withings's are not), so
check what you can: the user and device ids you stored when the account
was connected, and a date newer than the last event you acted on.

**Not logged.** Niles writes a line per request to your log, with the
method, size, status and time (`hook POST 89 B → 200 in 12 ms`), and never
the token, the query, the headers or the body. Do not log them yourself
either; they may carry what your users would not want in a log.

## Trying it

`niles plugin dev` does not yet serve the webhook or the callback. Install
the plugin on a Niles whose public address is set and reachable, and watch
its log on the panel. A request website such as webhook.site shows what a
service sends before your plugin has to understand it.
