---
name: memory
description: Memory, memories, remember, forget, birthday, notes, learn, learned, teach, skill, skills, routine, routines.
version: 1.0.0
---
# Memory and skills

```yaml
memory:
  household: "remember that the bins go out on Tuesdays" | "forget what I said about the bins"
  personal: "remember I take my coffee black" | "forget that I'm vegetarian" | "what do you know about me"
  birthday: "my birthday is the 3rd of May"
  skill: "remember how to do <routine>" | "save that as a routine" | "forget how to <routine>"
  tools:
    memory: action add|replace|remove|view, target user (facts about the household) | agent (what you have learned);
            replace/remove take old_text matching exactly one entry
    remember_about_me: add|remove|view one short fact in the speaker's own notes
    set_my_birthday: month, day, year only if said; they are wished a happy birthday on the day
    mint_skill: name (short kebab-case), description, version, body (the how-to, not the conversation)
    patch_skill: replace an existing skill's body; prefer it over a new skill when one overlaps
    view_skill: read a skill listed under Available skills before following it
    delete_skill: absorbed_into names the skill that took its content; pinned skills cannot be deleted
  notes:
    - both memory documents and the speaker's notes are in the prompt on every turn; no need to view before answering
    - a fact about one person goes in remember_about_me; memory is for the household as a whole
    - remember_about_me and set_my_birthday need a recognised voice (or a chat linked to one)
    - mint a skill only when the person explicitly asks to save or learn a routine
    - after a successful turn a background review may add memory or write a skill on its own;
      never failures, timers, readings or weather
    - skills Niles wrote itself go stale after 30 days unused and are archived after 90; ones the person asked for never are
    - the app's Skills page lists every skill: pin, archive, restore or delete (delete is refused while pinned)
    - short-term: the last few exchanges per room (3 minutes) and per chat (30 minutes) carry over, across a restart
  examples:
    - User: "remember that the cleaner comes on Thursdays" → memory add target=user
    - User: "I'm allergic to nuts" → remember_about_me add "allergic to nuts"
    - User: "learn this: movie night means TV on, living room lights at 20%" → mint_skill movie-night
```
