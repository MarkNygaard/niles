---
name: chat
description: Chat, chatting, typing, typed, dictate, dictation, conversation, Claude.
version: 1.0.0
---
# The app's chat

```yaml
chat:
  where: the Chat page in the Niles app (web and the installed phone app), for anyone signed in
  what: the same Niles as the satellites; typed words go through the same phrasings, tools and memory
  notes:
    - each signed-in person has their own conversation; the last 12 exchanges carry over for 30 minutes,
      kept across a restart
    - "New conversation" starts over
    - the person is known by the voice their sign-in is linked to: their name, notes and calendars come with it
    - no room is assumed and nothing is spoken: name the room ("turn off the kitchen lights"), since "here" means nothing
    - replies are shown as text; a short list or a few lines is fine where it reads better
    - a reply that is only an action shows "Done."
    - the microphone button dictates: recorded in the browser and written down by the same speech-to-text
      as the satellites, into the message field to check and send (not sent by itself)
    - a message is at most 2000 characters
    - Claude Code can answer the chat instead, on the household's own subscription, when switched on under
      Settings → Integrations (Haiku, Sonnet or Opus); it uses Niles's tools and nothing else
    - a reply from Claude is marked "Claude"; when Claude Code fails the chat falls back to Niles's own model and says why
    - spoken commands never go to Claude Code
    - an automation read-back can be answered with a plain "yes" typed in the chat
  examples:
    - User (typed): "lights off" → ask which room
    - User (typed): "what's on my calendar today?" → calendar_events for the signed-in person
    - User: "can I type to you?" → yes, in the Chat page of the Niles app
```
