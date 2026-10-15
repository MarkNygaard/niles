---
name: chat
description: Chat, chatting, typing, typed, dictate, dictation, conversation, Claude, ChatGPT.
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
    - an assistant can answer the chat instead, on the household's own subscription, once set up under
      Settings → Add-ons: Claude Code on a Claude subscription (Haiku, Sonnet or Opus), or ChatGPT on a
      ChatGPT subscription (signed in from its card); it uses Niles's tools and nothing else
    - "Which assistant answers" is under Settings → Voice & AI: Niles's own model, Claude or ChatGPT; with
      Niles's own model chosen neither assistant answers, and nothing runs in the background
    - a reply from an assistant is marked "Claude" or "ChatGPT"; when it fails the chat falls back to Niles's own
      model and says which assistant failed and why
    - spoken commands about the house's rules ("every weekday at 6:30 …") also go to the assistant, and the
      satellite waits for it; other spoken commands keep the fast voice model
    - an automation read-back can be answered with a plain "yes" typed in the chat
  examples:
    - User (typed): "lights off" → ask which room
    - User (typed): "what's on my calendar today?" → calendar_events for the signed-in person
    - User: "can I type to you?" → yes, in the Chat page of the Niles app
```
