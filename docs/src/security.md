# Security and privacy

Niles runs in your house, and keeps what it knows in your own database.
This page says who can reach what, what each person keeps to themselves,
and what leaves the house.

## Signing in

The app is reached by signing in with GitHub. GitHub only proves who you
are; the list under **Settings → People** decides who gets in. See
[People and voices](people.md#who-can-sign-in) for adding people.

Signing in needs two things:

- **A GitHub OAuth app.** Make one on github.com under Settings →
  Developer settings → OAuth Apps, with the callback URL
  `https://<your niles host>/auth/github/callback`. Put its client ID and
  secret in the **Signing in** card under **Settings → People**, or give
  them to Niles as environment variables.
- **At least one person on the list.** With nobody listed, signing in is
  off and Niles is open to anyone who can reach it. The People card says
  so.

When you sign in, Niles asks GitHub for your email addresses only. It
matches the address GitHub has verified against the list, ignoring upper
and lower case. An unverified address, or a GitHub username, never
matches.

You stay signed in for 400 days on that browser. **Sign out** is in the
avatar menu. Taking someone off the list locks them out on their next
request. A single signed-in browser cannot be signed out from elsewhere;
the only other way is to change the session key, which signs everybody
out.

There are no roles. Everyone who can sign in can change every setting and
add and remove people.

## The API key

**Settings → API access** has a key that lets scripts and other systems
use Niles. It can do everything a signed-in person can, so treat it like a
password. Send it as `Authorization: Bearer <key>`.

Niles makes the key itself when it has a database. **Show the key** and
**Copy the key** are there for signed-in people only. **New key** replaces
it, and scripts using the old one stop working at once. The key cannot be
used to read or replace itself.

The key does not stand for any person. It has no profile, calendar, mail,
Strava or phone, and everything done with it shares one chat.

## What is yours alone

| | Who sees it |
|---|---|
| Your conversations | You. Your spoken turns join them only when Niles knew your voice. |
| Your Saved items | You, and whoever you share one with. See [Saved](saved.md#sharing). |
| Your approvals and their history | You |
| Your calendars and mail | You, when Niles hears your voice or you ask in the app |
| Your Strava workouts and training plans | You |
| Your notes, birthday and how Niles speaks to you | Niles, when it is talking to you |
| Your notification devices | You |
| The household's memory | Everyone |
| Settings, devices, rules, scenes, presence | Everyone who can sign in |

Something of yours that is not shared with someone gives them no sign that
it exists.

## Passwords and keys

Keys you type into the app, such as an API key or the nemlig.com
password, are kept in Niles's database, encrypted with a key from the
environment variable `NILES_SECRET_KEY`. That key cannot be set from the
app. Lose it or change it, and every stored key has to be typed in again.

Nothing can read a stored key back out. The app only shows whether one is
**saved here** or **from the environment**, and lets you replace or clear
it. Keys never go into the config file, which holds only the names of
environment variables.

A key set by an environment variable is managed there: "Coming from an
environment variable. Remove it there to manage it here instead."

## If Niles can be reached from the internet

You do not need to expose Niles for it to work. Phone notifications reach
you through Apple's, Google's or Mozilla's push service either way.

If you do make it reachable, through a tunnel or your router:

- Put HTTPS in front of it. Niles marks its session cookie secure when the
  proxy in front says the request came over HTTPS.
- Everything is behind sign-in except the health check, the sign-in pages
  themselves, and webhook addresses an add-on asks for. Each of those
  carries a long secret token, and Niles limits how often it may be called.
- The app's live updates only accept connections from the app's own page.
- Pairing a phone works only from the home network.
- Keep the satellites' port, 10300, on the home network. Satellites do not
  sign in.

**Settings → API access → Public address** tells Niles where the internet
reaches it, for add-ons that build sign-in and webhook addresses on it.
Setting it opens nothing; only what your router or tunnel lets through is
reachable.

## History

**Settings → History** lists the last 50 changes to settings, newest
first, whether made in the app or by voice. Each row says what changed,
such as `lighting.daytime_brightness 100 → 85`, and when. It does not say
who. **Undo last change**, above the settings, puts back the last change;
press it again to go back further. Changes to stored keys are not in the
history.

## What Niles keeps

- Conversations: your last 100.
- Approvals: what you decided, your last 500, without the values.
- Voices: a fingerprint per sample, never the recording.
- Wake recordings: only if someone switches on **Keep wake recordings**
  under **Settings → People → Keep what it hears**. Then the last 500, with
  **Delete all**. Off unless you switch it on.
- Workouts fetched from Strava. They stay when you disconnect.
- Files attached in the chat, with their conversation.
- The log: the last 5,000 lines, in memory only, gone at a restart. It
  includes what was said to Niles and what it answered. The app does not
  show it, but anyone signed in, or holding the API key, can fetch it.

Niles keeps no copy of your mail. It reads what a question needs from the
provider when asked.

## What leaves the house

There is no telemetry: Niles sends nothing about you or your use of it to
its authors or anyone else. These are the outside services it does use:

| Service | What goes there |
|---|---|
| Speech-to-text (ElevenLabs by default) | The audio of each command after the wake word |
| Niles's own model (Cerebras by default) | The words of a command no fixed phrasing caught, with the context it needs |
| Claude or ChatGPT, if switched on | Chat messages, files you attach, rules to work out, and the background review |
| Calendar, mail, Strava, tado°, Spotify and other add-ons you connect | What each one needs to answer |
| Open-Meteo | The house's location, for the weather |
| OpenStreetMap | Map tiles and the address search under **Settings → Home** |
| Apple, Google or Mozilla push | Phone notifications |
| GitHub | Signing in, and your profile picture |

Speaking the replies (Piper), recognising voices and the fixed phrasings
all run on your own Niles.
