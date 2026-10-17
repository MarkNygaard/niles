# Podcasts

Ask Niles to research a topic, by voice or in the app. A few minutes later
you get a short episode, at most 8 minutes, of two hosts talking it through,
with the sources listed underneath. Listen in the app or on a Sonos speaker.

It's free, fast, and the scripts are voiced without leaving the house.
Niles does the research with its web search, writes the two-host script, and
then records it itself, line by line, in each host's voice.

The recommended setup is Kokoro, which you run yourself, and a private
Cloudflare R2 bucket with an access key for that bucket only.

## Running Kokoro

The voices come from Kokoro-FastAPI, the fastest way to run the Kokoro
voices. Run it on your own server or cluster, on port 8880, and give Niles
its internal service address, such as `http://kokoro.ns.svc:8880` — a
trailing `/v1` or `/` is fine, Niles tidies it up. Nothing here is sent to a
hosted voice service: the voices are made on your Kokoro.

## Making the bucket

Podcasts are stored in a private Cloudflare R2 bucket:

1. Create one bucket per Niles, and keep it private.
2. Create an API token scoped to that bucket only, with **Object Read &
   Write** permissions.
3. Note the account's S3 endpoint,
   `https://<account-id>.r2.cloudflarestorage.com`.
4. Add a lifecycle rule of about 30 days that deletes old objects. It is
   only a backstop — Niles deletes each episode itself when its time is up.

## Setting up in the app

Under **Settings → Add-ons**, add **Podcasts** and fill in:

- **Kokoro address** — the internal address of your Kokoro-FastAPI.
- **R2 endpoint** — the account's S3 address, with nothing after it.
- **Bucket** — the bucket's name.
- **Region** — `auto` for Cloudflare R2; it is only used for signing.
- **Access key ID** and **Secret access key** — the token's two halves.
  They go into the secret store, never into the config.
- **First host** and **Second host** — the two voices, picked from the
  English voices your Kokoro lists. They must differ.
- **Keep episodes for (days)** — how long an episode is kept, 1 to 365,
  14 by default.

Then press **Test**. It says a one-second sample in each host's voice and
checks it is MP3, then uploads a tiny file, reads it back the same way the
app and Sonos do, and deletes it. **Working** shows only when both parts
pass; otherwise it says which part failed and why. Nothing of the test
answer names your address or keys.

## Asking

By voice or in the app's chat:

- "make me a quick podcast about …"
- "play the podcast about … in the kitchen"

The episode appears under Saved: "Researching…", then "Recording…", then
ready, and a notification says when it is done. "Play … in the kitchen"
plays it on the kitchen's Sonos; a room without a Sonos is told so, and it
never plays on a satellite.

Episodes are in English, whatever language you ask in.

## What is kept, and for how long

Each episode is one MP3 in your bucket and a document under Saved. It is
kept for the number of days you chose, counted from when it was made, and
the episode says until when. After that the recording and the document are
deleted together, and anyone you shared it with is told it's gone. Deleting
an episode yourself removes the recording too. A finished episode cannot be
edited or re-recorded — ask for a new one instead.

## Privacy

- The research goes through Claude or ChatGPT and the web search, like any
  other background job.
- The voices are made on your Kokoro.
- The audio stays in your bucket; nothing is made public.
- A play link is signed and lasts 4 hours, then a new one is made on
  demand. It names the access key ID, never its secret.

## Removing it

Remove the add-on under **Settings → Add-ons**. Its keys and settings are
deleted, and no new episodes can be made. Episodes already recorded stay in
the bucket until its lifecycle rule clears them, and can no longer be
played.
