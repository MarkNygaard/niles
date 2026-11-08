# Niles architecture

This is the map and the rules. What exists is listed in `features.toml` (and the `MANIFEST.md` made from it); how a piece works and why is in its crate's doc comments; how to use Niles is the user guide in `site/content/docs/`, published at getniles.app; planned work and feature specs are Linear issues in the *Niles software* project. This file changes when a house rule, the crate map or a big decision changes.

## What Niles is

*Niles* (NYE-uls) is the project, the system and the wake word: one word, three roles. The acronym, **N**eural **I**ntelligence, **L**ightweight **E**dge **S**ystem, says what it is: an AI-first home assistant that runs on the household's own small hardware.

It replaces Home Assistant for a household that has a focused set of devices (mostly Zigbee through Zigbee2MQTT, a few Wi-Fi ones), wants voice that answers in well under a second, prefers config and naming to setup screens, and wants a language model built in rather than bolted on. Home Assistant solves a different problem, the widest device coverage for people who don't run infrastructure, and that makes it heavy and UI-led. Niles is opinionated and self-hosted: somebody in the house runs a container and a Postgres database, with Docker Compose or Kubernetes.

What Niles is not:

- **Not for every Home Assistant user.** Lots of vendor Wi-Fi devices, Z-Wave or Matter, or a family that needs a polished setup UI: stay on HA.
- **Not a design or planning surface.** Voice is a poor medium for code, design or long plans. Niles files the task (a Linear issue) and tells you when something is ready; the thinking happens at a computer.
- **Not a voice OS.** Niles is the brain; satellites are dedicated hardware that talk to it.
- **Not an LLM training framework.** It uses models; it doesn't train them.

## Design principles

1. **Speed first on the voice path.** Audio streams in and out, and a spoken command never waits on something it doesn't need: a plugin, a second model, a cloud call.
2. **Tiers by trigger, not always-on.** Tier 0 regex intents answer the common commands with no model at all. The fast model (Tier 1) takes the rest, with tools. A slower model (Tier 2, or the assistant) is used only when a turn calls for it.
3. **Tools, not RAG, for state.** The device registry is structured data the model queries with tool calls.
4. **One seam per device category.** A device source fills the shared `DeviceRegistry`. Commands go out through `niles_mqtt::CommandRouter`, which hands any device it doesn't reach over MQTT to its `DeviceCommander` (a plugin's, for example). A new protocol is a new source, not a rewrite.
5. **Satellites are appliances.** Identical firmware, no per-room state, easy to replace.
6. **The wake word is heard on the satellite** (microWakeWord, a model trained for "Niles"), so nothing streams until it is said.
7. **Voice answers come from the satellite, not the room speaker.** The Sonos is ducked while Niles listens and speaks. It is a tool the model uses, never the answer surface.
8. **The house is described upstream.** Rooms and devices are names in Zigbee2MQTT (or Niles's own WLED settings). The web UI edits values and presses switches; it never describes the house, and no screen creates a room. One-time setup in text (the ConfigMap, the manifests) is fine.
9. **The system that runs the home explains it.** One set of capability references (`docs/capabilities/*/SKILL.md`, compiled into the binary) grounds both how-to answers and command execution. They are loaded by topic, so a common command never carries them.
10. **A personal infrastructure surface.** Built-in integrations are crates in this repo, held to its tests. A remote MCP tool server brings a service's own tools. Sandboxed WebAssembly plugins from GitHub cover the long tail and reach only what the household approved.
11. **One shared home, several recognised people.** The house has one state: one set of devices, one curve, one set of scenes. People get personal *references* ("my calendar", "wake me up"), never per-person preferences for shared things.

## The crate map

A Cargo workspace of 39 crates; `niles-bin` is the only binary and wires the rest together.

**Core and state**

| Crate | What it is |
|---|---|
| `niles-core` | Device ids and types, the `DeviceRegistry`, the `EventBus`. No business logic. |
| `niles-config` | The TOML schema, its validation, and the layered `ConfigStore` (base plus overrides, hot or boot sections), and `Booted`, the fingerprint of what Niles started with. |
| `niles-db` | Every Postgres store: config overrides and their history, sealed secrets, conversations, artifacts, the approvals log, each person's notifications, timers, waits, voice prints and more. Each creates its own table on first use. |
| `niles-mqtt` | The MQTT client, the Zigbee2MQTT and WLED sources, and the `CommandRouter`. |

**Voice**

| Crate | What it is |
|---|---|
| `niles-wyoming` | The Wyoming server the satellites stream PCM to and get replies from. |
| `niles-stt` | Speech to text: ElevenLabs Scribe, or any OpenAI-compatible transcription server. |
| `niles-recognition` | Who is speaking: an ECAPA-TDNN embedder (ONNX Runtime), enrolment and matching. |
| `niles-intent` | Tier 0: the regex `IntentRouter`, and topic detection for capability references. |
| `niles-capabilities` | Loads the `SKILL.md` capability references from `docs/capabilities`. |
| `niles-llm` | One OpenAI-compatible chat client for Tier 1 and Tier 2, and the ChatGPT sign-in. |
| `niles-tools` | The `ToolRegistry`, the built-in tools, relevance groups, and the JSON-RPC of Niles's own MCP server for the assistant. |
| `niles-tts` | Piper text to speech. |

**What Niles learns**

| Crate | What it is |
|---|---|
| `niles-memory` | Hermes-style markdown memory (the household, and Niles's own notes), scanned before each write. |
| `niles-skills` | Skills the assistant writes and edits, with usage counts and a curator. |
| `niles-history` | The command and state history the model reads to answer "what did I just do" and to put the lights back how they were. Traits here, kept in Postgres by `niles-db` for 14 days. |

**The house**

| Crate | What it is |
|---|---|
| `niles-scheduler` | The lighting curve, manual mode, the morning routine, wall-switch actions, timers, scenes. |
| `niles-automations` | When-X-do-Y rules: the model, the engine, the run log, presence lights. |
| `niles-presence` | Who is home, from tado and UniFi, aggregated into the house's state. |
| `niles-speakers` | Sonos over SOAP/UPnP: rooms, favourites, TuneIn, Spotify search. |
| `niles-webos` | LG webOS TVs over their local websocket, woken by Wake-on-LAN. |
| `niles-notifications` | Unprompted speech at a satellite: priorities, chime, quiet hours, recall. |
| `niles-weather` | Open-Meteo forecasts and geocoding. |
| `niles-groceries` | The shopping list, and the catalogue of what the house actually buys. |
| `niles-nemlig` | nemlig.com's basket and delivery times, through its website's JSON. Never payment. |
| `niles-podcasts` | A podcast after the assistant's research: two Kokoro voices, MP3 frames joined without re-encoding, the file in the house's R2 bucket, presigned for playback. |

**People's own things**

| Crate | What it is |
|---|---|
| `niles-calendar` | Each person's calendars and mail: Microsoft Graph, Google, iCloud (CalDAV, IMAP, SMTP). |
| `niles-training` | Each person's workouts, from their own Strava app. |
| `niles-logins` | Each person's saved logins for Niles's browser: the sites' origins, the username and password, the browser profile. Kept in Postgres by `niles-db`, sealed. |
| `niles-artifacts` | What the assistant makes for a person: kinds, versions, pages, pictures, sharing. |

**Surfaces and extensions**

| Crate | What it is |
|---|---|
| `niles-api` | The HTTP API, sign-in, `/events/stream`, `/system/restart`, `/system/update`, and the web app's routes. |
| `niles-webpush` | Web Push to a person's devices, RFC 8291 and 8292 in pure Rust, and the record of what each person was told, for their Notifications page. |
| `niles-websearch` | The SearXNG client, and the fetcher that goes only to the public internet. |
| `niles-browser` | Niles's own Chrome DevTools Protocol client for headless Chromium in a container of its own: a browser context per session, each page read as text for the model, screenshots for the cards, and the `CONNECT` proxy the browser reaches the internet through. |
| `niles-mcp` | Niles as an MCP client, for remote tool servers such as Linear's. |
| `niles-plugins` | Third-party plugins: WebAssembly components run in wasmtime, installed from GitHub releases and found in the add-on library. |
| `niles-integration-linear` | Linear's webhook, turned into notifications. |
| `niles-tls` | TLS to devices that sign their own certificate (an LG TV). |
| `niles-update` | Which build runs, and whether a newer image of Niles is published: the `latest` manifest on GHCR and GitHub's compare API, asked anonymously. |
| `niles-bin` | The `niles` binary: `serve`, the dev subcommands, all the wiring, and the orderly stop (`shutdown.rs`). |

**Outside `crates/`:** `web/` (the React app, served by `niles-api`), `firmware/xiao-wyoming/satellite-idf/` (the satellite firmware), `hardware/satellite-v2/` (the custom satellite board), `schemas/` (the plugin WIT), `site/` (getniles.app: the landing page, the add-on library, the user guide in `site/content/docs/` and the plugin author guide in `site/content/docs/writing-add-ons/`), `docs/capabilities/` (capability references), `deploy/kustomize/` and `deploy/README.md` (deployment), and `examples/niles.example.toml`.

## How a voice turn flows

1. The satellite's XMOS DSP cleans the audio; its ESP32-S3 hears the wake word on the device and streams raw 16 kHz PCM over Wyoming TCP to `niles-wyoming`.
2. At the wake, `voice::hush_on_wake` ducks the room's Sonos to a quarter of its volume (`speak::Hush`).
3. `voice::spawn_dispatch_task` sends the utterance to `niles-stt` (ElevenLabs Scribe by default) while `niles-recognition` identifies the voice on a blocking thread beside it.
4. `dispatch::dispatch_transcript` drops what wasn't speech (no words, a hallucinated script, the transcriber's own noise gate), what didn't start with Niles's name when that is required, and an unknown voice when only known voices are answered. All of this runs before Tier 0.
5. `dispatch::text::dispatch_text` tries Tier 0: `niles-intent` turns the words into an `Intent`, which is acted on directly (a light through the `CommandRouter`, a timer, a scene, music).
6. On a miss, `dispatch::dispatch_tier1` builds the system prompt (`prompt.rs`: persona, capability references by topic, memory, skills) and runs `chat_loop` against the fast model (`niles-llm`, Cerebras by default) with the tools `niles-tools` judges relevant.
7. A spoken rule (caught by its words, or by the fast model reaching for an automation tool), or a tool server's tool the household left to the assistant, sends the turn to the assistant instead (Claude Code, or ChatGPT through pi; `assistant.rs`), which reaches the same tools over Niles's own MCP server. `escalate_to_tier2` re-runs the turn on Tier 2 if one is configured.
8. A tool set to Ask first, or a mail, is held as an approval (`approvals.rs`). A remote tool can be answered by a spoken yes; a mail waits in the app. Either way it is pushed at once to the person's own devices (`approvals/push.rs`).
9. `speak::speak_back` synthesises the reply with Piper (`niles-tts`) and streams it to the satellite. A reply that asks a question keeps the microphone open for the answer (`follow_up.rs`). The turn goes into the history, the journal and, for a recognised voice, that person's chat.

## How a chat turn flows

1. The app (`web/`) sends `POST /chat`, JSON or multipart with files, to `niles-api`. The session says who is typing; their address links them to their voice.
2. `app.rs` (`AppChat::reply`) finds the conversation in the chat log and seeds the context with it, spoken turns included.
3. Without files, `dispatch_text` runs as for voice: Tier 0 first, then `dispatch_tier1`, where a chat turn goes to the assistant when one is on, and to the fast model otherwise or when the assistant can't answer (with a note under the reply saying why). A turn that runs out of time in the middle of changing one artifact goes on as a job on it instead, and says so (`Artifacts::carry_on`).
4. With files, `dispatch::dispatch_attached` skips Tier 0 and the fast model: only the assistant can see them, and nobody else answers.
5. The assistant's tools come over MCP with a token for that run, which is how a tool knows whose turn it is. Artifact tools are offered only there. A change is said as edits, never the whole artifact again; a big artifact, a new page and a big change are background jobs. Each tool call of a chat turn is logged with its time, never its arguments.
6. The turn is kept with its approval card and artifacts. Later changes go to that person's connections on `/events/stream` (`approval_changed`, `artifact_changed`, `notification_added`), and a finished job reaches their Notifications page and their devices by Web Push. A card still waiting a minute after it was asked goes to their devices too. Allowing a browser step runs it and starts the conversation's next turn, and the answer comes back once that turn is done.

## How an automation flows

1. A rule is made by voice (the model reads it back and saves it on a yes), on the Automations page, or as `[[automation]]` in the config. The `niles-automations` store keeps the ordered set as one document in Postgres; the later of two conflicting rules wins.
2. The engine (`engine.rs`) follows the `EventBus` and the clock. A state trigger fires on the change into the state, and with `for` only once it has held. Conditions are read against the registry. A rule runs once at a time; firing again replaces the waiting run.
3. Actions go out through the `DeviceSink` (`MqttDeviceSink` in `niles-bin/src/wiring/automations.rs`, which uses the `CommandRouter`) and the `Notifier` (`niles-notifications`).
4. A delay is kept in `automation_waits`, and a `wake_up` action starts a wake-up (`wake_up.rs`) kept in `wake_ups`, so both survive a restart. The last 20 runs of each rule, including ones a condition stopped, are kept in memory to answer "why didn't it".
5. The curve and the morning routine are code, not rules: `niles-scheduler` computes them and `niles-bin/src/lighting.rs` ticks them.

## How an add-on is found

1. The author publishes releases as `site/content/docs/writing-add-ons/publishing.mdx` says, with a small `niles-installed` file beside the manifest and component, and gives the repo the GitHub topic `niles-addon`. One repo may hold several add-ons: each one's releases are tagged `<id>-vX.Y.Z`, and its address is `owner/repo/<id>`. The installer, the update check and `niles plugin check` then use that add-on's newest release by tag, never GitHub's "latest", and the install record keeps the name.
2. The registry repo, `MarkNygaard/niles-addons`, lists reviewed repos in `addons.toml`. A pull request that adds one runs `niles plugin check --listing <repo>`: everything an install checks, plus the marker. It puts the permission lines in the check's summary, and Mark merges by hand. Merged is **Listed**: it passed the checks and someone read what it asks for. That is not an audit of its code, and later releases aren't reviewed again.
3. An Action in the registry builds `index.json` every six hours and on every merge, and GitHub Pages serves it at `https://addons.getniles.app/index.json`, the project's own domain, so the repo can change owner without breaking any Niles. It holds the listed repos, every public repo with the topic as Unreviewed, and repos taken off the list (removed, with the reason). Each entry carries its latest release's manifest and logo as text, a README excerpt, its stars, and GitHub's download counts. An add-on of several in one repo is its own entry (index version 2, with `addon`), with its own counts and the repo's stars. Niles reads versions 1 and 2. The format is in `niles_plugins::library`.
4. `GET /plugins/install/library` fetches that one file when Browse or an add-on's panel asks, and keeps it six hours. It reads it as a stranger's: manifests go through the install's own parser, so permissions read as on the approval screen and `min_niles` is the install's own check; logos go through the logo allowlist; text is capped, and links are built from the repo. Browsing calls no GitHub API.
5. Browse's **Install** opens the paste flow with the address filled in: the same preview, checks and approval. An installed add-on's panel says Listed, Unreviewed or No longer listed.
6. After a fresh install succeeds, `Installer::install` downloads the release's `niles-installed` once per add-on (repo, and name in a repo of several) per house; `plugin_installs_counted` remembers, through a removal. A preview, an update or a reinstall never downloads it. GitHub's download count of that file across releases is the install count; without one, the library shows the component's downloads, labelled as such. This is not telemetry (rule 26): a public file is fetched from the address that just downloaded the add-on, and it carries nothing that download didn't.

## Restarting Niles

**What waits is worked out, not remembered.**

- At startup `niles serve` keeps a fingerprint (`niles_config::Booted`) of what it started with: each boot section's effective value, without the few settings inside one that are re-read (`path_reload`), and a digest, never the value, of the credentials its long-lived clients were built with (the MQTT login, the Tier 1 and Tier 2 endpoints and keys).
- A restart waits for whatever differs from that now. So a change from the app, by voice (an undo) or over the API counts the same, changing it back clears it, and every session gets the same answer.
- Enrolled voices are not in it: Niles's own enrolments apply at once, and only `niles enroll` from another process waits for a restart, which it says.
- A changed `niles.toml` is not noticed yet.

**`GET /system/restart`** answers what waits (`what`, the Settings label, `since`), this start's random boot id, the supervisor, whether the overrides are persistent, whether a restart is under way, and what a restart would cut short: background jobs, wake-up ramps under way (they pause and carry on), approvals waiting (they lapse). `restart_pending_changed` on `/events/stream` tells open apps to ask again. `restart_started` carries the boot id that is going away.

**A restart is Niles exiting cleanly with code 0, and its supervisor starting it again.** It needs no Kubernetes permissions.

- Offered under Kubernetes (`restartPolicy: Always`, found by `KUBERNETES_SERVICE_HOST`), or under whatever `NILES_SUPERVISOR` names (the Compose file sets `docker`; systemd needs `Restart=always`).
- With neither there is no button, only the message, because pressing it would only stop Niles.
- Without a database there is no button either: a restart would undo the very changes it would apply.

**The order of a stop**, the same for a restart, a SIGTERM or Ctrl-C:

1. Stop taking requests, voice turns and satellites.
2. Let the requests in flight finish (4 s).
3. Stop automations: no run starts, and a run waiting out a delay keeps its wait in the database. This comes before MQTT closes, so a delay that ends now can't run into a closed broker and then forget its wait.
4. Stop the add-ons (3 s).
5. Stop speaking (1 s).
6. Close the satellites' connections.
7. Send MQTT's DISCONNECT (1 s).
8. Exit 0.

A step that runs over its budget is left behind, so a stop takes at most about twelve seconds: within Kubernetes' 30 s grace, and past Docker's default 10 s only when several steps run long — what the SIGKILL then cuts is a goodbye (a DISCONNECT, a socket close), never a write.

**Under Kubernetes a restart can update Niles.** With `imagePullPolicy: Always`, the kubelet asks the registry every time it starts the container, a restart in the same pod included. So a pod on `:latest` starts the newest image, and the confirmation says it may. Restarting again within ten minutes waits out the kubelet's back-off (10 s, 20 s, 40 s …).

**Who may restart:** anyone who may change settings (a listed person or the API token), which is also NIL-13's rule for Update. Never by voice.

**In the app:** the bar at the top of Settings and a dot on its menu entry while something waits. The confirmation names what is cut short. Then "Restarting…" until `/system/restart` answers with another boot id, and a reload. After two minutes it says Niles hasn't come back and where to look (`kubectl logs deploy/niles`).

## Updating Niles

**Which Niles runs.** `image.yml` passes the commit and the image's `created` label to the build as `NILES_GIT_SHA` and `NILES_BUILD_TIME`, and `niles_update::Build` reads them with `option_env!`. `niles serve` logs them at start, and `GET /system/restart` and `GET /system/update` answer with them. A build without them, `cargo run` or a `docker build` without the arguments, is a dev build. It says so and never checks for a newer one.

**Whether a newer one is published.** `niles_update::Updates` asks GitHub without signing in. It checks two minutes after a start, every six hours after that (thirty minutes after a failure), and whenever About Niles opens. A check from the last half minute is reused.

1. An anonymous pull token from `ghcr.io`, then a `HEAD` of the `latest` manifest for its digest. A digest it already has ends the check, so a quiet check is two requests.
2. For a new digest, the manifest, the linux/amd64 image's config, and its `org.opencontainers.image.revision`, `.created` and `.version` labels.
3. For a revision other than this build's, GitHub's compare API between the two commits, once per pair. A failure is tried again after fifteen minutes. Its commits that aren't merges, newest first, are what's new. `development` is squash-merged, so they are pull request titles. Compare's `ahead_by` decides Update available. Without a comparison the later build date decides.

The answer is kept in memory. A failed check keeps the last answer and says how old it is. Offline is "couldn't check", never an error. Nothing in `niles.toml` switches it. The requests name Niles's public image and this build's public commit, and nothing about the house, which is why rule 26 allows them.

**`GET /system/update`** answers what is kept and never waits on the network. **`POST /system/update/check`** checks now. Both are behind sign-in. `update_checked` on `/events/stream` tells open apps to ask again.

**In the app.** Settings → System → About Niles shows the version, the commit (linked) and the build date. Then it says Up to date, or Update available with the newest image's date, What's new (newest first, each linked) and how to update where Niles runs. Settings and its About Niles entry carry a dot while an update is available. An open app reloads itself once when `/system/restart` answers with a commit other than the first one it saw. Its script is the old build's, and the app on an iPhone's Home Screen would otherwise keep it.

**For now, updating happens where Niles runs.** Niles has no permission to change its own Deployment, so About Niles shows how. On `latest` with `imagePullPolicy: Always`, a restart starts the newest image (see Restarting Niles), and so does `kubectl rollout restart deployment/niles`. A pinned tag is changed to the newest version or digest. Docker Compose pulls and starts again.

**The Update button is the second part of NIL-13, and is not built yet.**

- Optional manifests in `deploy/kustomize`, a component, give Niles a ServiceAccount and a Role with `get` and `patch` on the Deployment named `niles` in its own namespace, and nothing else. Without them there is no button, and the page shows the commands. That is the default and every GitOps setup. The permission is the switch.
- With them, Niles reads its own Deployment. On `:latest` with `Always` it sets the `kubectl.kubernetes.io/restartedAt` annotation, which is a rollout restart. On a pinned tag or digest it sets the image to the digest the page offered (`ghcr.io/marknygaard/niles@sha256:…`), so what runs is what was shown. A Deployment that Flux or Argo CD manages, known by their labels and annotations, gets no button, because they would revert the change. The page shows the line to change in git instead.
- The confirmation is Restart's. Niles is away for about a minute, satellites reconnect by themselves, and delays in automations are kept. It names what is cut short: background jobs, wake-up ramps and approvals waiting.
- A rollout keeps the old pod until the new one is ready. So after a bad image the old Niles keeps running, and the page says the update didn't take and how to see why. The new pod's log is not on the page, because reading it needs `list` on pods and `get` on their logs across the namespace.
- The same patch records the image it replaced in an annotation on the Deployment, and the page offers to go back to it. Schema changes stay additive (`create … if not exists`, `add column if not exists`), so going back one version is safe. A pull request whose migration isn't additive says so.
- Only the button updates Niles, never a timer. Who may press it is who may restart.

## Browsing on someone's behalf

NIL-74 built the browser, the proxy, sessions and the cards described below, and NIL-75 saved logins and My profile → Logins. NIL-76 signing in, sign-in codes, CAPTCHAs and browser profiles' use, and NIL-77 the order card are not built yet; what follows for those is their spec, and each of them rewrites its part of this section as what exists when it lands.

A person asks in the chat for something done on a website, for example "put a 2 m USB-C cable in my basket on shop.example". Niles opens and reads pages without asking. Every click, typing, choice and form submission waits on a card with a picture of the page, and runs only after the person's Allow. When the site needs a login Niles doesn't have, the person adds one in a form the model never sees, and Niles will fill it in only on that login's own origins (NIL-76). `read_page` stays the way to read a public page. The browser is for pages that need clicks, scripts or a login.

**The browser.**

- Headless Chromium runs in a container of its own, never in Niles's, and under Kubernetes in a pod of its own. Its image is the pinned upstream `chromedp/headless-shell:158.0.8127.18`, by version and digest (`sha256:830fa77e0602cb42e5cb2cbd9237ba27b649ad69d5fb82b54713aac8b7ab4f29`). Niles drives it over the Chrome DevTools Protocol (CDP) with a client of its own, `niles-browser`.
- `deploy/kustomize/components/browser` adds it, and so does `deploy/compose/browser.yaml`. Niles looks for `niles-browser:9222` every 30 s, or every minute while none is found, so adding the container is what turns browsing on, the way NIL-13's Role turns on the Update button. Nothing in `niles.toml` switches it (rule 6). Browsing needs the database too, because its cards are kept with the conversation.
- The browser tools are offered only while the browser answers. `browser_open` tells the model to say browsing isn't set up when no browser is there, to say it isn't answering when the browser has gone silent, and, for a spoken request, to ask in the app.
- Each session is a browser context of its own (`Target.createBrowserContext`) with `proxyServer` set to Niles's address on the browser's network (the local address of its DevTools connection) on port 3128, `proxyBypassList: <-loopback>` so even loopback goes through the proxy, and downloads denied. The context is disposed when Niles's connection drops, so the browser keeps nothing between sessions. A session holds one page; a page a step opens in a new tab replaces it. With no session open, Niles restarts the browser the first time after it starts and every 24 h after that, through `Browser.close` and the container's restart policy, so whatever a page leaves behind lasts a day at most.

**What each can reach.**

- The browser reaches the internet only through a proxy inside Niles, on port 3128. The proxy takes `CONNECT` to port 443 and nothing else, so the browser loads only `https` pages on their usual port. Its answers are 403 for a port other than 443 or a name and address that aren't the public internet, 405 for a method other than `CONNECT`, 502 for a name that doesn't resolve or an address that doesn't answer, and 503 when its 256 connection slots are full.
- For every connection, the proxy looks the name up itself, as an absolute name with a trailing dot, so the cluster's search domains can't turn a short name into a cluster Service. `.svc` names are refused. Then `read_page`'s address rules apply (`niles-websearch`'s `address.rs`): a name that is never the public internet is refused, and so is a name that resolves to any address that isn't public. The proxy connects to the checked addresses in turn, so DNS can't answer differently the second time. A redirect to another host is a new connection, checked the same way. So the browser never reaches the house's network, the cluster or Niles itself, and rule 13 holds for it.
- The proxy logs a refusal, with the host and the reason, and never what it let through.
- The browser is started so that nothing it sends goes around the proxy. Its own requests outside a session go to a dead proxy (`--proxy-server=http://127.0.0.1:9`); WebRTC's UDP is off (`--force-webrtc-ip-handling-policy=disable_non_proxied_udp`); and QUIC is off (`--disable-quic`).
- In Kubernetes, a NetworkPolicy is a second wall. It lets the browser reach only Niles's proxy, nothing else, not even DNS, because sessions name the proxy by address. It lets only Niles reach the browser's DevTools port, which has no login of its own. It holds only where the cluster's network plugin enforces NetworkPolicies (see the risks below). In Compose, the browser is on an internal network with Niles alone, and Docker enforces that. A page that takes Chromium over can still reach Niles's other ports on that network: the API holds to its login, and Wyoming takes a satellite's audio from anything that reaches it, as on the house network.

**A browsing session** is one browser context for one person, the turn's asker (rule 17), not one per conversation, so two conversations of the same person share its page. (Signing in is NIL-76, not built yet: a session will start with one of the person's saved logins or none, and a session signed in with a login will stay on that login's origins.) A session ends after sitting idle for 15 minutes, unless one of its cards waits, and a restart of Niles or of the browser ends every session. When the browser's connection drops, Niles lapses every step its cards hold.

**What the model gets.**

- Browsing is for the chat only, never for a background job. A spoken request to browse is told to ask in the app, because every step waits on a card there anyway.
- The model gets eight tools. `browser_open {url}`, `browser_read` and `browser_scroll {direction}` run without a card; `browser_scroll` takes `down`, `up`, `top` or `bottom`. `browser_click {ref, reason}`, `browser_type {ref, text, reason}`, `browser_choose {ref, option, reason}` and `browser_submit {ref, reason}` always wait on one. Each of those four names an element by its reference and gives the model's reason, and typing and choosing add the text or the option. `request_login {site, label}` asks the person for a saved login (below); `browser_sign_in` is NIL-76's. A page's address is upgraded from `http` to `https`; another scheme, another port, or an address carrying a login is refused.
- The model reads each page as text: the visible text, the links, the form fields with their labels, and a stable reference for each element it can act on. The read starts at the top of the view and runs to 12,000 characters, and `more_above` and `more_below` say which way the rest of the page lies. References stay the same for an element across reads of one page. Frames aren't read: a shown frame is a line saying so. The model never gets a picture. Screenshots go only on cards, for the person, because a scrub can find a saved value in text but not in pixels.
- It never gets a secret. A password field reads as masked, and typing into one is refused. (The scrub of a person's saved usernames and passwords from what Niles sends the model, the conversation log and tracing comes with signing in, NIL-76. Until then no saved value reaches a page, the model or a log.)
- Page text reaches the model marked as information, never as instructions (rule 12), and the chat loop logs a browser result's or a login request's size only, never its content.

**Every step is a card.**

- A step that acts on the page is held in `approvals.rs` like every approval, one at a time per session: a second step while one waits is an error telling the model to end its turn. Before Niles holds a step it checks the element is visible, enabled and of the kind the tool acts on, and, for a click, that nothing covers the point. Niles holds what it resolved, not the model's words: the page's address, the element the reference pointed to, the text or the option, and the model's reason (rule 10).
- The card's title asks the step: "Click "{label}"?" for a click, "Type into "{label}"?" for typing, "Choose "{option}"?" for a choice, "Submit "{label}"?" for a submission. Its subtitle is the page's host in its ASCII form, so an international name shows as `xn--…`. Its icon is the browser's, and its fields are the page's address, the element's label and kind, the text or the option when there is one, and the model's reason, marked as the model's. Niles re-encodes the screenshot as a JPEG and draws the element's outline on its own copy, never on the page, so the page can't move it. Niles keeps the picture in memory with the held step and drops it once the card is answered or lapses, and the conversation keeps the card without it. Only the asker sees the card (rule 18).
- While the card waits, its picture is served at `/me/approvals/{id}/picture` to the asker alone, with `no-store`, and it is gone once the card is answered or lapses.
- The card offers Allow once and Deny, and only an answer on the card in the app counts (`Allow::OnItsCard`, as for mail; rule 11). There is no Allow for this conversation and no spoken yes, and the card's notification offers Review instead of Approve.

**When the person allows a step**, Niles first checks that the session is still the card's, that the page's address is still what the card showed (compared without its fragment), and that the element is still the same node, by Chromium's backend node id, with the same kind, label, link and options. It checks the element is visible and enabled, and, for a click, runs a hit test at the point it will click.

- If they are, the step runs, and Niles starts the conversation's next turn, so nobody has to say "go on". The turn starts from a fixed message Niles writes, with no page text in it: what ran, and a request to read the page with `browser_read` before carrying on. It skips Tier 0 on purpose, so an element's label is never matched as a light command, and the app's answer request waits for it, run in a task of its own so a phone that locks mid-step leaves nothing half-answered.
- If not, nothing runs and the step lapses. When the same node is still on the page and can be acted on, a new card with a new screenshot takes its place. When it is gone or can't be acted on, the run ends.
- Alerts and leave-page prompts are accepted, and confirm and prompt dialogs are dismissed, because nobody allowed confirming: a step that needs a confirm confirmed thus does nothing.
- Deny ends the run. So does a card nobody answers, which lapses like any approval, and a restart, which lapses every waiting card (rule 9).

**Saved logins.**

- A saved login is one person's own. It has a label, a username and a password, when it was last used, and its origins. An origin is `https`, a public host and the usual port, kept in its ASCII form, with `http` read as `https`; anything else is refused because the browser reaches nothing else. A site that signs in on another host lists that one too.
- A login names 1 to 10 origins, and its label is at most 60 characters.
- The username and the password are sealed with the `Sealer` (`niles-db`'s `secrets.rs`). Postgres holds three tables, and removing a login removes them together: `saved_logins`, `saved_login_secrets` (the sealed username and password, in a table of their own) and `browser_profiles`.
- `GET /me/logins` lists the signed-in person's own logins as `{logins: [{id, label, origins, username, last_used, signed_in}]}`; `signed_in` means a browser profile is kept. No response holds the password.
- `POST /me/logins` adds one and answers 201. `PUT /me/logins/{id}` edits one and answers 200; an empty password keeps the current one. `DELETE /me/logins/{id}` and `POST /me/logins/{id}/sign-out` answer 204.
- An edit that changes the username or the origins forgets the browser profile.
- Remove and Sign out forget the profile. Ending a session open with the login comes with NIL-76, since until then no session uses a login.
- The API token gets 401, and another person's login answers 404 on every route (rule 18).
- The password is the credential rule 7 covers; no response type can carry one. The username is sealed too, and goes back only to its owner, never to the model or a log. `Debug` prints `…` for both values.
- Saved logins need the database and the secrets key. Without them, the Logins page says so.
- **My profile → Logins** in the app lists the person's logins, with Add, Edit, Remove and Sign out. The password field is write-only: it shows as set, leaving it empty keeps the current password, and Niles never sends it back.
- `request_login(site, label)` asks the person for a saved login, and is offered in the app chat while the browser answers and logins can be kept.
  - A saved login for the site answers at once with `{logins: [{id, label, site}]}`.
  - Otherwise the chat shows "Shop wants a login", Shop being the label, with **Add login**, kept with the turn and the conversation, so a restart doesn't lapse it and it stays answerable. The card's form shows the site's whole origin fixed.
  - The form posts to `POST /me/logins`, never through the chat, so the values are in no turn, log or conversation.
  - The app then sends only the new login's id to `POST /chat/conversations/{id}/login-requests/{request}`, which checks the login is the asker's and names the card's site, marks the card added once, and starts the conversation's next turn from fixed words, "Added a login", carrying `{id, label, site}`, as an allowed step does.
  - The model is told never to ask for a username or password in the chat.

**Signing in.**

- `browser_sign_in(login_id)` always waits on a card, which shows the site and the login's label, never the username. The login must be one of the asker's own, and anyone else's answers as if it didn't exist (rule 18).
- On Allow, Niles, not the model, finds the username and password fields, fills them and signs in. It fills a field only when the page's origin is one the login names. A script of Niles's own, in an isolated world in the field's own frame, checks the frame's origin and sets the value in one step, so a redirect, a look-alike site or a frame from elsewhere gets nothing.
- The check runs at the moment of filling and after every redirect, the way a password manager's does. A site that asks for the password on a later page gets it there, after the same check.
- The model learns whether the person is signed in, never the values.

**Sign-in codes and CAPTCHAs.**

- When a page asks for a code from a text message, a mail or an authenticator app, Niles shows a card, "Shop sent you a code", with a field for it. The code goes straight to the browser, never to the model or a log.
- A CAPTCHA stops the run, and Niles says the site wants a person. There is no live view. A live view would be a remote control for the browser, and a bot check is there to stop automated browsers. Niles doesn't disguise its browser either: no faked fingerprint and no CAPTCHA-solving service.

**Browser profiles.** A browser profile is a login's cookies and storage, one per person per login, sealed with the `Sealer` in Niles's database and never kept in the browser. Where they are kept (`browser_profiles`) and their dropping on Sign out, Remove and an edit of the account came with NIL-75; NIL-76 saves and restores them, and drops them after 30 days. The next session for the login starts with it, so the site doesn't ask for a code every time. Niles drops a profile 30 days after its login was last used, and at once on Sign out or Remove.

**The order card.** The step that places an order or pays gets a card of its own, titled "Place this order on shop.example?", with the total and the delivery address beside the screenshot. Niles reads both from the page, never from the model, and the card says so when it can't find one. Niles decides a step is an order from the page, by the button's label and the checkout around it, and errs towards the order card. Allow once and Deny work as on every card.

**Payment.** Niles uses only what the site's account has saved. It never asks for, stores or types card details, and a page that asks for them stops the run with a message. 3-D Secure is approved in the bank's app.

**Risks and limits.** The main risk is prompt injection. Page text can tell the model to do something else, and marking it as information (rule 12) doesn't make a model ignore it. Three things contain it:

1. Every step that acts is a card the person allows with the page in front of them, checked again at Allow.
2. The model never holds a secret, because Niles fills logins itself, and only on their own origins.
3. The browser reaches only the public internet, and a signed-in session only its login's origins.

The other limits:

- Opening an address needs no card, so a site that changes something on a plain visit, such as a link that adds to the basket, changes it without one. Placing an order takes a button or a form, and those are cards.
- A purchase takes about four to eight cards: adding to the basket, checkout, signing in, maybe a code, a delivery time, and the order itself. That is the price of a card for every step, which is why each card reads at a glance. The model moves between pages by opening the addresses links point to, which needs no card.
- A restart of Niles or of the browser lapses every held step and ends every session. A half-done purchase stops where it was, and the person asks again.
- Some sites refuse automated browsers, and Niles doesn't hide that it is one, so those sites refuse Niles. A site may ask an account that signs in from a new browser to verify itself, and the browser profile makes that rarer.
- A site that signs in or checks out on several origins needs each of them on its login.
- Chromium runs without its own sandbox, which needs user namespaces that containers usually refuse. The container is the sandbox (see Security notes).
- NetworkPolicies work only where the cluster's network plugin enforces them, and Flannel, Talos's default, doesn't. There the proxy holds only while Chromium does. Any pod in the cluster can drive the browser through DevTools, and a page that takes over Chromium can reach the cluster and the house network.
- A confirm a step needs is dismissed, so that step does nothing.
- Frames aren't read, so what a page shows only inside a frame the model never sees.
- Two conversations of one person share a session and its page.

## House rules

Every change keeps these. Each is one rule and, in italics, its reason.

**The house and its devices**

1. Devices are `<room>/<device>` in the source; Niles keeps no device database, and the UI stores only the source's ids. *There is nowhere else for a name to disagree.*
2. Core types use brightness `0..=100` and Kelvin; the source crate translates (Z2M 0–254, WLED 0–255, mireds). *One unit everywhere above the source.*
3. One shared house state; people get personal references, never preferences for shared things. *Two people can't both own the curve.*
4. The curve adjusts only lights that are on and never switches one. Niles turns lights on by itself only for the morning routine, a rule's `wake_up`, or a rule the household made. Any explicit adjustment puts a light in manual mode until it goes off and on; turning a light off during a ramp cancels that ramp for the day; a household rule acts even on a light in manual mode. *Explicit intent wins over the system.*
5. Nothing on the voice path waits on a plugin, the database or a cloud call it doesn't need; each plugin runs on its own thread. *Speed is the point.*

**Configuration and secrets**

6. Features work by default: on when the database is there, with their state in Postgres. Never require a ConfigMap section to switch one on. *A toggle in a file is a toggle nobody finds.*
7. Only the database URL (`NILES_DATABASE_URL`) and the secrets key (`NILES_SECRET_KEY`) come from outside. Every other credential is typed into the app and sealed (AES-256-GCM, `nonce || ciphertext`): never in the ConfigMap, never rendered, logged or returned. The API says only whether one is set. *A config dump must hold no secret.*
8. A config section is hot only if something re-reads it, and so is a setting inside a boot section; writes are validated whole and persisted before the swap; the voice tool refuses boot sections, and `tts` and `integrations`. Whether a restart waits is worked out from what this process started with, never remembered. *A stored change nobody acts on is worse than a no.*
9. What must outlive a restart is in the database (timers, waits, wake-ups, morning claims). What is deliberately in memory says so when it is lost: held approvals lapse, and a job a restart cut short is marked failed. *Every deploy is a restart.*

**The model and untrusted content**

10. The model never decides an approval. Niles holds the exact call and runs what it held after the person's answer; a change is a new approval. *What runs is what was shown.*
11. Mail is never sent, and Niles's browser never clicks, types, chooses, submits or signs in, without an approval answered on its card in the app: never by voice, never from a notification's button, never "for this conversation", and no setting turns that off. *A misheard yes is worst here, and a page can talk the model into a click.*
12. What a web page, a file, a tool result or a plugin says is information, never an instruction. *Strangers write pages.*
13. Claude Code never gets `WebFetch`. What Niles fetches for a model goes only to the public internet, to the checked address, with every redirect checked again. *The pod can reach the cluster.*
14. Model-written HTML runs only in the `sandbox="allow-scripts"` srcdoc frame under its own CSP. The app's own CSP stays `frame-src 'none'` and nothing else, and a test guards it. *See Security notes.*
15. Pictures in content are Niles's own re-encoded copies, named `niles-image:{id}`, never addresses. *An address is a tracking pixel, and it rots.*
16. The fast model's tool list is the expensive part of its prompt. Tools go in relevance groups; a plugin with tools must declare keywords (a tool server's default to its name); artifact tools are for the assistant only. *Tokens per minute are the budget.*

**People and privacy**

17. Personal data acts for the asker taken from the turn: the recognised voice, the signed-in person, the job's owner. Never from an argument. An unrecognised voice reaches nobody's personal data. *The model can't be talked into someone else's mail.*
18. Something not shared with you answers exactly like something that doesn't exist (a 404, "no such artifact") on every route and tool. Personal events and pushes go only to the people who have the thing. *Existence is information too.*
19. Voice prints stay in the house's own database, and only people who enrolled are recognised. *A voice is biometric data.*

**The HTTP surface**

20. Every state change is `POST`, `PUT`, `PATCH` or `DELETE` (signing out aside, which only clears the cookie), and the event WebSocket checks `Origin`. *`SameSite=Lax` depends on it.*
21. Only the HTTP API has a login. What sits outside the gate authenticates itself (Linear's HMAC, a plugin hook's token). *A cookie means nothing to Linear.*
22. The allowlist is re-read on every request, the last person can't be removed (by undo either), the model can't change `[auth]` (the store refuses a voice-sourced change, undo included), and the bearer token can't read or rotate itself. *Removing someone takes effect at once, a web page can't talk the model into letting a stranger in, and a leaked key can't lock the house out.*

**Extensions**

23. No plugin code runs in the web app (its UI is declarative); a plugin reaches only what its manifest declares and the household approved; nothing outside `niles-plugins` knows about WebAssembly. *The sandbox is one crate wide.*
24. Plugin-facing names stay as they are: `niles:integration`, `niles-integration.toml`, `/integrations`, `/plugins/…`, `integrations.*`. *Published plugins depend on them.*
25. No OpenSSL in the build: rustls and RustCrypto. *One build on every platform.*
26. No telemetry. If it is ever added, opt-in only. Two fetches carry nothing about the house and are not telemetry. The add-on library's install count is one `niles-installed` download per add-on per house, at install, from the host that install already reached. The update check reads Niles's own public image and the commits since this build from GitHub, and names only that image and this build's public commit. *Local-first is the point.*

**Code**

27. No traits without implementations; `#[non_exhaustive]` on public enums that grow; newtypes for validated identifiers; no compatibility shims before production; no half-finished implementations or `todo!()` paths. *See AGENTS.md.*

## Big decisions

- Postgres, not embedded SQLite (#16); stores create their own tables, and the database is just `NILES_DATABASE_URL` (#367).
- Layered config: a ConfigMap base plus overrides in Postgres, with history and undo, edited from the UI, by voice or over the API (#153, #154, #158; spec #168). Why Postgres: see Operations notes.
- Restarting from the app (NIL-14): Niles exits cleanly with code 0 and whatever runs it starts it again, offered only under Kubernetes or a supervisor `NILES_SUPERVISOR` names. What waits for a restart is a comparison with what Niles started with. **Rejected:** a remembered list of changes, which an undo or a change made elsewhere leaves wrong; and restarting through the Kubernetes API, which needs permissions this does not.
- Updating Niles (NIL-13). The image build bakes its commit and date into the binary. Niles asks GitHub without signing in whether `latest` is a newer build, every six hours and when About Niles opens, and lists the pull requests in between from GitHub's compare API. An open app reloads itself when the server's commit changes. The Update button, next, patches only Niles's own Deployment through an optional Role and pins the digest it offered. **Rejected:** updating unattended, which cuts jobs and approvals short with nobody watching and fights a GitOps setup; a Role wider than one Deployment, such as one that reads pod logs; and a server of Niles's own to say what's newest, which someone would have to run.
- Credentials typed into the app and sealed in Postgres (#209, #210); tado connected from the app by device flow (#203, #204).
- Custom ESP-IDF satellite firmware streaming raw PCM over Wyoming, not ESPHome, MQTT or wyoming-satellite (#103, #122, #123, #127); an on-device "Niles" wake word (#128).
- ElevenLabs Scribe for hearing (#281) and Cerebras for the fast model (#261); Groq removed, and the base system split from add-ons (#394). The optional Tier 2 is OpenAI (`gpt-5.5`) by default (#87).
- The assistant is Claude Code on the household's own subscription, reaching Niles's tools through Niles's own MCP server (#298); ChatGPT through pi beside it (#381).
- Capability references as agentskills.io `SKILL.md`, compiled into the binary (#44, #362), chosen by topic (#52, #57).
- Hermes-style memory and skills in Postgres, with a background review when the house is quiet and a weekly tidy (#77, #357, #358, #363, #364).
- Sign-in with GitHub against an allowlist kept in config; no roles, no passwords, no sign-up (spec #180, #185, #186). See Security notes.
- Sonos found from one speaker's address and placed room by room; music services are Sonos's services, not Niles's (#319, #320, #321, #322).
- WLED as a second MQTT device source (#110, #111).
- Automations as narrow JSON rules in Postgres with a flow-drawn page, and no scripting language, which would be a security and complexity hole (#342, #344); waits survive a restart, and a rule can start the morning routine (#431).
- Third-party add-ons as sandboxed WebAssembly components installed from GitHub releases (spec #384, #388, #390, #391); MQTT in 0.2 (#392); accounts and webhooks in 0.3 (#439, #444). **Rejected:** native Rust dylibs, because Rust has no stable ABI and a native library has the whole process's rights; and a subprocess or container per plugin, because it is sandboxed differently on every OS and Niles in Kubernetes can't start containers. A component is one portable file whose imports are all it can touch. Trust model: see Security notes.
- The add-on library (NIL-35): found by a GitHub topic, listed through pull requests to a registry repo, read from one `index.json` on GitHub Pages, and counted by GitHub's download count of a `niles-installed` file each fresh install fetches once. **Rejected:** a server of Niles's own, which someone would have to run and trust; counts reported by each Niles, which is telemetry; and the component's own download count, which previews and updates inflate.
- Linear through Linear's own MCP server, as a remote tool server; Niles files and queries issues and never talks to the ai-harness (#417, #419).
- Approvals: Ask-first tools and every mail wait for the person's yes (spec #422, #423); the panel and the history (#455).
- Approvals reach the person's own devices by Web Push (NIL-16). One asked in the open chat is pushed only if it still waits a minute later, and anything else at once. Every notification has Deny. Approve is only where a spoken yes would count too, and a mail gets Review instead. Answered or lapsed, it is replaced on the person's other devices by a quiet push with the same tag, never a hidden one, since Safari drops a subscription whose push shows nothing. iOS draws no buttons on a Home Screen app's notification, so a tap opens the card there. That comes from Apple's forums in October 2026 and is still to be checked on a current iPhone, along with whether iOS honours a quiet push.
- Accounts: one sign-in per provider serves Calendar, Mail or both (#424); mail (#425); iCloud over IMAP and SMTP (#426). App registrations: see Operations notes.
- Training from each person's own Strava app, not Garmin or Apple Health (#410, then #418). Why: see Operations notes.
- Artifacts made by the assistant and versioned per person (#429); pages in a sealed frame (#440); pictures as Niles's own copies (#442); sharing to view or to edit (#452); changes said as edits, and a change a chat turn runs out of time on carried on as a job (NIL-37).
- Web Push, with no inbound port (#432).
- SearXNG as the house's search; Claude's own `WebSearch` when there is none; `WebFetch` never (#451).
- Browsing on someone's behalf (NIL-72; the browser, the proxy and the cards in NIL-74). Headless Chromium runs in a container of its own, driven over CDP by Niles's own client in `niles-browser`, and reaches the internet only through a `CONNECT` proxy in Niles that applies `read_page`'s address rules. Every step that acts is a card, Niles fills a saved login itself and only on its origins, and the model reads pages as text and never holds a secret. CDP has what that needs: a browser context per session with its own proxy, isolated worlds to fill a login in, and screenshots for the cards. **Rejected:** Playwright MCP as a tool server, because the model would drive the browser through its tools with nothing of Niles's in between, its `--secrets` only replaces matching text in its responses (its README calls that "a convenience and not a security feature"), and its origin lists, by the same README, are not a security boundary and don't apply to redirects, so nothing would tie a password to its site; `chromiumoxide`, a client for the whole protocol (about 60,000 generated lines, slow to compile, by its README) where Niles makes a few dozen calls, some of them carrying a password, and Niles's own client never logs a call's parameters; Chromium in Niles's own container, where a page that took it over would find the secrets key and the database; and a sidecar in Niles's pod, which would share Niles's network, so no NetworkPolicy could hold it to the proxy.
- Chat attachments, sniffed by content and always answered by the assistant (#449).
- Blinds open to a normal level, and a second Open goes all the way (#441).
- Podcasts: the assistant researches and writes the script; the house's own Kokoro voices it, and the episode lives in a private R2 bucket the household set up, not on a hosted voice or storage service (#457).
- Release flow: `development` is the default branch, release-please cuts releases, images are built from `main` (#456).
- First-party add-ons live in the add-on library's repository, `MarkNygaard/niles-addons`, under `addons/`, built against a copy of the published WIT and checked with the published Niles image. Each is released on its own (tags like `bambulab-v0.2.0`) and installs as `MarkNygaard/niles-addons/<name>`. **Rejected:** an `addons/` folder in this repo, whose releases, tags and release PRs would crowd Niles's own (NIL-48, NIL-50).
- The website, getniles.app, is one static Next.js and Fumadocs site in `site/`: the landing page, the docs and the add-on library, built and hosted by Vercel from `main` (with a preview for every pull request) and rebuilt daily. It replaced the mdBook, and it shares the app's theme file (NIL-41).
- Self-hosted two ways, both supported: Docker Compose on one Linux machine (`deploy/compose`, which CI starts on every pull request) and Kubernetes with Kustomize (`deploy/kustomize`). The Compose stack runs Niles and Postgres only; the broker, Zigbee2MQTT and Piper stay the household's own (NIL-44).

## Security notes

**Artifact pages.** A page may be written by a model that has just read web pages, so it is trusted with nothing. It runs in a `sandbox="allow-scripts"` srcdoc frame with an opaque origin, under a CSP that allows inline script and style, `data:` and `blob:` images and media, and nothing else (`connect-src 'none'`, `webrtc 'block'`). Its only way out is a message the app reads as a stranger's. The app's own policy is only `frame-src 'none'`, which stops the frame navigating itself to an address that carries what it knows.

- **Tested in Chrome 155** against a page that tried every way out (fetch, XHR, sockets, beacons, remote images, CSS and fonts, every link hint, child frames, workers, WebRTC, popups, the top window, forms, `<a ping>`, cookies and storage, navigating itself): nothing reached a server, and only self-navigation did once the app's `frame-src` was removed.
- **What is left, and why it is small.** A script can still add its own `dns-prefetch` or `preconnect` link, and not every browser holds those hints to a policy, so a DNS lookup could carry a few bytes out. A determined script might find the WebRTC constructors again in a child frame. Either could carry only what the page has: this artifact's data, for the person looking at it. No session, no other artifact, nothing of the house.
- The details are in `web/src/lib/sandboxedPage.ts` and `crates/niles-artifacts/src/page.rs`.

**If Niles is ever exposed to the internet.** Only port 8080, the HTTP API, goes out; Wyoming and MQTT stay on the house network, and the satellite streams audio unauthenticated. **`/logs` changes character:** at home it shows voice transcripts to people already in the house; exposed, it shows them to anyone holding a session. That is a reason to keep the allowlist short, not to hide the route. Exposing a plugin's webhook (`/plugins/*/hook/*`) is not exposing Niles; the app can stay at home. Pushes to a phone need no inbound port at all.

**The allowlist amends itself, by design.** There are no roles, so anyone signed in can let anyone else in. That fits two adults sharing a home and is the wrong shape for anything larger: **revisit it first if Niles ever has a user who is not a householder.** If the list is ever emptied some other way, the bearer token is not subject to it, and `PATCH /config` restores it. People listed with no GitHub app is sign-in off, not failed closed (that would leave the bearer token as the only way in); the People card and the startup log say so. Anyone signed in can also restart Niles (`POST /system/restart`), the same people who can change any setting, and NIL-13's Update follows the same rule; nothing restarts it by voice.

**The plugin trust model.** The sandbox limits what a plugin can *reach*; it does not make a plugin *trustworthy*. An approved plugin can do anything its declared hosts and devices allow: one given an account's password can do what that account can, one subscribed to device state learns when the house is empty and can send that to any host it declared, and any plugin can tell the model untrue things in a tool result. What the sandbox guarantees: no undeclared hosts or local addresses, no other integration's secrets, no database, conversations, memory or people, no code in the app, and capped memory and time. So the approval screen says plainly what is granted, the repo is shown, the component is pinned, and an update that asks for more is approved again.

**The add-on library.** Listed means an add-on passed `niles plugin check --listing` when it was listed and someone read its permissions. It is not an audit of its code, and later releases aren't reviewed again. Nothing about installing changes: Browse's Install is the same preview and approval as a pasted address, and an update that asks for more is approved again. The index is a stranger's file. Niles parses its manifests with the install's parser, sanitises its logos with the install's allowlist, shows its text as text, and takes no address from it.

**The browser and its proxy (NIL-72).** The browser runs pages strangers wrote, steered by a model that reads them, so it holds as little as possible. Chromium's own sandbox needs user namespaces, which containers usually refuse, so Chromium runs without it and its container is the sandbox. The container runs as user 65534, not root, with a read-only root filesystem, no capabilities and no service-account token, and it holds no secrets key and no database address. A page that took over Chromium would hold the cookies of the sessions open at that moment and nothing more, because the browser keeps nothing between sessions and restarts daily when idle. (NIL-76 saves browser profiles, sealed in Niles's database.) The browser's traffic leaves only through Niles's proxy, which passes only `CONNECT` to port 443 on the public internet, to the address it checked. HTTPS runs through the proxy end to end, so the proxy sees hosts and ports and nothing inside, and it logs refusals, never what it let through. The browser's own requests outside a session go to a dead default proxy, so a page can't send anything around the proxy. The NetworkPolicy keeps a taken-over browser from going around the proxy: it allows no DNS at all, because sessions name the proxy by address, and it keeps everything but Niles off DevTools, which has no login of its own. That holds only where the network plugin enforces NetworkPolicies. Flannel, Talos's default, doesn't, and there any pod can drive the browser and read its open sessions, and a taken-over browser can reach the cluster and the house network. A plugin that enforces them, such as Cilium or Calico, closes both. In Compose, Docker enforces the internal network, but there is no NetworkPolicy: a taken-over browser can reach Niles's other ports on it. The API holds to its login, but Wyoming takes a satellite's audio from anything that reaches it, as on the house network. The model never holds a saved value, so a page that fools it can't make it leak one. What is left is the person. The Add login form shows the site's whole origin, fixed when a card asked for it, and a password saved for a look-alike site goes to that site.

## Operations notes

**Config overrides live in Postgres**, not on a node-local volume. A local volume pins the pod to one node and blocks drains, which on a Talos cluster means every upgrade. Voice prints are in Postgres for the same reason. Without a database, changes still apply but don't outlive the process, and every surface says so.

**Why Strava.** Garmin has no API for an individual (its developer programme is for businesses), and the unofficial login that libraries such as garminconnect used broke behind Cloudflare in March 2026. Apple Health has no web API; the Health Auto Export app that first fed Niles charges for its REST automation and runs only while the phone is unlocked. Strava has a web API, and Garmin Connect uploads every workout to it. A new Strava app serves only the athlete who made it, so **each person makes their own app** and enters its Client ID and Secret on their profile. Strava gives activities only: no sleep, HRV or weight.

**The household's app registrations** for accounts (Calendar and Mail):

- *Google:* the OAuth app must be **published**, not left in testing, where Google expires sign-ins after seven days. Unverified, it shows "Google hasn't verified this app", which is expected. Reading Gmail is a *restricted* scope, which an unverified app may hold for at most 100 people.
- *Microsoft:* register in Entra for "accounts in any organizational directory and personal Microsoft accounts". A work tenant may still refuse a person's consent to an outside app; that is the employer's call.
- *Both:* changing what an account serves means signing in again for the new scopes. iCloud's app-specific password can't be narrowed, so its read-only access is Niles's own rule, not Apple's.

**Deploying** is `deploy/compose/` (Docker Compose, with its README) or `deploy/README.md` and the Kustomize manifests in `deploy/kustomize/`. A fresh install needs the broker's address and login and the hearing and thinking keys before its first start, because `serve` connects to the broker and builds both clients before the API is up; the shipped `niles.toml` files name those variables.

## Glossary

- **Satellite**: a per-room device (reSpeaker XVF3800 with a XIAO ESP32-S3) that hears the wake word, streams audio to Niles and plays the reply.
- **Wyoming**: line-delimited JSON over TCP, with binary audio frames, between satellites and the server.
- **AEC / DSP**: the XMOS chip's echo cancellation and audio cleanup, so the satellite hears you over music.
- **Tier 0 / 1 / 2**: regex intents; the fast model with tools; the optional slower model a Tier 1 turn can escalate to.
- **The assistant**: Claude Code, or ChatGPT through pi, on the household's own subscription, reaching Niles's tools over MCP.
- **Device naming convention**: `<room>/<device>` names set in the source, the only description of the house's structure. A full id adds the source: `z2m:kitchen/ceiling`, `wled:…`, `x_<plugin>:…`.
- **The curve**: the daily brightness and colour-temperature function every light that is on follows.
- **Manual mode**: a light someone adjusted on purpose; the curve leaves it alone until it goes off and on.
- **Morning routine**: lights brought up from off on a schedule, or by a rule's `wake_up`.
- **Scene**: a saved set of light states for a room or the house; applying one puts its lights in manual mode.
- **Capability reference**: a `SKILL.md` in `docs/capabilities/` that tells the model how a feature works, chosen by topic for each turn.
- **Hot / boot section**: a config section re-read while running, or one read only at startup. A few settings inside boot sections are re-read (`path_reload`).
- **Boot id**: a random id each start of Niles gets, in `GET /system/restart`; the app knows Niles is back when it changes.
- **Build**: the commit and time an image was built from, which the binary knows (`niles_update::Build`). A binary built any other way is a dev build.
- **Approval**: a tool call or mail Niles holds until the person answers.
- **Artifact**: something the assistant makes for one person and keeps, with every version: a document, a training plan, a trip.
- **Personal reference**: "my calendar", "wake me up": resolved to the speaker's own resource, as opposed to a preference.
- **Voice print**: the 192-float embedding of a voice, matched against the enrolled ones.
- **Notification**: something Niles says unprompted at a satellite, with a chime and quiet hours; a person's own things also go to their devices by Web Push.
- **Automation**: when X, do Y. Rules are stored in Postgres; the curve and the morning routine are code.
- **Tool server**: a remote MCP server whose tools Niles offers, named `<id>__<tool>`.
- **Add-on library**: the add-ons other people published, read from the registry's `index.json` and shown under Add-ons → Browse. Each is Listed (reviewed into the registry), Unreviewed (found by the `niles-addon` topic) or, once taken off the list, No longer listed.
- **Plugin**: a WebAssembly component installed from a GitHub release, targeting the `niles:integration/plugin` world in `schemas/niles-integration.wit`.
- **features.toml / MANIFEST.md**: the catalogue of what Niles does, and the LLM-facing file `niles generate-manifest` makes from it.

## Where the details are

- **How a piece works, and why:** its crate's doc comments.
- **What exists:** `features.toml`, and `MANIFEST.md` made from it.
- **Planned work and feature specs:** Linear, the *Niles software* project.
- **How to use Niles:** the user guide in `site/content/docs/`, at getniles.app/docs.
- **Writing a plugin:** `site/content/docs/writing-add-ons/`.
- **The long spec this replaced:** git history.
