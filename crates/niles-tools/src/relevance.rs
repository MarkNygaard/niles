//! Which tools are worth sending with a request.
//!
//! Every tool's schema goes on the wire on every LLM call, whether or
//! not the request could possibly use it. With thirty-odd registered
//! that is most of the prompt — measured against Groq, a single call
//! was 3 497 tokens of an 8 000-per-minute budget, so one question
//! that needed a tool round trip consumed most of a minute.
//!
//! # Why groups rather than per-tool keywords
//!
//! A tool nobody thought to classify still has to work. Tools are
//! grouped by the subject they serve, and **anything not in a group is
//! always sent** — so adding a tool without touching this file leaves
//! it behaving exactly as it did before. The failure mode of
//! forgetting is "no saving", not "silently missing".
//!
//! The same reasoning caps the ambition: when nothing matches, every
//! tool goes. A request we can't classify is exactly the one where
//! guessing would hurt.

/// Tools sent with every request, whatever it says.
///
/// The ones a sentence can need without announcing it: acting on a
/// device, reading its state, knowing what exists, and the memory the
/// persona is told it has.
const ALWAYS: &[&str] = &[
    "current_datetime",
    "get_device_state",
    "list_all_devices",
    "list_devices_in_room",
    "look_up_capability",
    "memory",
    "set_device",
];

/// A subject, the words that signal it, and the tools it needs.
struct Group {
    keywords: &'static [&'static str],
    tools: &'static [&'static str],
}

const GROUPS: &[Group] = &[
    Group {
        keywords: &[
            "timer",
            "timers",
            "alarm",
            "alarms",
            "countdown",
            "remind",
            "reminder",
        ],
        tools: &["cancel_timer", "get_timer_remaining", "list_timers"],
    },
    Group {
        keywords: &[
            "weather",
            "forecast",
            "rain",
            "raining",
            "snow",
            "snowing",
            "sunny",
            "wind",
            "windy",
            "temperature",
            "degrees",
            "umbrella",
            "outside",
        ],
        tools: &["get_weather"],
    },
    Group {
        keywords: &[
            "task", "tasks", "todo", "to-do", "issue", "issues", "ticket", "linear",
        ],
        tools: &["create_task", "get_task", "list_tasks"],
    },
    Group {
        keywords: &[
            "skill",
            "skills",
            "routine",
            "remember how",
            "save that",
            "forget how",
        ],
        tools: &["delete_skill", "mint_skill", "patch_skill", "view_skill"],
    },
    Group {
        // Anything about the past. "why" earns its place here: "why is
        // the hall light on" is only answerable from history.
        keywords: &[
            "why",
            "earlier",
            "yesterday",
            "last night",
            "this morning",
            "history",
            "before",
            "when did",
            "who turned",
            "recently",
            "since",
        ],
        tools: &[
            "device_state_snapshot_at",
            "explain_device_state",
            "query_command_history",
            "query_device_state_history",
        ],
    },
    Group {
        keywords: &[
            "home", "away", "anyone", "anybody", "everyone", "presence", "back yet", "who is",
            "who's",
        ],
        tools: &["get_presence", "set_presence"],
    },
    Group {
        keywords: &[
            "announce",
            "tell everyone",
            "broadcast",
            "notification",
            "notifications",
        ],
        tools: &["announce", "list_recent_notifications"],
    },
    Group {
        keywords: &[
            "config",
            "setting",
            "settings",
            "configure",
            "curve",
            "sunset",
            "sunrise",
            "ambient",
            "brightness curve",
            "undo",
        ],
        tools: &[
            "get_config",
            "reset_config",
            "undo_config_change",
            "update_config",
        ],
    },
    Group {
        keywords: &["effect", "effects", "rainbow", "fireplace", "party"],
        tools: &["set_light_effect"],
    },
    Group {
        keywords: &[
            "search", "look up", "google", "online", "news", "who is", "what is",
        ],
        tools: &["web_search"],
    },
];

/// Whether this tool belongs to a subject at all.
///
/// A tool nobody classified is always sent, so forgetting to add one
/// here costs a saving rather than a capability.
pub fn is_grouped(name: &str) -> bool {
    GROUPS.iter().any(|g| g.tools.contains(&name))
}

/// Names of the tools worth sending for `transcript`.
///
/// Returns `None` when nothing matched, meaning "send everything" —
/// the caller shouldn't have to encode that policy twice.
pub fn relevant_tool_names(transcript: &str) -> Option<Vec<&'static str>> {
    let text = transcript.to_lowercase();
    let mut names: Vec<&'static str> = Vec::new();
    let mut matched = false;

    for group in GROUPS {
        if group.keywords.iter().any(|k| contains_word(&text, k)) {
            matched = true;
            names.extend_from_slice(group.tools);
        }
    }
    if !matched {
        return None;
    }
    names.extend_from_slice(ALWAYS);
    names.sort_unstable();
    names.dedup();
    Some(names)
}

/// Whether `needle` appears in `text` as a whole word (or phrase).
///
/// Substring matching would put the weather tools on the wire for
/// "turn on the wind chime", and "is" would match half the language.
fn contains_word(text: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(found) = text[from..].find(needle) {
        let start = from + found;
        let end = start + needle.len();
        let before_ok = start == 0
            || !text[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric());
        let after_ok = end == text.len()
            || !text[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unclassifiable_request_gets_everything() {
        // The one case where guessing would hurt most.
        assert!(relevant_tool_names("do the thing with the stuff").is_none());
    }

    #[test]
    fn a_subject_brings_its_own_tools_and_the_core() {
        let names = relevant_tool_names("what's the weather like").expect("matched");
        assert!(names.contains(&"get_weather"));
        assert!(names.contains(&"set_device"), "core is always there");
        assert!(
            !names.contains(&"list_timers"),
            "an unrelated subject stays off the wire"
        );
    }

    #[test]
    fn the_past_tense_reaches_the_history_tools() {
        let names = relevant_tool_names("why is the hall light on").expect("matched");
        assert!(names.contains(&"explain_device_state"));
        assert!(names.contains(&"query_device_state_history"));
    }

    #[test]
    fn two_subjects_bring_both() {
        let names = relevant_tool_names("set a timer and tell me the weather").expect("matched");
        assert!(names.contains(&"list_timers"));
        assert!(names.contains(&"get_weather"));
    }

    #[test]
    fn a_keyword_inside_another_word_does_not_count() {
        // "wind chime" must not summon the forecast.
        assert!(!contains_word("turn on the winder", "wind"));
        assert!(contains_word("is it windy", "windy"));
    }

    #[test]
    fn the_core_is_never_empty() {
        let names = relevant_tool_names("set a timer for five minutes").expect("matched");
        for core in ALWAYS {
            assert!(names.contains(core), "{core} should always be sent");
        }
    }
}
