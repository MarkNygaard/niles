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

A Cargo workspace of 37 crates; `niles-bin` is the only binary and wires the rest together.

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
| `niles-artifacts` | What the assistant makes for a person: kinds, versions, pages, pictures, sharing. |

**Surfaces and extensions**

| Crate | What it is |
|---|---|
| `niles-api` | The HTTP API, sign-in, `/events/stream`, `/system/restart`, `/system/update`, and the web app's routes. |
| `niles-webpush` | Web Push to a person's devices, RFC 8291 and 8292 in pure Rust, and the record of what each person was told, for their Notifications page. |
| `niles-websearch` | The SearXNG client, and the fetcher that goes only to the public internet. |
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
6. The turn is kept with its approval card and artifacts. Later changes go to that person's connections on `/events/stream` (`approval_changed`, `artifact_changed`, `notification_added`), and a finished job reaches their Notifications page and their devices by Web Push. A card still waiting a minute after it was asked goes to their devices too.

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

This is NIL-10's spec, and none of it is built yet. It lands in the steps at the end of this section. Each step rewrites the part it builds as what exists, and moves its rules into the house rules.

A person asks in the chat for something on a website, like "add this to my basket on amazon.de". The assistant opens and reads pages freely. Every click, typing and choice it wants to make on a page waits as an approval with a picture of the page, and runs only after the person's Allow once. When a site needs a login Niles doesn't have, the chat shows an **Add login** button. The person types the username and password into a form that posts straight to Niles's sealed store, and the model never sees them. The model can still ask Niles to sign in with them, and Niles types them only on the login's own sites. `read_page` stays the way to read a public page. The browser is for pages that need clicks, scripts or a login.

**The browser.** Headless Chromium runs in a container of its own, never in Niles's.

- Niles drives it over the Chrome DevTools Protocol (CDP) with a small client of its own in a new crate, `niles-browser`. Some of its messages carry a password, so the client never logs a message's parameters, and its errors never quote them.
- Niles finds it at `niles-browser:9222`, the name the Kubernetes Service and the Compose service share. The Service is headless, so the name resolves to the browser's own address. Niles looks for it in the background, at start and every five minutes, never during a turn.
- The container is the switch, as the Role is for NIL-13's Update button: a Kustomize component (`deploy/kustomize/components/browser`) and a Compose file to add beside the main one. Without it, Niles offers no browser tools, and My profile → Logins says the browser isn't set up and how to add it. Browsing also needs the assistant.
- The image is Debian's `chromium`, built by `image.yml` beside Niles's own as `ghcr.io/marknygaard/niles-browser`, and rebuilt every week for Chromium's security fixes. Niles ends the browser once a day when no session is open, so its supervisor starts it again, on the newest image where the pull policy asks the registry at every start.
- It doesn't hide that it is automated: no fingerprint masking and no CAPTCHA services. A site that turns it away is a site Niles can't use, and Niles says so.

**What the browser can reach.** Rule 13 holds for it.

- Every request a page makes goes through a forward proxy inside Niles. The proxy takes `CONNECT` to port 443 and nothing else. It resolves the name itself, checks the address with `niles-websearch`'s public-address rules, and connects to the address it checked. So the browser speaks only HTTPS, and a name that resolves into the cluster or the house gets nowhere.
- Niles makes each session's browser context with that proxy, loopback included (CDP's `Target.createBrowserContext` takes one). The proxy's address is Niles's own address on the DevTools connection, so the browser needs no DNS. Chromium's own requests, outside any session, go to a proxy that leads nowhere, and WebRTC is barred from UDP, which would go around the proxy.
- The proxy serves only connections from the browser's own address. It logs a refusal with the host and why, never what passed.
- A second wall: in Kubernetes, NetworkPolicies let the browser reach only Niles's proxy port and let only Niles reach the browser's DevTools port, where the cluster enforces them (see Security notes). In Compose, the browser is on an internal network with Niles alone.

**Sessions.**

- A session is one person's browser context, for one of their saved logins or for none. Each person has one session at a time. Opening a page with another login, or after `browser_close`, starts a new one.
- A session for a login starts with the login's browser profile (below) and stays on the login's sites, as browsers count sites: `checkout.amazon.de` is the same site as `www.amazon.de`. Niles stops a top-level page on any other site before it loads, because a page elsewhere could send requests that carry the session's cookies. The proxy's check still covers frames and a page's other requests.
- A session closes ten minutes after its last step, and never while one of its cards waits. Niles refuses downloads, file uploads and permission prompts. A new tab a page opens becomes the session's page.
- A restart of Niles closes every session. At its start, Niles closes whatever contexts it finds in the browser. Held steps lapse (rule 9), and `GET /system/restart` counts open sessions among what a restart cuts short. If the browser restarts, its sessions end and their cards lapse.

**What the model gets.** The browser tools are the assistant's only, like the artifact tools (rule 16). They act for the turn's asker (rule 17), who must be a signed-in person. A person asks for browsing in the chat. Niles answers a spoken request with "ask me in the app", because every step waits on a card there anyway. Background jobs don't browse.

- The tools are `browser_open` (an `https` address, and optionally a login), `browser_read` (the page again, scrolled, or its next part), `browser_click`, `browser_type`, `browser_select`, `browser_sign_in`, `browser_ask_for_login`, `browser_ask_for_code`, `browser_logins` and `browser_close`.
- The model reads text, never pictures: the page's address and title, what it says, and every link, button and field, each with a short reference the model acts on. Pictures are for the person's cards, because a filter can find a saved value in text but not in a picture.
- A password field's value never appears, and a field Niles filled reads "filled by Niles". As a backstop, Niles scrubs everything it sends the model of the session's saved username and password and of any code the person typed, wherever one appears as a whole word, in any case or percent-encoded.
- What a page says is information, never an instruction (rule 12). A capability reference, `docs/capabilities/browsing/SKILL.md`, tells the model to move around by opening the addresses a page links to, and to use a site's search address rather than its search box. It also tells the model never to ask for a password in the chat, and to stop at a CAPTCHA.

**Every step is a card.**

- Every click, typing and choice (`browser_select`) waits on a card, and so does every sign-in and code. A click on a plain link waits too, because Niles can't tell a link from a button a script made to look like one, and sites attach their handlers to the whole page. Opening an address, reading and scrolling don't wait, so the model moves around without a card by opening the addresses the page links to.
- Niles holds what it resolved, not the model's words: the session, the page's address, the element (its node, role, label and box), the text or option, and the model's reason. What runs is what was shown (rule 10). Niles holds a step only for an element a person could see and click: on the page, with a size, not hidden and not transparent.
- The card's title is the step and the element's own text, from the page and not the model, for example Click "Add to basket"? or Type "usb-c cable 2 m" in "Search Amazon.de"?. An element with no visible text uses its accessible name. The card shows a picture of the page with the element outlined, the site, the login (its label, never the username) and the model's one-line reason, marked as the model's. A label is the page's text, so the card cuts it to 80 characters and removes invisible and direction-changing characters. A site in another script shows as its `xn--` form, and an element in a frame from another site says which site.
- The card has Allow once and Deny, and only an answer on the card in the app counts (`Allow::OnItsCard`, as for mail). There is no "for this conversation" and no spoken yes, and its notification has Review instead of Approve.
- The picture is Niles's own JPEG, with the outline drawn on Niles's copy, never on the page. A field Niles filled is covered in it, and a password field shows only dots. Niles keeps the picture in memory with the held step, serves it only to the asker, and drops it once the card is answered or lapses. The chat log keeps the card without it, and the approval log keeps its title and one summary line, as for every approval.
- A session has one card at a time, and Niles refuses a step asked for while one waits.
- Allow runs the step, then starts the conversation's next turn with the page as it now is, so the model asks for the next step without the person saying go on. Niles carries on as itself, never as words the person typed. Deny, or a card left to lapse, ends the run there, and the person says what next.

**When the page changed.** When the person allows a step, Niles first checks that the session is still open, that the page's address is exactly the one the card showed, that the element is still there with the same role and label, and, for a click, that the point it will click is on that element with nothing covering it.

- If all of that holds, Niles acts. A click is a real mouse click at the element's centre, after scrolling it into view.
- If the address is the same but the element isn't, and exactly one element now has that role and label, the old card lapses with "The page changed", and a new card with a new picture takes its place.
- Otherwise the step lapses, and the model gets the reason and the page as it is now. Niles never clicks something the person didn't see.

**The order.** A click on a button whose label orders or pays gets a card of its own, Place this order on Amazon.de?. EU law makes a shop label that button plainly ("order with obligation to pay", in German "zahlungspflichtig bestellen"), and Niles knows those labels and the common ones ("Buy now", "Place your order"). The model can also mark a click as an order, which only ever adds this card and never removes it. The card shows the total and the delivery address as Niles found them on the page, never as the model said them, and says so when it found neither. Its picture is the whole page, not only the part in view.

**Payment.** Niles holds no card numbers and types none. A purchase uses what the shop account has saved. If the bank asks for 3-D Secure, the person approves it in the bank's app, as they would anyway. A 3-D Secure page that wants a click or a code gets a card like any other page.

**Saved logins.** Each is one person's own, in two `niles-db` tables, `browser_logins` and `browser_profiles`.

- A login has a label ("Amazon.de"), one to five sites, and a username and a password, each sealed with `Sealer`. A site is typed as `www.amazon.de` or as a full address, and kept as its exact `https` origin (`https://www.amazon.de`). A login's browser profile, its cookies, is a row in the second table, deleted with the login.
- `GET /me/logins` lists the person's own logins: label, sites, username, when each was last used, and whether a browser profile is kept or a session is open. `POST /me/logins` adds one, `PATCH /me/logins/{login}` edits one, `DELETE /me/logins/{login}` removes one, and `POST /me/logins/{login}/sign-out` forgets its browser profile. No endpoint returns a password. The username is the account's name, which its owner sees on the page; the password is the credential rule 7 covers.
- Only the person's own signed-in session reaches these routes. Another person's login answers 404 (rule 18), and the API token reaches none. Logins need the database and the secrets key, and without them the Logins page says so.
- My profile → Logins shows the list, with Add, Edit, Remove and Sign out. The password field is write-only: it shows as set, leaving it empty keeps the current one, and Niles never sends it back to the browser. An edit takes effect at the next sign-in, and a session already open is left alone. A new username or new sites also forget the browser profile, and a session open at the time keeps no cookies when it ends.
- Remove deletes the login and its browser profile together. Sign out forgets the profile and keeps the login. Both end an open session that uses the login, and lapse its card. Niles forgets the cookies but doesn't sign out on the site, so the site's list of devices may show the browser until the site expires it.
- The model sees only each login's id, label and sites, and whether a profile is kept (`browser_logins`).
- When a site needs a login the person hasn't saved, the model asks with `browser_ask_for_login`, and the chat shows a card, "www.amazon.de wants a login", with **Add login**. The site is the one the session's page is on, never the model's word, and if the person already has a login for it, Niles tells the model instead. The button opens the same form as the Logins page, which posts straight to `POST /me/logins` with the card's id, outside the conversation. So the values never enter a turn, the chat log or a log line. Saving answers the card, and the conversation carries on as after an Allow, with the model told only the new login's id, label and sites.

**Signing in.** `browser_sign_in(login)` works only in a session for that login, on a page whose origin is exactly one of its sites and that has a sign-in form Niles can find. Otherwise Niles tells the model why at once. Signing in is a card like any step, Sign in to Amazon.de?, with the site, the login's label and a picture of the empty form.

- At Allow, Niles finds the username and password fields and sets each value with a script of its own. The script runs in the field's own frame, in a world the page's scripts can't touch (a CDP isolated world). In one step it checks that the frame's origin is exactly one of the login's sites, and sets the value. The page around the frame must be exactly one of the login's sites too. If the page has moved on, the script has nowhere to run and Niles types nothing, so a redirect, a lookalike site or a frame from elsewhere gets nothing.
- Niles sets the values rather than typing them key by key, then submits with Enter. A site that asks for the password on a second page, after the username, gets it there after the same check, up to three pages. Anything else on the form, like a "keep me signed in" box, is the model's click, held like any other. `browser_type` refuses a password field, so only a sign-in fills one.
- The model then gets the page as it now is. Niles can't always tell a wrong password from a success and doesn't guess, so the model reads the page.
- Inside Niles a saved value lives in a type whose `Debug` prints `…` and that has no `Display` or `Serialize`, as `niles-nemlig`'s credentials do, so it can't reach a log or a response by accident.

**Codes and CAPTCHAs.**

- When a site sends a code by text message or mail, the model asks with `browser_ask_for_code(field)`, and the chat shows a card, "Amazon.de sent you a code", with a field for it. The code goes straight to Niles, which sets it the way it sets a password, into that field on the login's sites, and submits. Only the asker's own session answers the card, with Send or Cancel, and the card lapses after ten minutes. Niles scrubs the code like the password and never logs it.
- A CAPTCHA, or any check that wants a person at the page, stops the run, and Niles says so. Niles knows the common ones by their frames and pages, and the capability reference tells the model to stop at any other. There is no code card for a CAPTCHA.

**Browser profiles.** After a sign-in that leaves no sign-in form on the page, and when a signed-in session ends, Niles keeps the session's cookies and the local storage of the login's sites, as JSON sealed with `Sealer`, up to 1 MB. The next session for the login starts with them, so the site doesn't ask for a new device's code every time. Niles forgets a profile 30 days after the login was last used, on Sign out or Remove, and when the username or sites change. The site may expire it sooner, and then the next session signs in again, with its card.

**The four open questions, decided.**

- Clicks that only navigate wait on a card too, as "Every step is a card" says.
- Logins are each person's own, never the household's. A login is personal data, so the asker's own is used (rule 17), and only they answer its cards. A household that shares an account saves it under each person, and each gets their own cookies.
- There is no live view, and a CAPTCHA stops the run. A live view would stream the page to the phone and send taps back. That is a remote control of the browser, large to build, and solving a bot check for an automated browser is what the check exists to stop. Revisit after step 5 if amazon.de asks every time.
- A browser profile lasts 30 days from the login's last use.

**The rules it adds.** Each step moves these into the house rules.

- Step 2 rewrites rule 11 to read: Mail, and every click, typing or choice in Niles's browser, runs only after an approval answered on its card in the app: never by voice, never from a notification's button, never "for this conversation", and no setting turns that off. *A misheard yes is worst here.*
- Step 2 adds to rule 13: Niles's browser reaches the internet only through Niles's proxy, which checks every connection the same way.
- Step 3 adds a group, **Saved logins**, before **Code**, holding a new rule 27, and the code rule becomes 28: A saved login is typed only by Niles, from its own script in the field's frame, which checks in the same step that the frame and the page are exactly one of the login's sites. Nothing sent to a model, the chat, the approval log or a log line holds a saved value, and no endpoint returns a password. *A page can't talk the model out of a password it never had.* AGENTS.md and the docs' architecture page then count 28 house rules.

**Risks and limits.**

- Amazon's Conditions of Use prohibit robots. Amazon sued Perplexity over its Comet shopping agent, and the Ninth Circuit (August 2026) reversed a district court's injunction (March 2026), with the case still going on. For a household on amazon.de the practical risk is the account being challenged or locked, not a lawsuit. Expect bot detection, CAPTCHAs and new-device codes. Niles doesn't disguise itself, so amazon.de may refuse it, and step 5 finds out.
- Page text can tell the model to do things. Every step waits on a card, Niles types logins only on their own sites, a signed-in session stays on its sites, and the model holds no secret to leak.
- Basket, checkout and order take about 4 to 8 cards. That is the price of approving everything, so a card reads at a glance: the step as its title, the picture, the site and the reason.
- Restarts lapse held steps and close sessions, which is the right end for a half-done purchase.
- The browser costs one Chromium container, with no page open between sessions and a 2 GiB memory limit.
- A shop with an API is better served by an integration, as nemlig.com is by `niles-nemlig`. Saved logins and the cards carry over to those.

**How it's built.** One pull request a step, each with its docs, its `features.toml` entries and its tests.

- **Step 2, the browser without logins.** The image, the Kustomize component and the Compose file, the proxy, `niles-browser` in the crate map, sessions, the page as text, held steps with their pictures, the page-changed check, and the turn Niles starts after an Allow, added to "How a chat turn flows". Tests: the proxy refuses private, loopback, link-local and cluster addresses, other ports and other peers, and connects to the address it checked; every click, typing and choice makes a card and does nothing before Allow; a changed address, or an element removed, relabelled or covered, lapses the step; nothing sent to the model is a picture. Integration tests drive a real Chromium (GitHub's Ubuntu runners have Chrome) against pages the test serves itself.
- **Step 3, saved logins.** The store, `/me/logins`, My profile → Logins, the Add login card and its form, `browser_sign_in`, the scrubbing, the order card, and glossary entries for a saved login and a browser session. Tests: a login is never typed on another origin, including after a redirect, in a frame from elsewhere, and when the page moves between the card and the Allow; no saved value reaches anything sent to the model, the chat log, the approval log or tracing captured at `TRACE`; the form posts to `/me/logins` and never to `/chat`, and the conversation holds no saved value afterwards; no endpoint returns a password; saving with an empty password keeps the old one; another person's logins can't be listed, edited, removed or signed out, in API tests and not only in the UI.
- **Step 4, codes and browser profiles.** The code card, and the sealed profiles with their 30 days. Tests: a code is set only on the login's sites and never logged; what is stored is sealed; Remove deletes the profile; Sign out and a new username or new sites forget it.
- **Step 5, amazon.de end to end.** By hand, from the phone: a search, then the basket, every step approved, stopping before the order. Then one real low-value order, supervised. The pull request, or a comment on NIL-10, says what happened, codes and CAPTCHAs included.

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
11. Mail is never sent without an approval answered on its card in the app: never by voice, never from a notification's button, never "for this conversation", and no setting turns that off. *A misheard yes is worst here.*
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
- Browsing on someone's behalf (NIL-10, not built yet): headless Chromium in a container of its own, driven over the Chrome DevTools Protocol by a small client in a new crate, `niles-browser`, and reaching the internet only through a proxy in Niles that checks every connection as `read_page` checks its fetches. Every click, typing and choice waits on its card, Niles types a saved login only on its own sites, and the model reads pages as text and never holds a saved value. **Rejected:** Playwright MCP as a tool server, whose `--secrets` only replaces matching text in its responses (its README calls it "a convenience and not a security feature") and whose origin lists, by the same README, are not a security boundary and don't apply to redirects, so nothing would tie a password to its site; Chromium in Niles's own container, where a page that took it over would find the secrets key; a sidecar in Niles's pod, which would keep DevTools off the network without a NetworkPolicy but share Niles's way into the cluster; `chromiumoxide`, a client for the whole protocol where Niles makes a few dozen calls, some of them carrying a password through code Niles doesn't own; a live view of the browser; and logins shared by the household.
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

**The browser (NIL-10, not built yet).** It runs pages strangers wrote, steered by a model that reads them, so it holds as little as possible. Chromium's own sandbox needs user namespaces, which containers usually don't allow, so Chromium runs without it and the container is the sandbox: no root, no capabilities, a read-only filesystem, and no secrets key or database. A cluster that gives pods their own user namespaces (`hostUsers: false`) could turn Chromium's sandbox back on later. A page that took over Chromium would hold the open sessions' cookies. That is why a session closes after ten idle minutes, a signed-in one stays on its login's sites, the image is rebuilt weekly, and the browser restarts once a day when idle. Its requests leave only through Niles's proxy. The NetworkPolicies keep a taken-over browser from going around the proxy and keep everything but Niles off its DevTools port, but only where the cluster's network plugin enforces them. Flannel, Talos's default, doesn't, and there any pod in the cluster can drive the browser, and a taken-over browser can reach the cluster. In Compose, Docker enforces the internal network. The model never holds a saved value, so a page that fools it can't make it leak one. What is left is the person. The Add login card shows the site the page is really on, in full, and someone who saves their password for a lookalike site has given it to that site.

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
