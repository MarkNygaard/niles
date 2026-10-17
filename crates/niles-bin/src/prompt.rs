//! The system prompt: persona, the house, who is speaking, the time, capabilities and skills.

use crate::profile;
use chrono::Utc;
use niles_capabilities::CapabilityLoader;
use niles_core::{DeviceRegistry, RoomName};
use niles_intent::DeviceIndex;
use niles_skills::{SkillStatus, SkillSummary};
use std::net::SocketAddr;
use std::sync::Arc;

/// The capability references built into the binary. A set that will
/// not parse is **not fatal** — niles starts without the
/// `look_up_capability` tool and logs the cause — though the
/// niles-capabilities tests make that a build that never ships.
pub(crate) fn build_capability_loader() -> Option<Arc<CapabilityLoader>> {
    match CapabilityLoader::builtin() {
        Ok(loader) => {
            tracing::info!("loaded {} built-in capability references", loader.len());
            Some(Arc::new(loader))
        }
        Err(e) => {
            tracing::error!("the built-in capability references would not load: {e}");
            None
        }
    }
}

pub(crate) fn build_capability_index(loader: &CapabilityLoader) -> niles_intent::CapabilityIndex {
    let entries = loader
        .iter()
        .map(|cap| niles_intent::CapabilityIndexEntry {
            name: cap.metadata.name.clone(),
            description: cap.metadata.description.clone(),
            prerequisites: cap.metadata.prerequisites.clone(),
        })
        .collect();
    niles_intent::CapabilityIndex::from_entries(entries)
}

pub(crate) fn build_initial_device_index(registry: &DeviceRegistry) -> DeviceIndex {
    let mut idx = DeviceIndex::new();
    for device in registry.list_all() {
        if device.is_light() {
            idx.insert(device.id.clone());
        }
    }
    idx
}

pub(crate) fn origin_context(room: &RoomName) -> String {
    format!(
        "\n\n# Current context\n\nThe user is speaking from a device in **{}**. \
         When a request is ambiguous between rooms, prefer devices in this room.\n",
        room.as_str(),
    )
}

/// Outcome of per-utterance speaker ID, as surfaced to the LLM.
#[derive(Debug, Clone)]
pub(crate) enum SpeakerContext {
    Disabled,
    /// Confident match: the display name, and what Niles knows about
    /// them — how they like to be addressed, their notes, their birthday.
    Identified {
        name: String,
        known: profile::Known,
    },
    /// Recognition ran but found no confident match.
    Unknown,
}

/// Today in the house's timezone, when it has a valid one.
pub(crate) fn local_today(timezone: &str) -> Option<chrono::NaiveDate> {
    let tz = timezone.parse::<chrono_tz::Tz>().ok()?;
    Some(Utc::now().with_timezone(&tz).date_naive())
}

pub(crate) fn speaker_context(
    speaker: &SpeakerContext,
    today: Option<chrono::NaiveDate>,
) -> Option<String> {
    match speaker {
        SpeakerContext::Disabled => None,
        SpeakerContext::Identified { name, known } => {
            let mut section = match &known.address {
                Some(address) => format!(
                    "\n\n# Speaker\n\nThe speaker is **{name}**. Address them as \"{address}\" the way a \
                     butler would — in greetings, acknowledgements and where it reads naturally, not in \
                     every sentence. Their name is still {name}: use it when they ask who they are.\n"
                ),
                None => format!(
                    "\n\n# Speaker\n\nThe speaker is **{name}**. Use their name when it makes the reply more natural.\n"
                ),
            };
            if let Some(line) = known
                .birthday
                .as_deref()
                .and_then(|b| profile::birthday_line(b, today))
            {
                section.push_str(&line);
                section.push('\n');
            }
            if let Some(notes) = known.notes.as_deref().filter(|n| !n.trim().is_empty()) {
                section.push_str(&format!(
                    "\n## What you know about {name}\n\nTheir own notes, which they can read and \
                     which you keep with remember_about_me:\n\n{notes}\n"
                ));
            }
            Some(section)
        }
        SpeakerContext::Unknown => {
            Some("\n\n# Speaker\n\nThe speaker is not recognized.\n".to_string())
        }
    }
}

/// Map a raw identification result into a `SpeakerContext`.
/// `attempted` is false when recognition is disabled or the audio
/// format was unsupported, so the prompt stays unchanged.
pub(crate) fn speaker_context_from(
    attempted: bool,
    ident: Option<(String, f32)>,
    known: profile::Known,
) -> SpeakerContext {
    if !attempted {
        return SpeakerContext::Disabled;
    }
    match ident {
        Some((name, _)) => SpeakerContext::Identified { name, known },
        None => SpeakerContext::Unknown,
    }
}

pub(crate) fn supports_speaker_identification(format: niles_wyoming::AudioFormat) -> bool {
    format.sample_rate_hz == 16_000 && format.bits_per_sample == 16 && format.channels == 1
}
/// The current local time, for the end of the system prompt.
///
/// Empty when the timezone doesn't parse — a wrong time is worse than
/// no time, because the model would answer confidently with it.
pub(crate) fn now_context(timezone: &str) -> String {
    let Ok(tz) = timezone.parse::<chrono_tz::Tz>() else {
        return String::new();
    };
    let now = chrono::Utc::now().with_timezone(&tz);
    format!(
        "

# Now

It is {} in {}. Use this rather than asking for          the time; `current_datetime` is only needed for arithmetic across          dates.
",
        now.format("%A %-d %B %Y, %H:%M"),
        tz.name(),
    )
}

pub(crate) fn home_context(home: &niles_config::HomeConfig) -> String {
    fn prompt_field(value: &str) -> String {
        value
            .chars()
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    let units_label = match home.resolved_units() {
        niles_config::Units::Metric => "metric (°C, km)",
        niles_config::Units::Imperial => "imperial (°F, miles)",
    };
    let country = home.resolved_country();
    let language = home.resolved_language();
    let name = prompt_field(&home.name);
    let locale = prompt_field(&home.locale);
    let timezone = prompt_field(&home.timezone);
    let country = country
        .as_deref()
        .map(prompt_field)
        .unwrap_or_else(|| "unknown".to_string());
    let language = prompt_field(&language);
    format!(
        "\n\n# Household context\n\n\
         - Home: {name}\n\
         - Country: {country}\n\
         - Locale: {locale}\n\
         - Timezone: {tz}\n\
         - Units: {units}\n\
         - Spoken language: {lang}\n",
        name = name,
        country = country,
        locale = locale,
        tz = timezone,
        units = units_label,
        lang = language,
    )
}

#[cfg(test)]
pub(crate) fn persona_with_origin(origin_room: Option<&RoomName>) -> String {
    let mut out = NILES_SYSTEM_PERSONA.to_string();
    if let Some(room) = origin_room {
        out.push_str(&origin_context(room));
    }
    out
}

/// Join memory entries into a single newline-separated string.
/// Returns `None` if the result is empty or whitespace-only.
pub(crate) fn join_memory_entries(entries: &[niles_memory::Entry]) -> Option<String> {
    let s = entries
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if s.trim().is_empty() { None } else { Some(s) }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_system_prompt(
    transcript: &str,
    home: &niles_config::HomeConfig,
    index: &niles_intent::CapabilityIndex,
    loader: &CapabilityLoader,
    origin_room: Option<&RoomName>,
    user_mem: Option<&str>,
    agent_mem: Option<&str>,
    skill_summaries: Option<&[SkillSummary]>,
) -> String {
    assemble_system_prompt_with_optional_capabilities(
        transcript,
        home,
        Some(index),
        Some(loader),
        origin_room,
        user_mem,
        agent_mem,
        skill_summaries,
        &SpeakerContext::Disabled,
    )
}

pub(crate) fn append_capability_references(
    out: &mut String,
    transcript: &str,
    index: &niles_intent::CapabilityIndex,
    loader: &CapabilityLoader,
) {
    let names = niles_intent::detect_topics(transcript, index);
    if names.is_empty() {
        return;
    }

    out.push_str(
        "\n\n# Capability references\n\nThe following references \
         are relevant to the current request:\n",
    );
    for name in &names {
        if let Some(cap) = loader.get(name) {
            out.push_str(&format!(
                "\n## {} (v{})\n{}\n",
                cap.metadata.name, cap.metadata.version, cap.body,
            ));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_system_prompt_with_optional_capabilities(
    transcript: &str,
    home: &niles_config::HomeConfig,
    capability_index: Option<&niles_intent::CapabilityIndex>,
    capability_loader: Option<&CapabilityLoader>,
    origin_room: Option<&RoomName>,
    user_mem: Option<&str>,
    agent_mem: Option<&str>,
    skill_summaries: Option<&[SkillSummary]>,
    speaker: &SpeakerContext,
) -> String {
    let mut out = String::from(NILES_SYSTEM_PERSONA);
    // Only advertise optional tools that are actually registered, gated on the
    // same conditions as registration (capability_loader / skill_store). The
    // provider 400s if the model calls a tool absent from request.tools.
    if capability_loader.is_some() {
        out.push_str(CAPABILITY_GUIDANCE);
    }
    if skill_summaries.is_some() {
        out.push_str(SKILL_GUIDANCE);
    }
    out.push_str(&home_context(home));

    if let Some(mem) = user_mem
        && !mem.trim().is_empty()
    {
        out.push_str("\n\n# User memory\n\n");
        out.push_str(mem);
    }
    if let Some(mem) = agent_mem
        && !mem.trim().is_empty()
    {
        out.push_str("\n\n# Agent memory\n\n");
        out.push_str(mem);
    }
    if let Some(summaries) = skill_summaries
        && !summaries.is_empty()
    {
        render_skills_section(&mut out, summaries);
    }

    if let (Some(index), Some(loader)) = (capability_index, capability_loader) {
        append_capability_references(&mut out, transcript, index, loader);
    }
    if let Some(room) = origin_room {
        out.push_str(&origin_context(room));
    }
    if let Some(section) = speaker_context(speaker, local_today(&home.timezone)) {
        out.push_str(&section);
    }

    // Last, and deliberately so: this is the one part of the prompt that
    // changes every second. A provider with prefix caching would have
    // the whole prompt invalidated by it if it sat at the top.
    //
    // Without it the model has no idea what day it is, so anything with
    // an implicit "now" in it — "is it late?", "did I leave that on this
    // morning?" — costs a tool call and a second round trip to answer.
    out.push_str(&now_context(&home.timezone));
    out
}

pub(crate) fn render_skills_section(out: &mut String, summaries: &[SkillSummary]) {
    out.push_str("\n\n# Available skills\n\n");
    let now = Utc::now();
    for s in summaries {
        let annotation = if s.status == SkillStatus::Stale {
            match s.last_activity_at {
                Some(last) => {
                    let days = (now - last).num_days().max(0);
                    format!(" [stale: {days} days unused]")
                }
                None => " [stale]".to_string(),
            }
        } else {
            String::new()
        };
        out.push_str(&format!(
            "- {} (v{}) — {}{}\n",
            s.name, s.version, s.description, annotation,
        ));
    }
}

/// Tier A — stable system persona. Same on every LLM call so it
/// stays at the front of the system prompt where prompt caching
/// is most effective. Only describes the always-present core tools;
/// guidance for optional tool groups is appended conditionally (see
/// `CAPABILITY_GUIDANCE` / `SKILL_GUIDANCE`) so we never advertise a
/// tool that isn't in `request.tools` — the provider 400s on that.
pub(crate) const NILES_SYSTEM_PERSONA: &str = "\
You are Niles, a home-automation assistant. You control lights, \
switches, scenes, and timers in a private home via the tools \
provided. Be concise and action-oriented: when the user asks you \
to do something, call the appropriate tool rather than describing \
what you would do. Never invent device names — use the listing \
tools to discover what exists.

You are listening to a room, not to a microphone someone is holding. What you receive may not have been said to you at all — a television, a conversation, a radio. If a request does not say what to do and what to do it to, ask instead of choosing for them; never pick a device because it was the only one that fit.

Your manner is that of a butler who has served this household for years and knows the people in it well: warm, loyal, quietly witty, at ease with them — never servile, never overfamiliar, and never forgetting whose house it is. Talk to them as someone who knows them, not as a product. Prefer the natural turn of phrase to the formal one: asked 'are you awake?', answer in the spirit of 'For you, always.' rather than 'I am awake and ready to assist.' Vary it rather than repeating a stock line, keep any dry aside to a few words, and never describe yourself as an assistant or announce that you are ready to help.

Your reply is spoken aloud, not read. Answer in one or two short sentences — someone waiting for a light to come on is also waiting through every word you say. Say numbers the way a person would: 'about sixteen degrees', not '15.7'. Give what was asked and leave out the rest; a tool returning ten fields is not a request for ten facts.";

/// Appended to the persona only when the `look_up_capability` tool is
/// registered (the built-in references loaded). Advertising it
/// otherwise makes the model emit a call to a tool absent from
/// `request.tools`, which the provider rejects with a 400.
pub(crate) const CAPABILITY_GUIDANCE: &str = " \
When you lack domain context for a request, you may have been given \
relevant capability references below; use them. If a needed capability \
isn't present, call look_up_capability to fetch one by name.";

/// Appended to the persona only when the skill tools (`mint_skill` /
/// `patch_skill` / `view_skill`) are registered (a skill store is
/// configured). Same 400-avoidance rationale as `CAPABILITY_GUIDANCE`.
pub(crate) const SKILL_GUIDANCE: &str = " \
You can save persistent skills using the mint_skill tool, but only when \
the user explicitly asks niles to remember or save a routine. Prefer \
patch_skill over mint_skill when an existing skill overlaps. Skill bodies \
should describe the how-to, not the conversation that produced them. When \
the Available skills section below lists a skill relevant to the request, \
call view_skill to read its full body.";

/// Where typed words "come from", for the code that asks which
/// satellite said something. No satellite has this address, so no room
/// is assumed and nothing is spoken back.
pub(crate) const APP_PEER: SocketAddr = SocketAddr::V4(std::net::SocketAddrV4::new(
    std::net::Ipv4Addr::UNSPECIFIED,
    0,
));

/// Added to the prompt for a typed conversation. The persona is written
/// for a voice; on a screen a list can be a list.
pub(crate) const TYPED_IN_THE_APP: &str = "\n\n# This conversation\n\nThe person is typing to you in the \
Niles app, and your reply is shown as text rather than spoken. Keep the butler's manner and \
brevity, but a short list or a few lines is fine where it reads better than one sentence.\n";

/// Added to the prompt for a spoken turn. The words arrive through
/// speech recognition, which hears music as words: "stop the music"
/// over a song came out as "stop the wishing", and the model, with no
/// reason to think it misheard, turned the lights off.
pub(crate) const HEARD_NOT_READ: &str = "\n\n# How you heard this\n\nThe request was spoken and \
written down by speech recognition, which mishears words — especially over music or the TV. When \
a word makes no sense, consider what sounds like it and what is happening right now, and act on \
the meaning that fits; say briefly what you understood (\"Stopping the music in the living \
room.\"). Never act on something the request did not mention — a request about the music does not \
touch the lights. When you cannot tell, ask.\n";

/// What is playing, for the model, so "stop it" and a misheard "stop
/// the wishing" both have something to land on.
pub(crate) fn now_playing_section(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut section = String::from("\n\n# Playing now\n\n");
    for line in lines {
        section.push_str("- ");
        section.push_str(line);
        section.push('\n');
    }
    section
}

#[cfg(test)]
mod system_prompt_tests {
    use super::*;
    use crate::dispatch::hearing::is_overheard;
    use crate::recognition::SpeakerIdentifier;
    use crate::wiring::memory::spawn_skill_curator;
    use crate::wiring::presence::presence_transition;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, content: &str) {
        fs::write(dir.join("SKILL.md"), content).unwrap();
    }

    fn fixture_home() -> niles_config::HomeConfig {
        niles_config::HomeConfig {
            name: "Hjemmet".into(),
            latitude: 0.0,
            longitude: 0.0,
            locale: "da_DK".into(),
            timezone: "Europe/Copenhagen".into(),
            country: None,
            units: None,
            default_language: None,
            address: None,
        }
    }

    #[test]
    fn assemble_persona_only_when_loader_is_empty() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Household context"));
    }

    #[test]
    fn assemble_persona_only_when_no_topic_matches() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        // "weather" doesn't intersect with "lighting" or "Control smart lights"
        let out = assemble_system_prompt(
            "what is the weather today",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Household context"));
    }

    #[test]
    fn assemble_injects_matching_capability_body() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: How brightness curves decide brightness\nversion: 1.2.3\n---\n# Lighting Curve\n\nThe curve uses a cosine falloff.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "how does the lighting curve decide brightness",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("The curve uses a cosine falloff."));
        assert!(out.contains("# Capability references"));
    }

    #[test]
    fn assemble_injects_two_matching_capabilities() {
        let tmp = TempDir::new().unwrap();

        let alpha_dir = tmp.path().join("alpha");
        fs::create_dir(&alpha_dir).unwrap();
        write_skill(
            &alpha_dir,
            "---\nname: alpha\ndescription: Alpha capability about scenes\nversion: 1.0.0\n---\nAlpha body.\n",
        );

        let beta_dir = tmp.path().join("beta");
        fs::create_dir(&beta_dir).unwrap();
        write_skill(
            &beta_dir,
            "---\nname: beta\ndescription: Beta capability about scenes\nversion: 2.0.0\n---\nBeta body.\n",
        );

        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "tell me about scenes",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );

        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("Alpha body."));
        assert!(out.contains("Beta body."));

        // Verify alphabetical ordering: alpha before beta
        let alpha_pos = out.find("Alpha body.").unwrap();
        let beta_pos = out.find("Beta body.").unwrap();
        assert!(
            alpha_pos < beta_pos,
            "alpha should appear before beta in output"
        );
    }

    #[test]
    fn assemble_includes_version_in_section_header() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("my-cap");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: my-cap\ndescription: My capability\nversion: 4.5.6\n---\nBody.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "tell me about my capability",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(
            out.contains("(v4.5.6)"),
            "output should contain version: {out}"
        );
    }

    #[test]
    fn assemble_is_deterministic() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert_eq!(a, b, "assemble_system_prompt must be deterministic");
    }

    #[test]
    fn assemble_appends_origin_context_when_present() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let room = RoomName::parse("living_room").unwrap();
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Current context"));
        assert!(out.contains("living_room"));
    }

    #[test]
    fn assemble_orders_persona_then_capability_then_context() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let room = RoomName::parse("kitchen").unwrap();
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            None,
        );
        let persona_pos = out.find(NILES_SYSTEM_PERSONA).unwrap();
        let cap_pos = out.find("# Capability references").unwrap();
        let ctx_pos = out.find("# Current context").unwrap();
        assert!(
            persona_pos < cap_pos,
            "persona should come before capability references"
        );
        assert!(
            cap_pos < ctx_pos,
            "capability references should come before current context"
        );
    }

    #[test]
    fn assemble_deterministic_same_inputs() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let room = RoomName::parse("bedroom").unwrap();
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            None,
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            None,
        );
        assert_eq!(
            a, b,
            "assemble_system_prompt must be deterministic with origin_room"
        );
    }

    #[test]
    fn persona_with_origin_none_equals_persona() {
        assert_eq!(persona_with_origin(None), NILES_SYSTEM_PERSONA);
        let room = RoomName::parse("office").unwrap();
        let out = persona_with_origin(Some(&room));
        assert!(out.contains("# Current context"));
        assert!(out.contains("office"));
    }

    #[test]
    fn build_capability_index_round_trips_metadata() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("prereq");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: prereq\ndescription: Needs deps\nversion: 1.0.0\nprerequisites:\n  - foo\n  - bar\n---\nBody.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);

        let entries = index.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "prereq");
        assert_eq!(entries[0].description, "Needs deps");
        assert_eq!(entries[0].prerequisites, vec!["foo", "bar"]);

        // Indirectly verify the capability is detectable and that
        // prerequisite expansion works when the capability itself matches.
        let names = niles_intent::detect_topics("prereq", &index);
        assert!(names.contains(&"prereq".to_string()));
    }

    #[test]
    fn assemble_no_memory_injection_when_none() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Household context"));
    }

    #[test]
    fn assemble_injects_user_memory() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            None,
            None,
        );
        assert!(out.contains("# User memory"));
        assert!(out.contains("Alice likes tea."));
        assert!(!out.contains("# Agent memory"));
    }

    #[test]
    fn assemble_injects_agent_memory() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            Some("Learned about lighting."),
            None,
        );
        assert!(out.contains("# Agent memory"));
        assert!(out.contains("Learned about lighting."));
        assert!(!out.contains("# User memory"));
    }

    #[test]
    fn assemble_injects_both_memories() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            Some("Learned about lighting."),
            None,
        );
        let user_pos = out.find("# User memory").unwrap();
        let agent_pos = out.find("# Agent memory").unwrap();
        assert!(
            user_pos < agent_pos,
            "user memory should come before agent memory"
        );
    }

    #[test]
    fn assemble_memory_before_capability_references() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            None,
            None,
        );
        let mem_pos = out.find("# User memory").unwrap();
        let cap_pos = out.find("# Capability references").unwrap();
        assert!(
            mem_pos < cap_pos,
            "memory should come before capability references"
        );
    }

    #[test]
    fn assemble_empty_memory_string_is_skipped() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("   "),
            Some(""),
            None,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# Household context"));
    }

    #[test]
    fn assemble_no_skills_section_when_summaries_none() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(!out.contains("# Available skills"));
    }

    #[test]
    fn assemble_no_skills_section_when_summaries_empty() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            Some(&[]),
        );
        assert!(!out.contains("# Available skills"));
    }

    #[test]
    fn assemble_renders_available_skills_after_memory_before_caps() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let summaries = vec![SkillSummary {
            name: "dinner-time".into(),
            description: "Dim lights and start playlist".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::UserCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            None,
            Some(&summaries),
        );
        let user_pos = out.find("# User memory").unwrap();
        let skills_pos = out.find("# Available skills").unwrap();
        let cap_pos = out.find("# Capability references").unwrap();
        assert!(
            user_pos < skills_pos,
            "user memory should come before available skills"
        );
        assert!(
            skills_pos < cap_pos,
            "available skills should come before capability references"
        );
        assert!(out.contains("- dinner-time (v0.1.0) — Dim lights and start playlist"));
    }

    #[test]
    fn assemble_skills_list_preserves_incoming_order() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let summaries = vec![
            SkillSummary {
                name: "zebra".into(),
                description: "Z".into(),
                version: "1.0.0".into(),
                pinned: false,
                provenance: niles_skills::Provenance::UserCreated,
                status: SkillStatus::Active,
                last_activity_at: None,
            },
            SkillSummary {
                name: "alpha".into(),
                description: "A".into(),
                version: "1.0.0".into(),
                pinned: false,
                provenance: niles_skills::Provenance::UserCreated,
                status: SkillStatus::Active,
                last_activity_at: None,
            },
        ];
        let out = assemble_system_prompt(
            "hello",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            Some(&summaries),
        );
        let zebra_pos = out.find("zebra").unwrap();
        let alpha_pos = out.find("alpha").unwrap();
        assert!(
            zebra_pos < alpha_pos,
            "incoming order should be preserved: zebra before alpha"
        );
    }

    #[test]
    fn assemble_is_deterministic_with_skills() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let summaries = vec![SkillSummary {
            name: "skill-a".into(),
            description: "Desc".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::UserCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            Some(&summaries),
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            Some(&summaries),
        );
        assert_eq!(
            a, b,
            "assemble_system_prompt must be deterministic with skills"
        );
    }

    #[test]
    fn assemble_deterministic_same_inputs_with_skills() {
        let tmp = TempDir::new().unwrap();
        let cap_dir = tmp.path().join("lighting");
        fs::create_dir(&cap_dir).unwrap();
        write_skill(
            &cap_dir,
            "---\nname: lighting\ndescription: Control smart lights\nversion: 1.0.0\n---\n# Lighting\n\nTurn on/off lights.\n",
        );
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let room = RoomName::parse("bedroom").unwrap();
        let summaries = vec![SkillSummary {
            name: "skill-a".into(),
            description: "Desc".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::UserCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            Some(&summaries),
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            Some(&room),
            None,
            None,
            Some(&summaries),
        );
        assert_eq!(
            a, b,
            "assemble_system_prompt must be deterministic with origin_room and skills"
        );
    }

    #[test]
    fn assemble_optional_caps_includes_memory_and_skills_without_capabilities() {
        let summaries = vec![SkillSummary {
            name: "dinner-time".into(),
            description: "Dim lights and start playlist".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::UserCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let out = assemble_system_prompt_with_optional_capabilities(
            "turn on the lights",
            &fixture_home(),
            None,
            None,
            None,
            Some("Alice likes tea."),
            Some("Learned about lighting."),
            Some(&summaries),
            &SpeakerContext::Disabled,
        );
        assert!(out.starts_with(NILES_SYSTEM_PERSONA));
        assert!(out.contains("# User memory"));
        assert!(out.contains("# Agent memory"));
        assert!(out.contains("# Available skills"));
        assert!(!out.contains("# Capability references"));
    }

    #[test]
    fn assemble_optional_caps_appends_origin_without_capabilities() {
        let room = RoomName::parse("living_room").unwrap();
        let out = assemble_system_prompt_with_optional_capabilities(
            "hello",
            &fixture_home(),
            None,
            None,
            Some(&room),
            None,
            None,
            None,
            &SpeakerContext::Disabled,
        );
        assert!(out.contains("# Current context"));
        assert!(out.contains("living_room"));
    }

    #[test]
    fn presence_transition_first_resolve_seeds_without_publishing() {
        // First resolve after startup must NOT publish — it only
        // seeds `last`. Otherwise presence automations would re-fire
        // on every service restart.
        let mut last = None;
        assert_eq!(
            presence_transition(&mut last, niles_presence::HomeState::Home,),
            None,
        );
        assert_eq!(last, Some(niles_presence::HomeState::Home));
    }

    #[test]
    fn presence_transition_after_seed_publishes_on_change() {
        // The first genuine transition after the startup seed does publish.
        let mut last = None;
        assert_eq!(
            presence_transition(&mut last, niles_presence::HomeState::Home,),
            None,
        );
        assert_eq!(
            presence_transition(&mut last, niles_presence::HomeState::Away,),
            Some(niles_presence::HomeState::Away),
        );
        assert_eq!(last, Some(niles_presence::HomeState::Away));
    }

    #[test]
    fn presence_transition_same_state_no_publish() {
        let mut last = Some(niles_presence::HomeState::Home);
        assert_eq!(
            presence_transition(&mut last, niles_presence::HomeState::Home,),
            None,
        );
    }

    #[test]
    fn presence_transition_home_to_away_publishes() {
        let mut last = Some(niles_presence::HomeState::Home);
        assert_eq!(
            presence_transition(&mut last, niles_presence::HomeState::Away,),
            Some(niles_presence::HomeState::Away),
        );
        assert_eq!(last, Some(niles_presence::HomeState::Away));
    }

    #[test]
    fn presence_transition_away_to_home_publishes() {
        let mut last = Some(niles_presence::HomeState::Away);
        assert_eq!(
            presence_transition(&mut last, niles_presence::HomeState::Home,),
            Some(niles_presence::HomeState::Home),
        );
        assert_eq!(last, Some(niles_presence::HomeState::Home));
    }

    #[test]
    fn presence_transition_home_to_unknown_publishes() {
        let mut last = Some(niles_presence::HomeState::Home);
        assert_eq!(
            presence_transition(&mut last, niles_presence::HomeState::Unknown,),
            Some(niles_presence::HomeState::Unknown),
        );
        assert_eq!(last, Some(niles_presence::HomeState::Unknown));
    }

    #[test]
    fn system_prompt_renders_stale_annotation() {
        let fixed_now = chrono::Utc::now();
        let summaries = vec![SkillSummary {
            name: "stale-skill".into(),
            description: "Does something".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::AgentCreated,
            status: SkillStatus::Stale,
            last_activity_at: Some(fixed_now - chrono::Duration::days(35)),
        }];
        let mut out = String::new();
        render_skills_section(&mut out, &summaries);
        assert!(out.contains("[stale: 35 days unused]"));
    }

    #[test]
    fn system_prompt_omits_annotation_for_active() {
        let summaries = vec![SkillSummary {
            name: "active-skill".into(),
            description: "Does something".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::AgentCreated,
            status: SkillStatus::Active,
            last_activity_at: None,
        }];
        let mut out = String::new();
        render_skills_section(&mut out, &summaries);
        assert!(!out.contains("[stale:"));
        assert!(out.contains("- active-skill (v0.1.0) — Does something"));
    }

    #[test]
    fn system_prompt_renders_stale_without_days_when_no_activity() {
        let summaries = vec![SkillSummary {
            name: "stale-skill".into(),
            description: "Does something".into(),
            version: "0.1.0".into(),
            pinned: false,
            provenance: niles_skills::Provenance::AgentCreated,
            status: SkillStatus::Stale,
            last_activity_at: None,
        }];
        let mut out = String::new();
        render_skills_section(&mut out, &summaries);
        assert!(out.contains("[stale]"));
        assert!(!out.contains("days unused"));
    }

    #[test]
    fn spawn_skill_curator_disabled_returns_none() {
        let cfg = niles_config::SkillsCuratorConfig {
            enabled: false,
            ..Default::default()
        };
        // No tokio runtime needed — the enabled branch returns None
        // before any spawn.
        assert!(
            spawn_skill_curator(Arc::new(niles_skills::SkillStore::new(100_000)), cfg,).is_none()
        );
    }

    #[test]
    fn a_lone_overheard_word_is_not_an_instruction() {
        // A real turn: the satellite woke, the television said
        // "Dance.", and the LLM set a light effect on the living room
        // ceiling. The house changed because a programme said a word.
        assert!(is_overheard("Dance."));
        assert!(is_overheard("so"));
        assert!(is_overheard("  right  "));
    }

    #[test]
    fn a_question_is_always_worth_answering() {
        // "Weather?" is a real thing to say to a house.
        assert!(!is_overheard("Weather?"));
        assert!(!is_overheard("Why?"));
    }

    #[test]
    fn anything_with_two_words_still_reaches_the_llm() {
        // The guard is for what a room emits, not for terse people.
        assert!(!is_overheard("lights off"));
        assert!(!is_overheard("play music"));
    }

    #[test]
    fn the_persona_warns_that_it_is_listening_to_a_room() {
        assert!(
            NILES_SYSTEM_PERSONA.contains("listening to a room"),
            "the model must know that what it hears may not be addressed to it"
        );
    }

    #[test]
    fn the_persona_says_the_reply_is_spoken() {
        // A measured turn spent 6.4 s speaking sixty words of weather
        // with two decimal places. "Be concise" was already there and
        // was not enough — the model had no idea anyone was listening
        // rather than reading.
        assert!(
            NILES_SYSTEM_PERSONA.contains("spoken aloud"),
            "the model has to know it is being listened to"
        );
    }

    #[test]
    fn the_prompt_carries_the_clock() {
        // Without it the model spends a tool call and a second round
        // trip on anything with an implicit "now" in it.
        let section = now_context("Europe/Copenhagen");
        assert!(section.contains("# Now"), "{section}");
        assert!(section.contains("Europe/Copenhagen"), "{section}");
    }

    #[test]
    fn an_unparseable_timezone_adds_nothing() {
        // A wrong time is worse than no time: the model would answer
        // confidently with it.
        assert!(now_context("Not/AZone").is_empty());
    }

    #[test]
    fn home_context_includes_all_fields() {
        let home = fixture_home();
        let out = home_context(&home);
        assert!(out.contains("Home: Hjemmet"), "expected home name: {out}");
        assert!(out.contains("Country: DK"), "expected country DK: {out}");
        assert!(
            out.contains("Locale: da_DK"),
            "expected locale da_DK: {out}"
        );
        assert!(
            out.contains("Timezone: Europe/Copenhagen"),
            "expected timezone: {out}"
        );
        assert!(
            out.contains("Units: metric (°C, km)"),
            "expected metric units: {out}"
        );
        assert!(
            out.contains("Spoken language: da"),
            "expected language da: {out}"
        );
    }

    #[test]
    fn home_context_imperial_for_en_us() {
        let mut home = fixture_home();
        home.locale = "en_US".into();
        let out = home_context(&home);
        assert!(
            out.contains("Units: imperial (°F, miles)"),
            "expected imperial units: {out}"
        );
        assert!(out.contains("Country: US"), "expected country US: {out}");
        assert!(
            out.contains("Locale: en_US"),
            "expected locale en_US: {out}"
        );
    }

    #[test]
    fn home_context_unknown_country_when_bare_locale() {
        let mut home = fixture_home();
        home.locale = "en".into();
        home.country = None;
        let out = home_context(&home);
        assert!(out.contains("unknown"), "expected unknown country: {out}");
    }

    #[test]
    fn home_context_sanitizes_control_characters() {
        let mut home = fixture_home();
        home.name = "Hjemmet\n# injected".into();
        home.locale = "da_DK\r\nignore".into();
        home.timezone = "Europe/Copenhagen\t# hidden".into();
        let out = home_context(&home);
        assert!(
            out.contains("Home: Hjemmet # injected"),
            "expected sanitized home field: {out}"
        );
        assert!(
            out.contains("Locale: da_DK ignore"),
            "expected sanitized locale field: {out}"
        );
        assert!(
            out.contains("Timezone: Europe/Copenhagen # hidden"),
            "expected sanitized timezone field: {out}"
        );
    }

    #[test]
    fn system_prompt_includes_household_context_after_persona_before_memory() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            Some("Alice likes tea."),
            None,
            None,
        );
        let persona_pos = out.find(NILES_SYSTEM_PERSONA).unwrap();
        let home_idx = out.find("# Household context").unwrap();
        let memory_idx = out.find("# User memory").unwrap();
        assert!(
            persona_pos < home_idx,
            "persona should come before household context"
        );
        assert!(
            home_idx < memory_idx,
            "household context should come before memory"
        );
    }

    #[test]
    fn assemble_system_prompt_is_deterministic_with_home() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let a = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        let b = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            a, b,
            "assemble_system_prompt must be deterministic with home"
        );
    }

    #[test]
    fn speaker_prompt_injection_identified() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt_with_optional_capabilities(
            "turn on the lights",
            &fixture_home(),
            Some(&index),
            Some(&loader),
            None,
            None,
            None,
            None,
            &SpeakerContext::Identified {
                name: "Mark".into(),
                known: profile::Known::default(),
            },
        );
        assert!(
            out.contains("# Speaker"),
            "expected # Speaker section: {out}"
        );
        assert!(out.contains("Mark"), "expected speaker name Mark: {out}");
    }

    #[test]
    fn speaker_prompt_injection_unknown() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out = assemble_system_prompt_with_optional_capabilities(
            "turn on the lights",
            &fixture_home(),
            Some(&index),
            Some(&loader),
            None,
            None,
            None,
            None,
            &SpeakerContext::Unknown,
        );
        assert!(
            out.contains("# Speaker"),
            "expected # Speaker section: {out}"
        );
        assert!(
            out.contains("not recognized"),
            "expected 'not recognized': {out}"
        );
    }

    #[test]
    fn speaker_prompt_disabled_no_injection() {
        let tmp = TempDir::new().unwrap();
        let loader = CapabilityLoader::load_from_dir(tmp.path()).unwrap();
        let index = build_capability_index(&loader);
        let out_direct = assemble_system_prompt_with_optional_capabilities(
            "turn on the lights",
            &fixture_home(),
            Some(&index),
            Some(&loader),
            None,
            None,
            None,
            None,
            &SpeakerContext::Disabled,
        );
        let out_helper = assemble_system_prompt(
            "turn on the lights",
            &fixture_home(),
            &index,
            &loader,
            None,
            None,
            None,
            None,
        );
        assert!(
            !out_direct.contains("# Speaker"),
            "expected no # Speaker when disabled: {out_direct}"
        );
        assert_eq!(
            out_direct, out_helper,
            "Disabled path must match the test helper"
        );
    }

    struct MockIdentifier {
        result: Option<(String, f32)>,
        /// Names already taken by a different voice.
        taken: Vec<String>,
        enrolled: std::sync::Mutex<Vec<(String, usize)>>,
    }

    impl MockIdentifier {
        fn with(result: Option<(String, f32)>) -> Self {
            Self {
                result,
                taken: Vec::new(),
                enrolled: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl SpeakerIdentifier for MockIdentifier {
        fn embed(&self, _pcm: &[i16], _sample_rate_hz: u32) -> Option<Vec<f32>> {
            Some(vec![0.0; 192])
        }

        fn classify(&self, _embedding: &[f32]) -> Option<(String, f32)> {
            self.result.clone()
        }

        async fn enroll(&self, name: &str, _embedding: &[f32]) -> anyhow::Result<usize> {
            let mut enrolled = self.enrolled.lock().unwrap();
            let clips = enrolled.iter().filter(|(n, _)| n == name).count() + 1;
            enrolled.push((name.to_string(), clips));
            Ok(clips)
        }

        async fn is_someone_else(&self, name: &str, _embedding: &[f32]) -> bool {
            self.taken.iter().any(|n| n == name)
        }

        fn how_to_say(&self, speaker: &str) -> Option<String> {
            Some(speaker.to_string())
        }

        fn address_as(&self, _speaker: &str) -> Option<String> {
            None
        }

        fn profile(&self, _speaker: &str) -> (Option<String>, Option<String>) {
            (None, None)
        }

        fn whose_voice(&self, _embedding: &[f32]) -> Option<String> {
            self.result.as_ref().map(|(name, _)| name.to_lowercase())
        }

        fn knows_anybody(&self) -> bool {
            !self.enrolled.lock().unwrap().is_empty() || !self.taken.is_empty()
        }
    }

    #[tokio::test]
    async fn introducing_yourself_enrols_the_sentence_that_said_it() {
        let mock = Arc::new(MockIdentifier::with(None));
        let clips = mock.enroll("mark", &[0.0; 192]).await.unwrap();
        assert_eq!(clips, 1);
        assert!(
            crate::response::enrolled("mark", clips).contains("Mark"),
            "the reply should use the name"
        );
    }

    #[tokio::test]
    async fn saying_it_again_adds_a_clip() {
        // One sample is a thin voice print. Repeating the phrase is the
        // fix, so it has to accumulate rather than replace.
        let mock = MockIdentifier::with(None);
        assert_eq!(mock.enroll("mark", &[0.0; 192]).await.unwrap(), 1);
        assert_eq!(mock.enroll("mark", &[0.0; 192]).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn a_stranger_cannot_take_an_enrolled_name() {
        // Otherwise a guest saying "I am Mark" becomes Mark, and every
        // later sentence of theirs is attributed to him.
        let mock = MockIdentifier {
            result: None,
            taken: vec!["mark".into()],
            enrolled: std::sync::Mutex::new(Vec::new()),
        };
        assert!(mock.is_someone_else("mark", &[0.0; 192]).await);
        assert!(!mock.is_someone_else("guest", &[0.0; 192]).await);
    }

    #[test]
    fn the_enrolment_reply_asks_for_more_until_it_has_enough() {
        assert!(crate::response::enrolled("mark", 1).contains("once or twice more"));
        assert!(crate::response::enrolled("mark", 2).contains("One more"));
        assert!(!crate::response::enrolled("mark", 3).contains("more"));
    }

    #[test]
    fn speaker_context_from_attempted_match() {
        let mock = MockIdentifier::with(Some(("Mark".into(), 0.95)));
        let ctx = speaker_context_from(true, mock.classify(&[0.0; 192]), profile::Known::default());
        assert!(matches!(ctx, SpeakerContext::Identified { ref name, .. } if name == "Mark"),);
    }

    #[test]
    fn speaker_context_from_attempted_unknown() {
        let mock = MockIdentifier::with(None);
        let ctx = speaker_context_from(true, mock.classify(&[0.0; 192]), profile::Known::default());
        assert!(
            matches!(ctx, SpeakerContext::Unknown),
            "expected Unknown, got {ctx:?}"
        );
    }

    #[test]
    fn speaker_context_from_not_attempted_disabled() {
        let mock = MockIdentifier::with(Some(("Mark".into(), 0.95)));
        let ctx =
            speaker_context_from(false, mock.classify(&[0.0; 192]), profile::Known::default());
        assert!(
            matches!(ctx, SpeakerContext::Disabled),
            "expected Disabled when attempted=false, got {ctx:?}"
        );
    }

    #[test]
    fn speaker_identification_accepts_16khz_mono_16bit() {
        let format = niles_wyoming::AudioFormat::new(16_000, 16, 1);
        assert!(supports_speaker_identification(format));
    }

    #[test]
    fn speaker_identification_rejects_stereo_pcm() {
        let format = niles_wyoming::AudioFormat::new(16_000, 16, 2);
        assert!(
            !supports_speaker_identification(format),
            "speaker embeddings expect mono PCM, not interleaved stereo"
        );
    }

    #[test]
    fn speaker_identification_rejects_wrong_rate_or_width() {
        assert!(!supports_speaker_identification(
            niles_wyoming::AudioFormat::new(8_000, 16, 1)
        ));
        assert!(!supports_speaker_identification(
            niles_wyoming::AudioFormat::new(16_000, 24, 1)
        ));
    }
}
