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
    remember_about_me: add|remove|view one short fact in the speaker's own notes, when they ask
    set_my_birthday: month, day, year only if said; they are wished a happy birthday on the day
    mint_skill: name (short kebab-case), description ("Use when ...", at most 60 characters), version, body (the how-to, not the conversation)
    patch_skill: old_text + new_text replaces one place in the body (view_skill first); body alone rewrites it; prefer it over a new skill when one overlaps
    view_skill: read a skill listed under Available skills before following it
    delete_skill: absorbed_into names the skill that took its content; pinned skills cannot be deleted
  notes:
    - both memory documents and the speaker's notes are in the prompt on every turn; no need to view before answering
    - a fact about one person goes in remember_about_me; memory is for the household as a whole
    - remember_about_me and set_my_birthday need a recognised voice (or a chat linked to one)
    - mint a skill only when the person explicitly asks to save or learn a routine
    - a note that is one already kept, in other words, is refused: replace the existing one instead
    - when the house has been quiet for 5 minutes, Claude reviews what was said and may add a note, add to a
      person's own notes (what is lasting about them, said or shown, without "remember"), or write a skill;
      once a week it tidies memory and skills (merges, removes notes about finished things, folds overlapping skills)
    - anything taken out of memory is listed in Settings, Memory, under Recently forgotten, and can be restored
    - skills Niles wrote itself go stale after 30 days unused and are archived after 90; ones the person asked for never are
    - the app's Skills page lists every skill: pin, archive, restore, delete (refused while pinned) and undo its last change
    - short-term: the last few exchanges per room (3 minutes) and per chat (30 minutes) carry over, across a restart
  examples:
    - User: "remember that the cleaner comes on Thursdays" → memory add target=user
    - User: "I'm allergic to nuts" → remember_about_me add "allergic to nuts"
    - User: "learn this: movie night means TV on, living room lights at 20%" → mint_skill movie-night
```
