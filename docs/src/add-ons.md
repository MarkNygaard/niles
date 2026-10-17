# Add-ons and accounts

Add-ons are the optional extras: music, the TV, presence, heating,
calendars, shopping, the assistants and web search, and add-ons other
people wrote. Everything Niles needs to work at all (the broker, hearing
and thinking) is under **System** in Settings instead.

Accounts are personal. Each person connects their own calendar, mail and
Strava from **My profile**.

## The Add-ons page

**Settings → Add-ons** has two tabs, **Connected** and **Available**, and
a search box. Press **Connect** next to an add-on to add it and open its
settings. A connected add-on shows a status line, such as **Working** or
**Needs its address**; click it to change its settings or **Remove
add-on**. Removing one deletes its saved keys too.

| Add-on | What it does | What it needs |
|---|---|---|
| Sonos | Music and radio in each room, turned down while Niles speaks. | Sonos speakers on the same network as Niles. |
| Spotify | Any artist, song or album, played on Sonos. | Sonos, Spotify Premium linked in the Sonos app, and a free Spotify developer app for its two keys. |
| LG TV | On and off, apps and inputs by voice. | An LG TV on the same network. You accept a prompt on the TV once. |
| UniFi | Knows who is home from their phone. | A UniFi console on your network and an API key made in UniFi Network. |
| tado° | The heating in each room by voice. | tado° thermostats and their account. |
| Microsoft calendars | Outlook and work calendars and mail. | A free app registration in Microsoft Entra, made once for the house. |
| Google calendars | Google calendar and Gmail. | A free Google Cloud project, made once for the house. |
| nemlig.com | Turns the shopping list into a filled basket. | A nemlig.com account's email and password. |
| Claude Code | Claude answers the chat and handles new rules. | A paid Claude subscription. |
| ChatGPT | ChatGPT answers the chat and handles new rules. | A paid ChatGPT subscription (Plus or Pro). |
| Linear | Issues, projects and comments, through Linear's own tools. | A Linear workspace and a personal API key. To hear when an issue moves, a Linear webhook and its signing secret. |
| Web search (SearXNG) | Web search and pictures for Claude and ChatGPT. | A SearXNG instance Niles can reach. |

iCloud calendars need nothing here. Each person connects theirs from
their profile.

## Claude and ChatGPT

Claude or ChatGPT can answer the chat, work out new rules, write what
goes under [Saved](saved.md), look at files, and review the day for
things worth remembering. They run on your own subscription and use only
Niles's tools.

**Claude Code** needs a token. On a computer signed in to your Claude
account, run `claude setup-token` and paste what it prints. It lasts a
year. Pick a model: Haiku is quickest, Sonnet balanced (the default),
Opus most capable.

**ChatGPT** is signed in from its panel. Press **Sign in with ChatGPT**
and sign in with the account that has the subscription. The page you land
on afterwards fails to load; that is expected. Copy its address from the
address bar, paste it into the box and press **Connect**. Niles keeps the
sign-in and renews it itself. This needs Niles's database and its secrets
key.

Then choose which one answers under **Settings → Voice & AI → Assistant**.
"Niles's own model" means neither does, and nothing runs in the
background.

## Web search

Claude and ChatGPT can search the web in one of two ways.

- **Without SearXNG**, Claude uses its own built-in search. ChatGPT
  cannot search, and says when it answers from memory. Saved items get no
  pictures.
- **With the Web search (SearXNG) add-on**, both search through your own
  SearXNG, and find pictures for what they save with its image engines.
  Claude's own search is off while SearXNG is there.

To add it, give its **SearXNG address**, such as
`https://search.example.com/search`, and press **Test**. The test runs one
web search and one picture search and says how many results each got.

A new SearXNG has its JSON output off, and Niles needs it. In SearXNG's
`settings.yml`, add `json` under `search` → `formats`. For pictures, switch
on an image engine too, such as Wikimedia Commons or Bing Images.

Either way, when an assistant opens a page a search found, Niles fetches
it itself, and only from the public internet. What a page says is treated
as information, never as instructions.

## Linear and other tool servers

**Linear** adds Linear's own tools: issues, projects and comments. Give it
a personal API key. Its panel then lists every tool, each set to **On**,
**Ask first** or **Off**. A tool set to ask first waits for your yes; see
[Approvals](approvals.md).

**Let Claude or ChatGPT handle these** sends a spoken request for these
tools to the assistant, which is slower and better at writing. Off keeps
quick lookups on the fast model. **Notes for Niles, given with these
tools** tells the model how your workspace is used, such as which team a
coding task goes to.

### Hearing when an issue moves

Linear can also tell Niles when an issue goes to review or is done, and
Niles says so: "The Fix the hall light task has a pull request ready for
review." It is an announcement for the house, spoken in every room with a
satellite at Important priority.

1. In Linear, open **Settings → API → Webhooks** and make a webhook for
   **Issues**. Pick the teams you want to hear about there; Niles
   announces every team the webhook sends.
2. Send it to your Niles's address followed by `/webhooks/linear`. The
   panel shows the full address. Linear has to be able to reach it, so
   this needs Niles on an address outside the house.
3. Copy the webhook's signing secret into **Linear webhook signing
   secret** on Linear's panel.

The webhook is on once the secret is saved, with no restart. Until then
the address answers as if it did not exist. Niles checks Linear's
signature on every call and ignores one more than a minute old. Removing
the Linear add-on deletes the secret too.

Linear is the one tool server the app can add. Others can be written into
Niles's config file by whoever runs it.

## Add-ons from GitHub

Somebody else's add-on, for what Niles does not cover, installs from a
GitHub release. On the **Available** tab, under **From GitHub**, press
**Add** next to **Add-on from GitHub**. Paste the repository's address,
such as `jane/niles-spotprice`, and press **Preview**.

Nothing is installed yet. The preview shows the add-on's name, version,
author and description, and a list under **It will be able to**: the
addresses it may reach, the devices it adds, the tools it offers the
assistant, whether it may speak through your satellites, and so on. It
always ends with: "It cannot see your conversations, the people in your
house, other add-ons' passwords or Niles's database." **What its tools do**
lists each tool. Press **Install** if you agree.

An add-on runs walled off from the rest of Niles. It has no files and no
network beyond the addresses it declared, and it can reach your home
network only if the list said so.

Its panel shows where it came from, its version and its log, and the
settings it asks for. It starts once the required ones are filled in.

**Updates.** Niles checks for a newer release every six hours but never
updates by itself. The row says **Update available**, and **Update…** shows
what changes, including anything more the new version would be able to
do. If an update fails within ten minutes, Niles puts the old version
back and says so. **Roll back to** the previous version goes back by hand.
**Check for updates** looks at once.

**Turn off** stops it and keeps everything. **Remove add-on** takes away
its devices, pages and widgets, and deletes its settings, saved keys and
stored data. Rules that used its devices are kept.

## Calendars and mail

**For the house, once.** Someone sets up the Microsoft or Google add-on
with an app registration, following the steps on its panel, and pastes
its client ID and secret. **Use Microsoft calendars** or **Use Google
calendars** switches it off without losing anything. iCloud needs no
setup here.

**For each person.** Under **My profile → Calendars** and **My profile →
Mail**, pick Microsoft, Google or iCloud, give the account a short name
such as Private or Work, and choose what Niles may do:

| | Read only | Read and write / Read and send |
|---|---|---|
| Calendar | Niles can tell you what's on it. | Niles can also add, move and delete events. |
| Mail | Niles can tell you what's in your mail. | Niles can also write mail, which you check and send in the app. |

Microsoft and Google send you to their own sign-in. A work account may
need your IT department's approval. Google warns that it has not verified
the app when it is asked for Gmail; that is expected for your household's
own app.

For iCloud, give your Apple ID and an app-specific password, made at
appleid.apple.com under **Sign-In and Security → App-Specific Passwords**.
Apple has no read-only password, so for iCloud "read only" is a rule
Niles keeps. You can revoke the password at appleid.apple.com at any time.

An account connected for mail can be reused for the calendar, and the
other way round. Changing what a Microsoft or Google account may do means
signing in again.

Calendars and Mail only show on your profile once Niles knows your voice
and it is linked to your sign-in. See [People and voices](people.md).

**What you can ask.** "What's on my calendar tomorrow?", "Put the dentist
in on Thursday at ten", "Any unread mail from the school?", "Reply that
we'll be there." Niles reads all your connected calendars together. It
uses the default calendar of a Microsoft account, the primary one of a
Google account, and every calendar of an iCloud account.

Before it adds, moves or deletes an event, Niles reads it back and asks.
Niles never sends a mail it wrote on its own. The mail waits in the app
until you press **Allow once**. See [Approvals](approvals.md).

**Whose.** Niles uses the calendar and mail of the voice it heard, or of
the person signed in to the chat. Nobody can ask for someone else's. A
voice Niles does not know gets no calendar and no mail.

Niles keeps no copy of your mail. It reads what a question needs from
the provider, when asked.
