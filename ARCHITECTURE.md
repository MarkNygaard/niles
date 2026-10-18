# Niles architecture

This is the map and the rules. What exists is listed in `features.toml` (and the `MANIFEST.md` made from it); how a piece works and why is in its crate's doc comments; how to use Niles is the user guide in `docs/src/`; planned work and feature specs are Linear issues in the *Niles software* project. This file changes when a house rule, the crate map or a big decision changes.

## What Niles is

*Niles* (NYE-uls) is the project, the system and the wake word: one word, three roles. The acronym, **N**eural **I**ntelligence, **L**ightweight **E**dge **S**ystem, says what it is: an AI-first home assistant that runs on the household's own small hardware.

It replaces Home Assistant for a household that has a focused set of devices (mostly Zigbee through Zigbee2MQTT, a few Wi-Fi ones), wants voice that answers in well under a second, prefers config and naming to setup screens, and wants a language model built in rather than bolted on. Home Assistant solves a different problem, the widest device coverage for people who don't run infrastructure, and that makes it heavy and UI-led. Niles is opinionated and assumes somebody in the house runs a cluster.

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

A Cargo workspace of 36 crates; `niles-bin` is the only binary and wires the rest together.

**Core and state**

| Crate | What it is |
|---|---|
| `niles-core` | Device ids and types, the `DeviceRegistry`, the `EventBus`. No business logic. |
| `niles-config` | The TOML schema, its validation, and the layered `ConfigStore` (base plus overrides, hot or boot sections). |
| `niles-db` | Every Postgres store: config overrides and their history, sealed secrets, conversations, artifacts, the approvals log, timers, waits, voice prints and more. Each creates its own table on first use. |
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
| `niles-history` | The command and state history the model reads to answer "what did I just do". |

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
| `niles-api` | The HTTP API, sign-in, `/events/stream`, and the web app's routes. |
| `niles-webpush` | Web Push to a person's devices, RFC 8291 and 8292 in pure Rust. |
| `niles-websearch` | The SearXNG client, and the fetcher that goes only to the public internet. |
| `niles-mcp` | Niles as an MCP client, for remote tool servers such as Linear's. |
| `niles-plugins` | Third-party plugins: WebAssembly components run in wasmtime, installed from GitHub releases and found in the add-on library. |
| `niles-integration-linear` | Linear's webhook, turned into notifications. |
| `niles-tls` | TLS to devices that sign their own certificate (an LG TV). |
| `niles-bin` | The `niles` binary: `serve`, the dev subcommands, and all the wiring. |

**Outside `crates/`:** `web/` (the React app, served by `niles-api`), `firmware/xiao-wyoming/satellite-idf/` (the satellite firmware), `hardware/satellite-v2/` (the custom satellite board), `schemas/` (the plugin WIT), `docs/src/` (the user guide, an mdBook), `docs/capabilities/` (capability references), `docs/integrations/` (the plugin author guide), `deploy/kustomize/` and `deploy/README.md` (deployment), and `examples/niles.example.toml`.

## How a voice turn flows

1. The satellite's XMOS DSP cleans the audio; its ESP32-S3 hears the wake word on the device and streams raw 16 kHz PCM over Wyoming TCP to `niles-wyoming`.
2. At the wake, `voice::hush_on_wake` ducks the room's Sonos to a quarter of its volume (`speak::Hush`).
3. `voice::spawn_dispatch_task` sends the utterance to `niles-stt` (ElevenLabs Scribe by default) while `niles-recognition` identifies the voice on a blocking thread beside it.
4. `dispatch::dispatch_transcript` drops what wasn't speech (no words, a hallucinated script, the transcriber's own noise gate), what didn't start with Niles's name when that is required, and an unknown voice when only known voices are answered. All of this runs before Tier 0.
5. `dispatch::text::dispatch_text` tries Tier 0: `niles-intent` turns the words into an `Intent`, which is acted on directly (a light through the `CommandRouter`, a timer, a scene, music).
6. On a miss, `dispatch::dispatch_tier1` builds the system prompt (`prompt.rs`: persona, capability references by topic, memory, skills) and runs `chat_loop` against the fast model (`niles-llm`, Cerebras by default) with the tools `niles-tools` judges relevant.
7. A spoken rule (caught by its words, or by the fast model reaching for an automation tool), or a tool server's tool the household left to the assistant, sends the turn to the assistant instead (Claude Code, or ChatGPT through pi; `assistant.rs`), which reaches the same tools over Niles's own MCP server. `escalate_to_tier2` re-runs the turn on Tier 2 if one is configured.
8. A tool set to Ask first, or a mail, is held as an approval (`approvals.rs`). A remote tool can be answered by a spoken yes; a mail waits in the app.
9. `speak::speak_back` synthesises the reply with Piper (`niles-tts`) and streams it to the satellite. A reply that asks a question keeps the microphone open for the answer (`follow_up.rs`). The turn goes into the history, the journal and, for a recognised voice, that person's chat.

## How a chat turn flows

1. The app (`web/`) sends `POST /chat`, JSON or multipart with files, to `niles-api`. The session says who is typing; their address links them to their voice.
2. `app.rs` (`AppChat::reply`) finds the conversation in the chat log and seeds the context with it, spoken turns included.
3. Without files, `dispatch_text` runs as for voice: Tier 0 first, then `dispatch_tier1`, where a chat turn goes to the assistant when one is on, and to the fast model otherwise or when the assistant can't answer (with a note under the reply saying why).
4. With files, `dispatch::dispatch_attached` skips Tier 0 and the fast model: only the assistant can see them, and nobody else answers.
5. The assistant's tools come over MCP with a token for that run, which is how a tool knows whose turn it is. Artifact tools are offered only there; a big artifact is a background job.
6. The turn is kept with its approval card and artifacts. Later changes go to that person's connections on `/events/stream` (`approval_changed`, `artifact_changed`), and a finished job reaches their devices by Web Push.

## How an automation flows

1. A rule is made by voice (the model reads it back and saves it on a yes), on the Automations page, or as `[[automation]]` in the config. The `niles-automations` store keeps the ordered set as one document in Postgres; the later of two conflicting rules wins.
2. The engine (`engine.rs`) follows the `EventBus` and the clock. A state trigger fires on the change into the state, and with `for` only once it has held. Conditions are read against the registry. A rule runs once at a time; firing again replaces the waiting run.
3. Actions go out through the `DeviceSink` (`MqttDeviceSink` in `niles-bin/src/wiring/automations.rs`, which uses the `CommandRouter`) and the `Notifier` (`niles-notifications`).
4. A delay is kept in `automation_waits`, and a `wake_up` action starts a wake-up (`wake_up.rs`) kept in `wake_ups`, so both survive a restart. The last 20 runs of each rule, including ones a condition stopped, are kept in memory to answer "why didn't it".
5. The curve and the morning routine are code, not rules: `niles-scheduler` computes them and `niles-bin/src/lighting.rs` ticks them.

## How an add-on is found

1. The author publishes releases as `docs/integrations/publishing.md` says, with a small `niles-installed` file beside the manifest and component, and gives the repo the GitHub topic `niles-addon`.
2. The registry repo, `MarkNygaard/niles-addons`, lists reviewed repos in `addons.toml`. A pull request that adds one runs `niles plugin check --listing <repo>`: everything an install checks, plus the marker. It puts the permission lines in the check's summary, and Mark merges by hand. Merged is **Listed**: it passed the checks and someone read what it asks for. That is not an audit of its code, and later releases aren't reviewed again.
3. An Action in the registry builds `index.json` every six hours and on every merge, and GitHub Pages serves it. It holds the listed repos, every public repo with the topic as Unreviewed, and repos taken off the list (removed, with the reason). Each entry carries its latest release's manifest and logo as text, a README excerpt, its stars, and GitHub's download counts. The format is in `niles_plugins::library`.
4. `GET /plugins/install/library` fetches that one file when Browse or an add-on's panel asks, and keeps it six hours. It reads it as a stranger's: manifests go through the install's own parser, so permissions read as on the approval screen and `min_niles` is the install's own check; logos go through the logo allowlist; text is capped, and links are built from the repo. Browsing calls no GitHub API.
5. Browse's **Install** opens the paste flow with the address filled in: the same preview, checks and approval. An installed add-on's panel says Listed, Unreviewed or No longer listed.
6. After a fresh install succeeds, `Installer::install` downloads the release's `niles-installed` once per repo per house; `plugin_installs_counted` remembers, through a removal. A preview, an update or a reinstall never downloads it. GitHub's download count of that file across releases is the install count; without one, the library shows the component's downloads, labelled as such. This is not telemetry (rule 26): a public file is fetched from the address that just downloaded the add-on, and it carries nothing that download didn't.

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
8. A config section is hot only if something re-reads it; writes are validated whole and persisted before the swap; the voice tool refuses boot sections. *A stored change nobody acts on is worse than a no.*
9. What must outlive a restart is in the database (timers, waits, wake-ups, morning claims). What is deliberately in memory says so when it is lost: held approvals lapse, and a job a restart cut short is marked failed. *Every deploy is a restart.*

**The model and untrusted content**

10. The model never decides an approval. Niles holds the exact call and runs what it held after the person's answer; a change is a new approval. *What runs is what was shown.*
11. Mail is never sent without an approval answered in the app: never by voice, never "for this conversation", and no setting turns that off. *A misheard yes is worst here.*
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
26. No telemetry. If it is ever added, opt-in only. A download a person's own action starts, from the host that action already reaches, and carrying nothing that action didn't, is not telemetry: the add-on library's install count (one `niles-installed` per repo per house, at install) is the one such download. *Local-first is the point.*

**Code**

27. No traits without implementations; `#[non_exhaustive]` on public enums that grow; newtypes for validated identifiers; no compatibility shims before production; no half-finished implementations or `todo!()` paths. *See AGENTS.md.*

## Big decisions

- Cluster Postgres, not embedded SQLite (#16); stores create their own tables, and the database is just `NILES_DATABASE_URL` (#367).
- Layered config: a ConfigMap base plus overrides in Postgres, with history and undo, edited from the UI, by voice or over the API (#153, #154, #158; spec #168). Why Postgres: see Operations notes.
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
- Accounts: one sign-in per provider serves Calendar, Mail or both (#424); mail (#425); iCloud over IMAP and SMTP (#426). App registrations: see Operations notes.
- Training from each person's own Strava app, not Garmin or Apple Health (#410, then #418). Why: see Operations notes.
- Artifacts made by the assistant and versioned per person (#429); pages in a sealed frame (#440); pictures as Niles's own copies (#442); sharing to view or to edit (#452).
- Web Push, with no inbound port (#432).
- SearXNG as the house's search; Claude's own `WebSearch` when there is none; `WebFetch` never (#451).
- Chat attachments, sniffed by content and always answered by the assistant (#449).
- Blinds open to a normal level, and a second Open goes all the way (#441).
- Podcasts: the assistant researches and writes the script; the house's own Kokoro voices it, and the episode lives in a private R2 bucket the household set up, not on a hosted voice or storage service (#457).
- Release flow: `development` is the default branch, release-please cuts releases, images are built from `main` (#456).

## Security notes

**Artifact pages.** A page may be written by a model that has just read web pages, so it is trusted with nothing. It runs in a `sandbox="allow-scripts"` srcdoc frame with an opaque origin, under a CSP that allows inline script and style, `data:` and `blob:` images and media, and nothing else (`connect-src 'none'`, `webrtc 'block'`). Its only way out is a message the app reads as a stranger's. The app's own policy is only `frame-src 'none'`, which stops the frame navigating itself to an address that carries what it knows.

- **Tested in Chrome 155** against a page that tried every way out (fetch, XHR, sockets, beacons, remote images, CSS and fonts, every link hint, child frames, workers, WebRTC, popups, the top window, forms, `<a ping>`, cookies and storage, navigating itself): nothing reached a server, and only self-navigation did once the app's `frame-src` was removed.
- **What is left, and why it is small.** A script can still add its own `dns-prefetch` or `preconnect` link, and not every browser holds those hints to a policy, so a DNS lookup could carry a few bytes out. A determined script might find the WebRTC constructors again in a child frame. Either could carry only what the page has: this artifact's data, for the person looking at it. No session, no other artifact, nothing of the house.
- The details are in `web/src/lib/sandboxedPage.ts` and `crates/niles-artifacts/src/page.rs`.

**If Niles is ever exposed to the internet.** Only port 8080, the HTTP API, goes out; Wyoming and MQTT stay on the house network, and the satellite streams audio unauthenticated. **`/logs` changes character:** at home it shows voice transcripts to people already in the house; exposed, it shows them to anyone holding a session. That is a reason to keep the allowlist short, not to hide the route. Exposing a plugin's webhook (`/plugins/*/hook/*`) is not exposing Niles; the app can stay at home. Pushes to a phone need no inbound port at all.

**The allowlist amends itself, by design.** There are no roles, so anyone signed in can let anyone else in. That fits two adults sharing a home and is the wrong shape for anything larger: **revisit it first if Niles ever has a user who is not a householder.** If the list is ever emptied some other way, the bearer token is not subject to it, and `PATCH /config` restores it. People listed with no GitHub app is sign-in off, not failed closed (that would leave the bearer token as the only way in); the People card and the startup log say so.

**The plugin trust model.** The sandbox limits what a plugin can *reach*; it does not make a plugin *trustworthy*. An approved plugin can do anything its declared hosts and devices allow: one given an account's password can do what that account can, one subscribed to device state learns when the house is empty and can send that to any host it declared, and any plugin can tell the model untrue things in a tool result. What the sandbox guarantees: no undeclared hosts or local addresses, no other integration's secrets, no database, conversations, memory or people, no code in the app, and capped memory and time. So the approval screen says plainly what is granted, the repo is shown, the component is pinned, and an update that asks for more is approved again.

**The add-on library.** Listed means an add-on passed `niles plugin check --listing` when it was listed and someone read its permissions. It is not an audit of its code, and later releases aren't reviewed again. Nothing about installing changes: Browse's Install is the same preview and approval as a pasted address, and an update that asks for more is approved again. The index is a stranger's file. Niles parses its manifests with the install's parser, sanitises its logos with the install's allowlist, shows its text as text, and takes no address from it.

## Operations notes

**Config overrides live in Postgres**, not on a node-local volume. A local volume pins the pod to one node and blocks drains, which on a Talos cluster means every upgrade. Voice prints are in Postgres for the same reason. Without a database, changes still apply but don't outlive the process, and every surface says so.

**Why Strava.** Garmin has no API for an individual (its developer programme is for businesses), and the unofficial login that libraries such as garminconnect used broke behind Cloudflare in March 2026. Apple Health has no web API; the Health Auto Export app that first fed Niles charges for its REST automation and runs only while the phone is unlocked. Strava has a web API, and Garmin Connect uploads every workout to it. A new Strava app serves only the athlete who made it, so **each person makes their own app** and enters its Client ID and Secret on their profile. Strava gives activities only: no sleep, HRV or weight.

**The household's app registrations** for accounts (Calendar and Mail):

- *Google:* the OAuth app must be **published**, not left in testing, where Google expires sign-ins after seven days. Unverified, it shows "Google hasn't verified this app", which is expected. Reading Gmail is a *restricted* scope, which an unverified app may hold for at most 100 people.
- *Microsoft:* register in Entra for "accounts in any organizational directory and personal Microsoft accounts". A work tenant may still refuse a person's consent to an outside app; that is the employer's call.
- *Both:* changing what an account serves means signing in again for the new scopes. iCloud's app-specific password can't be narrowed, so its read-only access is Niles's own rule, not Apple's.

**Deploying** is `deploy/README.md` and the Kustomize manifests in `deploy/kustomize/`.

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
- **Hot / boot section**: a config section re-read while running, or one read only at startup.
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
- **How to use Niles:** the user guide in `docs/src/`.
- **Writing a plugin:** `docs/integrations/`.
- **The long spec this replaced:** git history.
