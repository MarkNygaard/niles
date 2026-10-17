# Voice loop

Niles processes every command, spoken or typed, through a tiered pipeline
that trades latency against capability. The loop is deterministic at the
bottom and a language model above it. The share of commands each tier
takes and the timings below are design targets, not measurements.

For what a person can say, see [Talking to Niles](talking.md).

## Tier 0: regex router

Tier 0 is a deterministic regex router in `niles-intent`. It matches
anchored phrasings such as "turn on the kitchen lights" or "set a timer for
five minutes" and acts without touching an LLM. It covers lights, blinds,
scenes, music and the TV, timers, heating, the weather, the clock, and
introducing a voice. The target is most everyday commands, end to end in
around half a second.

Courtesy ("please", "can you", "thanks") is stripped before matching. A
sentence phrased as a condition ("when the door opens...") is never matched
here, so it reaches the model as a rule to make.

## Tier 1: fast LLM with tools

When Tier 0 misses, the request goes to Tier 1: a fast LLM (Cerebras, by
default, set under Settings → Voice & AI) with the tool registry. Tools are
offered by topic, from keywords in the request, so a turn carries only the
tools it is likely to need. The default request timeout is 30 seconds.

In the app's chat, and by voice for rules and handed-off tool servers, the
assistant (Claude Code or ChatGPT) answers instead when one is switched
on. See [Chat](chat.md).

## Tier 2: optional escalation

Tier 2 is off unless the config file has an `[llm.tier2]` section. There
is no setting for it in the app. When configured it defaults to OpenAI's
`gpt-5.5` with a 30-second timeout, and Tier 1 reaches it only by calling
the `escalate_to_tier2` tool.

## System prompt layout

The system prompt is assembled from stable to volatile, so a provider's
prefix cache keeps as much of it as possible:

1. **Persona**: who Niles is and how it speaks.
2. **Tool guidance**: how to use capability references and skills, when
   those tools are registered.
3. **Household context**: the home's name, country, locale, timezone,
   units and language. Not devices or their states; those come from tools.
4. **User memory**: notes about the household.
5. **Agent memory**: what Niles has noted.
6. **Skills**: the available skills, by name and description.
7. **Capability references**: the reference for each topic detected in
   the request, and only those.
8. **Origin room**: which satellite heard the command.
9. **Speaker context**: who is speaking, when the voice is recognised.
10. **Current time**: last, because it changes every turn.

## Background review

Every exchange goes into a journal. Once the house has been quiet for 5
minutes, the assistant reads what was said since the last review and
decides what is worth keeping: a fact for memory, a note about a person,
or a skill. Most of the time it keeps nothing. Once a week, after two
hours of quiet, it tidies memory and skills. It merges notes that say the
same thing, takes out what is finished, and folds overlapping skills
together.

The review only adds. The tidy-up may remove, and what it removes can be
restored from the app (Settings → Memory, Recently forgotten, and the
Skills page). Neither can touch the house. Their only tools are memory,
skills and the list of automations. It needs Claude Code or ChatGPT; with
"Niles's own model" chosen as the assistant, nothing runs in the
background.

The skill tools are `mint_skill`, `patch_skill`, `view_skill`,
`list_skills`, `retire_skill` and `delete_skill`.

For the deep dive on every subsystem, see the canonical
[Overall architecture](./architecture.md) chapter.
