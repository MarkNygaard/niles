<!-- DO NOT EDIT BY HAND. Generated from features.toml — run `cargo run -p niles-bin -- generate-manifest`. -->

# niles — feature manifest

Niles is an AI-first home-automation system for private homes. It listens to natural-language voice commands, dispatches them through a tiered intent router, and can escalate to an LLM when a request exceeds the deterministic tier. The feature catalog below tracks every user-observable capability shipped so far. For the crate map, the house rules and the big decisions see ARCHITECTURE.md.

## ambient

| Feature | Summary | Since |
| --- | --- | --- |
| `curve-brightness` | Per-minute adaptive brightness curve based on time of day | [#5](https://github.com/MarkNygaard/niles/pull/5) |
| `curve-color-temp` | Per-minute adaptive color-temperature curve based on time of day | [#7](https://github.com/MarkNygaard/niles/pull/7) |
| `curve-morning-routine` | Gradual morning-routine ramp that respects per-room claims | [#24](https://github.com/MarkNygaard/niles/pull/24) |
| `morning-routine-keeps-manual` | At the end of the morning routine, a light set by hand during the ramp keeps its level instead of going to full | [#466](https://github.com/MarkNygaard/niles/pull/466) |
| `manual-mode-from-tools-and-rules` | A level, warmth, colour or effect set by the model or an automation puts the light in manual mode; on or off does not | [#466](https://github.com/MarkNygaard/niles/pull/466) |
| `manual-mode-override` | Clear manual-mode overrides after a configurable timeout | [#32](https://github.com/MarkNygaard/niles/pull/32) |

## api

| Feature | Summary | Since |
| --- | --- | --- |
| `api-events-approval-changed` | approval_changed on /events/stream when one is held, answered or lapses, to its person only | [#455](https://github.com/MarkNygaard/niles/pull/455) |
| `api-events-stream` | WebSocket /events/stream pushes real-time device and system events | [#86](https://github.com/MarkNygaard/niles/pull/86) |
| `events-artifact-status` | artifact_changed on /events/stream, to its owner and whoever it is shared with only: the status line follows a job | [#429](https://github.com/MarkNygaard/niles/pull/429) |
| `api-artifact-images` | GET /me/artifacts/{id}/images/{image}: a picture an artifact keeps, its owner's only; GET lists them | [#442](https://github.com/MarkNygaard/niles/pull/442) |
| `api-get-devices` | GET /devices returns the full device registry snapshot | [#13](https://github.com/MarkNygaard/niles/pull/13) |
| `api-get-room` | GET /rooms/{room} returns the devices in a single room | [#13](https://github.com/MarkNygaard/niles/pull/13) |
| `api-healthz` | GET /healthz returns a liveness probe for load balancers | [#13](https://github.com/MarkNygaard/niles/pull/13) |
| `api-plugin-log-image` | GET /plugins/{id}/log for a plugin's last lines, and /plugins/{id}/image for its pages' pictures, fetched by Niles | [#391](https://github.com/MarkNygaard/niles/pull/391) |
| `api-plugins` | GET /plugins lists installed plugins with their status and last error | [#388](https://github.com/MarkNygaard/niles/pull/388) |
| `api-plugins-library` | GET /plugins/install/library: the add-on library from the registry's one index.json, kept 6 hours; no GitHub API calls | [#469](https://github.com/MarkNygaard/niles/pull/469) |
| `api-training-today` | GET /me/training/today: the signed-in person's training plan read for today, or plan null without one | [#438](https://github.com/MarkNygaard/niles/pull/438) |
| `api-artifact-pages` | An artifact's page_html and the house's today on GET /me/artifacts/{id}; PATCH remove_page; each version with its page | [#440](https://github.com/MarkNygaard/niles/pull/440) |
| `api-artifact-shares` | GET/PUT/DELETE /me/artifacts/{id}/shares, its owner's only; every artifact route answers by owner, edit or view | [#452](https://github.com/MarkNygaard/niles/pull/452) |
| `api-chat-attachments` | POST /chat takes a multipart body with up to 5 files of 10 MB; GET …/attachments/{a} serves one to its owner | [#449](https://github.com/MarkNygaard/niles/pull/449) |
| `api-me-approvals` | GET /me/approvals, POST /me/approvals/{id}, GET /me/approvals/history?before=; the caller's own only | [#455](https://github.com/MarkNygaard/niles/pull/455) |
| `api-plugins-manage` | POST /plugins/install/preview and /plugins/install; update, rollback, enable, disable and DELETE per plugin | [#390](https://github.com/MarkNygaard/niles/pull/390) |
| `api-post-device` | POST /rooms/{room}/{device} sets one light, in whichever source it comes from | [#59](https://github.com/MarkNygaard/niles/pull/59) |
| `api-post-lights` | POST /lights sets every light in the house in one request | [#189](https://github.com/MarkNygaard/niles/pull/189) |
| `api-post-room` | POST /rooms/{room} sets every light in a room in one request | [#178](https://github.com/MarkNygaard/niles/pull/178) |
| `api-push` | GET /push/key; GET, POST and DELETE /me/push for this person's devices; POST /me/push/test | [#432](https://github.com/MarkNygaard/niles/pull/432) |
| `api-web-search-test` | POST /integrations/web_search/test runs a web and an image query through the saved SearXNG and counts the results | [#451](https://github.com/MarkNygaard/niles/pull/451) |

## automations

| Feature | Summary | Since |
| --- | --- | --- |
| `automations-any-device-action` | Any device that reports an action, such as a printer finishing, can start a rule, and the editor offers it | [#393](https://github.com/MarkNygaard/niles/pull/393) |
| `automations-motion-groups` | A motion trigger or condition over several sensors or rooms: no motion in any of them for 2 minutes, or motion in one | [#450](https://github.com/MarkNygaard/niles/pull/450) |
| `automations-rule-in-full` | A rule said a line at a time from the rule itself, shown with Save in the chat and used for the confirmation | [#450](https://github.com/MarkNygaard/niles/pull/450) |
| `automations-save-warnings` | Saving or proposing a rule warns when a check after a wait could end it with nothing to try again | [#450](https://github.com/MarkNygaard/niles/pull/450) |

### `automations-motion-groups` — example phrasings

- keep the hallway and kitchen lights on while I move between them
- turn both off once neither room has had motion for two minutes

## llm

| Feature | Summary | Since |
| --- | --- | --- |
| `tool-command-history` | Query the command-history log for past user requests | [#70](https://github.com/MarkNygaard/niles/pull/70) |
| `tool-datetime` | Return the current date and time in the configured timezone | [#84](https://github.com/MarkNygaard/niles/pull/84) |
| `tool-device-history` | Query the device-state history log for past state changes | [#71](https://github.com/MarkNygaard/niles/pull/71) |
| `tool-explain-device-state` | Provide a natural-language explanation of a device's current state | [#70](https://github.com/MarkNygaard/niles/pull/70) |
| `tool-light-control` | Turn lights on or off and set brightness or color temperature | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `tool-look-up-capability` | Retrieve a capability reference document by name for the LLM context | [#84](https://github.com/MarkNygaard/niles/pull/84) |
| `tool-scene-control` | Apply or delete saved scenes via structured tool calls | [#43](https://github.com/MarkNygaard/niles/pull/43) |
| `tool-sonos-transport` | Control Sonos speaker transport state via the speakers subsystem | [#65](https://github.com/MarkNygaard/niles/pull/65) |
| `tool-timer` | Set, cancel, and list timers via structured tool calls | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `tool-tv-volume` | tv_volume sets, steps, mutes or unmutes the TV's own speakers; music_volume turns the TV in a room with no Sonos | [#403](https://github.com/MarkNygaard/niles/pull/403) |
| `tool-weather` | Fetch current weather and forecast via Open-Meteo | [#81](https://github.com/MarkNygaard/niles/pull/81) |
| `tool-web-search` | Search the web via SearXNG and return summarized results; offered while the SearXNG add-on is set up | [#83](https://github.com/MarkNygaard/niles/pull/83) |

## runtime

| Feature | Summary | Since |
| --- | --- | --- |
| `approval-log` | Every approval logged when asked and answered, lapses included; one summary line, never the arguments | [#455](https://github.com/MarkNygaard/niles/pull/455) |
| `artifacts-documents` | Claude or ChatGPT saves documents for you, with versions, and writes a big one in the background | [#429](https://github.com/MarkNygaard/niles/pull/429) |
| `artifacts-edits` | Small changes to a saved item or its page made in the chat as edits; big or slow ones finish in the background | [#474](https://github.com/MarkNygaard/niles/pull/474) |
| `artifacts-pages` | Ask Claude or ChatGPT for a page for any saved artifact: HTML that draws its data, and draws again as the data changes | [#440](https://github.com/MarkNygaard/niles/pull/440) |
| `artifacts-pictures` | Pictures in a document, a trip or a page, found with the house's search and kept as Niles's own clean copies | [#442](https://github.com/MarkNygaard/niles/pull/442) |
| `artifacts-sharing` | Share a saved artifact with someone in the house, to read or to edit; an editor's own Claude or ChatGPT can change it | [#452](https://github.com/MarkNygaard/niles/pull/452) |
| `artifacts-trip` | A trip planned in the background: days, places, stays and bookings, a countdown, and a packing list to tick | [#434](https://github.com/MarkNygaard/niles/pull/434) |
| `assistant-built-in-search` | Without SearXNG, Claude searches with Claude Code's own WebSearch (never WebFetch); ChatGPT is told it has no search | [#451](https://github.com/MarkNygaard/niles/pull/451) |
| `assistant-none` | Choose Niles's own model as the assistant: neither Claude nor ChatGPT answers, and nothing runs in the background | [#394](https://github.com/MarkNygaard/niles/pull/394) |
| `assistant-reads-attachments` | Claude or ChatGPT answers about an attached photo or PDF; a message with files always goes to the assistant | [#449](https://github.com/MarkNygaard/niles/pull/449) |
| `web-search-live` | Adding, changing or removing the SearXNG address takes hold with the next search, without a restart | [#451](https://github.com/MarkNygaard/niles/pull/451) |
| `notification-history` | What Niles says unprompted is kept in Postgres, so "what did I miss" still knows after a restart | [#466](https://github.com/MarkNygaard/niles/pull/466) |
| `notifications-for-the-house` | An announcement with no room is spoken in every room with a satellite, still under quiet hours | [#466](https://github.com/MarkNygaard/niles/pull/466) |
| `push-artifact-ready` | A notification on your phone or computer when a document written in the background is ready; a tap opens it | [#432](https://github.com/MarkNygaard/niles/pull/432) |
| `push-artifact-shared` | A notification when someone shares an artifact with you, from the app or by asking: Mark shared “Thailand trip” with you | [#452](https://github.com/MarkNygaard/niles/pull/452) |
| `observability-command-log` | Structured command-history log for every voice and API request | [#70](https://github.com/MarkNygaard/niles/pull/70) |
| `observability-state-log` | Structured device-state history log for every state change | [#71](https://github.com/MarkNygaard/niles/pull/71) |
| `persistence-json-store` | JSON file persistence for timers, scenes, and morning-routine claims | [#64](https://github.com/MarkNygaard/niles/pull/64) |
| `plugins-check-listing` | `niles plugin check` lists a plugin's permissions; `--listing` also checks the marker the add-on library needs | [#469](https://github.com/MarkNygaard/niles/pull/469) |
| `plugins-dev-check` | `niles plugin check` validates a plugin as an install would; `niles plugin dev` runs one against a stand-in house | [#388](https://github.com/MarkNygaard/niles/pull/388) |
| `plugins-dev-loopback-wait` | `niles plugin dev` waits for MQTT before --call, runs the newer build, and reaches a fake device with --allow-loopback | [#393](https://github.com/MarkNygaard/niles/pull/393) |
| `plugins-disable-remove` | Turn a plugin off and keep its data, or remove it with its devices, storage, settings and secrets | [#390](https://github.com/MarkNygaard/niles/pull/390) |
| `plugins-install` | Install a plugin from a GitHub release after a preview of its permissions; exactly the previewed files run | [#390](https://github.com/MarkNygaard/niles/pull/390) |
| `plugins-install-count` | A fresh install downloads the release's niles-installed once per house, so GitHub counts installs; previews don't | [#469](https://github.com/MarkNygaard/niles/pull/469) |
| `plugins-interface-versions` | Plugins built for interface 0.1 keep running beside 0.2 ones, marked as built for an older Niles | [#392](https://github.com/MarkNygaard/niles/pull/392) |
| `plugins-mqtt` | Plugins on interface 0.2 talk MQTT to the brokers their manifest names; Niles holds the connection and reconnects | [#392](https://github.com/MarkNygaard/niles/pull/392) |
| `plugins-mqtt-trust-device` | A device's own MQTT certificate is trusted on first use and pinned; a changed one waits for the household to accept it | [#392](https://github.com/MarkNygaard/niles/pull/392) |
| `plugins-plain-errors` | Plugins' MQTT and HTTP failures read as plain reasons, such as connection refused or wrong username or password | [#393](https://github.com/MarkNygaard/niles/pull/393) |
| `plugins-room-light` | A plugin can keep a light out of its room's lights, so room commands, scenes, presence and the curve leave it alone | [#393](https://github.com/MarkNygaard/niles/pull/393) |
| `plugins-runtime` | Run third-party integrations as sandboxed WebAssembly components; their devices, tools and pages join the house | [#388](https://github.com/MarkNygaard/niles/pull/388) |
| `plugins-speech-stream` | Permission-gated room speech sessions with explicit enablement, bounded work, expiry and asynchronous outcomes | [#482](https://github.com/MarkNygaard/niles/pull/482) |
| `plugins-time-zone` | Plugins on interface 0.2.1 are told the house's time zone; 0.2.0 plugins run unchanged | [#393](https://github.com/MarkNygaard/niles/pull/393) |
| `plugins-update-rollback` | Plugins are checked for newer releases; an update shows the permission change and rolls back if it soon fails | [#390](https://github.com/MarkNygaard/niles/pull/390) |
| `plugins-websocket` | Add-ons connect to permission-scoped WebSocket endpoints through the host, with bounded queues and reconnect callbacks | [#470](https://github.com/MarkNygaard/niles/pull/470) |
| `providers-default-elevenlabs-cerebras` | A new install hears with ElevenLabs and thinks with Cerebras; stored Groq settings fall back to them and say so | [#394](https://github.com/MarkNygaard/niles/pull/394) |
| `artifacts-training-plan` | A training plan toward a race, made from your own Strava runs: weeks, a countdown and today's session under Saved | [#430](https://github.com/MarkNygaard/niles/pull/430) |
| `training-plan-follows-runs` | A training plan follows your Strava runs: each marks its session done, and Claude or ChatGPT adjusts what is to come | [#436](https://github.com/MarkNygaard/niles/pull/436) |
| `training-strava-sync` | Each person's own Strava: 180 days on connect, then a poll every 15 minutes within Strava's rate limits | [#418](https://github.com/MarkNygaard/niles/pull/418) |

### `artifacts-documents` — example phrasings

- write a packing list for the cabin
- make a travel plan for our Thailand trip
- add sunscreen to my packing list

### `artifacts-edits` — example phrasings

- make the headings on the Thailand page green
- move the ferry to Koh Tao to 9:30
- make our trip a week longer

### `artifacts-pages` — example phrasings

- make my trip plan look nicer
- make a page for my training plan
- make the packing list into a poster

### `artifacts-pictures` — example phrasings

- add some pictures to my trip
- put a photo of the beach on day three
- make a page for our trip with pictures of each place

### `artifacts-sharing` — example phrasings

- share the Thailand trip with Majse so she can edit it
- let Majse see my training plan
- add Chiang Mai to our Thailand trip
- stop sharing the packing list with Majse

### `artifacts-trip` — example phrasings

- plan our trip to Thailand from 3 to 23 January
- make an itinerary for two weeks in Japan
- add a day in Chiang Mai to our Thailand trip

### `assistant-built-in-search` — example phrasings

- what's the news today
- when does the ferry to Samsø leave

### `assistant-reads-attachments` — example phrasings

- what's this plant
- add what's on this receipt to the shopping list

### `artifacts-training-plan` — example phrasings

- make me a half marathon training plan
- plan my training for the Copenhagen Half on 13 December
- move my long run to Saturday

## ui

| Feature | Summary | Since |
| --- | --- | --- |
| `auth-api-key-made` | Niles makes its own API key on a start with a database, and a new one works at once with no restart | [#405](https://github.com/MarkNygaard/niles/pull/405) |
| `auth-api-token` | A bearer token for reading the API from a terminal | [#186](https://github.com/MarkNygaard/niles/pull/186) |
| `auth-github-sign-in` | Sign in with GitHub, against an allowlist of verified email addresses | [#186](https://github.com/MarkNygaard/niles/pull/186) |
| `chat-attachments` | Attach photos and PDFs to a chat message by dragging, the paperclip or a paste; chips before sending, thumbnails after | [#449](https://github.com/MarkNygaard/niles/pull/449) |
| `ui-approvals-history` | Approvals history in the panel and on My profile: each decided or lapsed, "Allowed once · 15:34", Show older | [#455](https://github.com/MarkNygaard/niles/pull/455) |
| `ui-approvals-panel` | Approvals tab in Niles's panel: what needs review, folded with Details, answered there; a count on the tab | [#455](https://github.com/MarkNygaard/niles/pull/455) |
| `ui-artifact-page-frame` | An artifact's page in a sealed frame, no network and no Niles: a Page / Built-in switch, links opened on a yes | [#440](https://github.com/MarkNygaard/niles/pull/440) |
| `ui-artifact-pictures` | Pictures drawn from Niles's own copies with their credit; a picture by its web address is never fetched | [#442](https://github.com/MarkNygaard/niles/pull/442) |
| `ui-artifact-share-menu` | A share menu on an open artifact: Share, Copy, Word for documents, plans and trips, Markdown, a podcast's MP3 | [#472](https://github.com/MarkNygaard/niles/pull/472) |
| `ui-artifact-sharing` | Share from an open artifact: None, View or Edit for each person; Saved marks what is shared and who changed it last | [#452](https://github.com/MarkNygaard/niles/pull/452) |
| `ui-push-nudge` | While a document is being written, one tap on "Notify me when it's done" turns notifications on for this device | [#432](https://github.com/MarkNygaard/niles/pull/432) |
| `ui-saved-artifacts` | Saved tab: pin, archive, rename, tick checklists, restore versions; a card with a preview under the reply | [#429](https://github.com/MarkNygaard/niles/pull/429) |
| `ui-training-plan-sessions` | Tap a training plan's session to mark it done or skipped, and read the coach's notes on the plan | [#436](https://github.com/MarkNygaard/niles/pull/436) |
| `ui-todays-run-card` | Today's run on the dashboard: the session, the next one, the week's km and the coach's note, with Done and Skipped | [#438](https://github.com/MarkNygaard/niles/pull/438) |
| `linear-webhook` | Linear's card takes a webhook signing secret; once saved, an issue going to review or done is announced | [#466](https://github.com/MarkNygaard/niles/pull/466) |
| `plugins-library` | Browse lists add-ons others published, Listed before Unreviewed, with installs and stars; Install opens the same preview | [#469](https://github.com/MarkNygaard/niles/pull/469) |
| `plugins-widget-room-setting` | A plugin's widget can start in the room one of its settings names, and moves when the setting changes | [#393](https://github.com/MarkNygaard/niles/pull/393) |
| `ui-media-without-sonos` | Media without Sonos: the TV's switch, what is on and its own volume; no Sonos-only cards; with no TV, where to add one | [#408](https://github.com/MarkNygaard/niles/pull/408) |
| `ui-plugin-pages` | Plugins' pages join the menu and their widgets the front page and rooms, drawn from their declared trees | [#391](https://github.com/MarkNygaard/niles/pull/391) |
| `ui-plugin-setup` | A plugin's panel: a setup form generated from its manifest's settings, its log, and where its widgets go | [#391](https://github.com/MarkNygaard/niles/pull/391) |
| `ui-plugins` | Add-ons page: add-on from GitHub, and a panel per plugin to turn it off, update, roll back or remove it | [#390](https://github.com/MarkNygaard/niles/pull/390) |
| `ui-tv-volume` | The TV row has volume down and up, a slider and mute while it is on, where the TV is the speaker; POST /tv/volume | [#403](https://github.com/MarkNygaard/niles/pull/403) |
| `ui-tv-wake-mac` | TV panel: the wake MAC, detected or set by hand and kept through pairing; power-on says when the TV did not answer | [#409](https://github.com/MarkNygaard/niles/pull/409) |
| `ui-web-search-addon` | Web search (SearXNG) as an add-on: its address saved in the app, its status, a Test of web and image search, Remove | [#451](https://github.com/MarkNygaard/niles/pull/451) |
| `ui-installable` | Install the dashboard to a phone's Home Screen as an app | [#179](https://github.com/MarkNygaard/niles/pull/179) |
| `ui-room-card-blinds` | A room card with lights and blinds has separate Lights and Blinds buttons beside Heating, each opening its own sheet | [#404](https://github.com/MarkNygaard/niles/pull/404) |
| `ui-room-dashboard` | Web dashboard of every room, switching a whole room or one light | [#178](https://github.com/MarkNygaard/niles/pull/178) |
| `ui-room-hidden` | Hide a room from the dashboard's grid in Settings → Rooms; voice, rules, the API and the house's counts still reach it | [#404](https://github.com/MarkNygaard/niles/pull/404) |
| `ui-api-access` | Settings → API access: the key masked, with reveal, copy, a curl example and New key, for a signed-in person only | [#405](https://github.com/MarkNygaard/niles/pull/405) |
| `ui-profile-sections` | My profile laid out like Settings: About you, My phone, Notifications, each add-on a page, Approvals | [#455](https://github.com/MarkNygaard/niles/pull/455) |
| `ui-push-notifications` | My profile → Notifications: on for this device, your devices, Send a test; iPhone asks for the Home Screen app | [#432](https://github.com/MarkNygaard/niles/pull/432) |
| `ui-settings-system` | Settings splits the system (Devices: the broker; Voice & AI: hearing, thinking, speaking) from optional add-ons | [#394](https://github.com/MarkNygaard/niles/pull/394) |
| `ui-setup-checklist` | On a new install each System page leads with what it still needs, and the settings list counts it | [#394](https://github.com/MarkNygaard/niles/pull/394) |
| `ui-strava-connect` | My profile → Strava: set up your own Strava app, save its keys, connect, last sync, Sync now, disconnect | [#418](https://github.com/MarkNygaard/niles/pull/418) |
| `ui-voice-roles` | Picking a hearing or thinking provider sets it up: its API key sits beside it, and one save declares it | [#394](https://github.com/MarkNygaard/niles/pull/394) |

## voice

| Feature | Summary | Since |
| --- | --- | --- |
| `tier-two-escalation` | Escalate a voice request to the Tier 2 LLM when Tier 0/1 cannot answer | [#87](https://github.com/MarkNygaard/niles/pull/87) |
| `light-all-brightness` | Set brightness for every light in a room simultaneously | [#25](https://github.com/MarkNygaard/niles/pull/25) |
| `light-all-on-off` | Turn all lights in a room on or off at once | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `light-all-temp` | Set color temperature for every light in a room simultaneously | [#72](https://github.com/MarkNygaard/niles/pull/72) |
| `light-brightness` | Set a light's brightness to a specific percent | [#25](https://github.com/MarkNygaard/niles/pull/25) |
| `light-named-white` | Set a light to a named white such as warm or cool | [#74](https://github.com/MarkNygaard/niles/pull/74) |
| `light-on-off` | Turn a single light on or off by room and name | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `light-step` | Step a light brighter or dimmer by one increment | [#27](https://github.com/MarkNygaard/niles/pull/27) |
| `light-temp-step` | Step a light warmer or cooler by one color-temperature increment | [#72](https://github.com/MarkNygaard/niles/pull/72) |
| `media-duck` | Temporarily lower volume for voice responses | [#73](https://github.com/MarkNygaard/niles/pull/73) |
| `media-next-previous` | Skip to the next or previous track | [#75](https://github.com/MarkNygaard/niles/pull/75) |
| `media-play-pause` | Play or pause media playback on the active speaker | [#65](https://github.com/MarkNygaard/niles/pull/65) |
| `media-volume-set` | Set the speaker volume to a specific level | [#65](https://github.com/MarkNygaard/niles/pull/65) |
| `media-volume-step` | Raise or lower the speaker volume by one step | [#65](https://github.com/MarkNygaard/niles/pull/65) |
| `podcasts` | A researched two-host episode, voiced by the house's own Kokoro, played in the app or on a Sonos | [#457](https://github.com/MarkNygaard/niles/pull/457) |
| `tv-volume` | The LG TV's own speakers by voice, and a room's volume on its TV when the room has no Sonos | [#403](https://github.com/MarkNygaard/niles/pull/403) |
| `tv-wake-connected-interface` | Turning on the LG TV wakes the interface it is connected by, and every other MAC it reported too | [#409](https://github.com/MarkNygaard/niles/pull/409) |
| `memory-add` | Append a note to the user's persistent memory | [#77](https://github.com/MarkNygaard/niles/pull/77) |
| `memory-remove` | Remove a note from the user's persistent memory | [#77](https://github.com/MarkNygaard/niles/pull/77) |
| `memory-replace` | Replace an existing memory entry | [#77](https://github.com/MarkNygaard/niles/pull/77) |
| `memory-view` | Show the contents of the user's persistent memory | [#77](https://github.com/MarkNygaard/niles/pull/77) |
| `scene-apply` | Restore a previously saved scene by name | [#43](https://github.com/MarkNygaard/niles/pull/43) |
| `scene-delete` | Delete a saved scene by name | [#45](https://github.com/MarkNygaard/niles/pull/45) |
| `scene-list` | List all saved scenes | [#45](https://github.com/MarkNygaard/niles/pull/45) |
| `scene-room-by-name` | "cosy in the living room" saves or applies the living room's cosy when the house has that room | [#466](https://github.com/MarkNygaard/niles/pull/466) |
| `scene-save` | Save the current device states as a named scene | [#43](https://github.com/MarkNygaard/niles/pull/43) |
| `skill-background-review` | Review skill suggestions generated in the background | [#85](https://github.com/MarkNygaard/niles/pull/85) |
| `skill-curator` | Automatic background transitions for stale and archived skills | [#80](https://github.com/MarkNygaard/niles/pull/80) |
| `skill-delete` | Delete a saved skill by name | [#78](https://github.com/MarkNygaard/niles/pull/78) |
| `skill-mint` | Create a new skill from a natural-language description | [#78](https://github.com/MarkNygaard/niles/pull/78) |
| `skill-patch` | Update an existing skill's body while keeping its name | [#78](https://github.com/MarkNygaard/niles/pull/78) |
| `skill-view` | Read the full body of a saved skill | [#78](https://github.com/MarkNygaard/niles/pull/78) |
| `timer-cancel` | Cancel an active timer by name or duration | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `timer-list` | List all currently active timers | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `timer-set` | Set a timer for a given duration | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `voice-training-questions` | Ask about your own runs and workouts from your Strava by voice or chat, and have a week or month added up | [#410](https://github.com/MarkNygaard/niles/pull/410) |
| `voice-training-today` | Ask what your run is today or tomorrow, from your own training plan, with the week's distance and the countdown | [#436](https://github.com/MarkNygaard/niles/pull/436) |

### `tier-two-escalation` — example phrasings

- ask the smart assistant
- escalate this to tier two

### `light-all-brightness` — example phrasings

- set all lights to 75 percent

### `light-all-on-off` — example phrasings

- turn on all the lights
- switch everything off

### `light-all-temp` — example phrasings

- make all the lights warmer

### `light-brightness` — example phrasings

- set the living room light to 50 percent
- dim the hallway to twenty percent

### `light-named-white` — example phrasings

- set the light to warm white
- switch to cool white

### `light-on-off` — example phrasings

- turn on the kitchen light
- switch off the bedroom lamp

### `light-step` — example phrasings

- make it brighter
- dim the light a little

### `light-temp-step` — example phrasings

- make it warmer
- cool down the light

### `media-duck` — example phrasings

- duck the music
- lower the music while you speak

### `media-next-previous` — example phrasings

- next song
- previous track

### `media-play-pause` — example phrasings

- play music
- pause the music

### `media-volume-set` — example phrasings

- set the volume to thirty
- turn the music down to ten

### `media-volume-step` — example phrasings

- turn it up
- make it quieter

### `podcasts` — example phrasings

- make me a quick podcast about the Øresund bridge
- play the podcast about the bridge in the kitchen

### `tv-volume` — example phrasings

- set the TV volume to 20
- turn the TV up
- mute the TV
- volume up in the living room

### `tv-wake-connected-interface` — example phrasings

- turn on the TV

### `memory-add` — example phrasings

- remember that I like the lights dim
- add to my memory

### `memory-remove` — example phrasings

- forget that I like dim lights
- remove from my memory

### `memory-replace` — example phrasings

- update my memory to say I prefer warm white

### `memory-view` — example phrasings

- what do you know about me
- read my memory

### `scene-apply` — example phrasings

- activate movie mode
- set the scene to dinner

### `scene-delete` — example phrasings

- delete the dinner scene
- remove movie mode

### `scene-list` — example phrasings

- what scenes do we have
- list my scenes

### `scene-room-by-name` — example phrasings

- save this as cosy in the living room
- apply cosy in the living room

### `scene-save` — example phrasings

- save this as movie mode
- remember this scene as dinner

### `skill-background-review` — example phrasings

- review my pending skills
- show background skill suggestions

### `skill-curator` — example phrasings

- run the skill curator
- clean up old skills

### `skill-delete` — example phrasings

- delete the bedtime skill
- remove movie night

### `skill-mint` — example phrasings

- save this as a skill called bedtime
- mint a skill named movie night

### `skill-patch` — example phrasings

- update the bedtime skill
- patch movie night

### `skill-view` — example phrasings

- show me the bedtime skill
- what is in movie night

### `timer-cancel` — example phrasings

- cancel the five minute timer
- stop the timer

### `timer-list` — example phrasings

- what timers are running
- list my timers

### `timer-set` — example phrasings

- set a timer for five minutes
- timer ten minutes

### `voice-training-questions` — example phrasings

- how was my week
- how far did I run on Sunday
- what was my pace on my last run

### `voice-training-today` — example phrasings

- what's my run today
- what run do I have tomorrow
- what's on my training plan
