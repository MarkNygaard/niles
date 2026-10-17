//! Telling what was heard: noise, hallucinated scripts, the wake word, and being addressed.

use niles_intent::{Intent, IntentRouter};
use niles_scheduler::TimerStore;

/// Parse a transcript and act on any Tier 0 intent it produces.
/// True if `text` has no actual words — it's pure punctuation / symbols
/// / music notes (e.g. "...", "♪") that Whisper sometimes emits for
/// non-speech. Anything containing at least one letter is kept, including
/// social phrases like "thank you" (which now earn a real reply).
///
/// We previously denylisted specific hallucinations ("you", "thank you",
/// "thanks for watching", …) because an always-on mic fed silence and the
/// mute-button click pop to Whisper, which turned them into those phrases.
/// The satellite-side VAD now keeps silence and the brief click transient
/// from ever reaching STT, so the phrase denylist is gone — only this
/// letterless guard remains.
pub(crate) fn is_noise_transcript(text: &str) -> bool {
    !text.chars().any(|c| c.is_alphabetic())
}

/// True if `c` belongs to a CJK script (Han, Hiragana, Katakana, Hangul, or
/// CJK symbols/punctuation) — the scripts Whisper hallucinates into when fed
/// near-silent audio.
pub(crate) fn is_cjk_char(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF   // Hangul Jamo
        | 0x3000..=0x303F // CJK symbols and punctuation
        | 0x3040..=0x30FF // Hiragana + Katakana
        | 0x3130..=0x318F // Hangul compatibility Jamo
        | 0x3400..=0x4DBF // CJK Unified Ideographs Ext A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
    )
}

/// True when `text` is predominantly CJK while the household language (`lang`,
/// resolved two-letter code) is not CJK. Whisper invents CJK stock phrases
/// (Korean/Japanese/Chinese news intros, etc.) when given quiet/noisy audio;
/// in a non-CJK household those are always hallucinations, never real input.
/// Skipped entirely if the household actually speaks a CJK language.
pub(crate) fn is_foreign_script_hallucination(text: &str, lang: &str) -> bool {
    if matches!(lang, "ja" | "ko" | "zh") {
        return false;
    }
    let mut cjk = 0usize;
    let mut latin = 0usize;
    for c in text.chars() {
        if is_cjk_char(c) {
            cjk += 1;
        } else if c.is_alphabetic() {
            latin += 1;
        }
    }
    cjk > 0 && cjk >= latin
}

/// Whether this is "stop" said to a ringing alarm.
///
/// That gets past both the noise gate and the voice lock. It is one short
/// word — the least Whisper can be sure of and the least a voice print can
/// be matched on — and the one command a house must never lose. A
/// television that stops somebody's timer has done less harm than an
/// alarm nobody can silence; with nothing ringing the gates apply as
/// ever, so "stop" from the television still gets silence rather than
/// "nothing's running".
pub(crate) fn is_stopping_an_alarm(text: &str, timers: &TimerStore) -> bool {
    let router = IntentRouter::new();
    let parsed = router
        .parse(text)
        .or_else(|| without_vocative(text).and_then(|rest| router.parse(rest)));
    matches!(parsed, Some(Intent::Stop | Intent::Cancel))
        && timers.list().iter().any(|t| t.is_ringing())
}

/// The sentence after a first word that addressed somebody.
///
/// Whisper hears the wake word the satellite sends in front of every
/// command, and spells it however it likes: Myles, Miles, Charles, Lars,
/// Nines, Nodels — all from one afternoon. [`strip_wake_word`] knows the
/// spellings it can be sure of; this is for the rest. A single word set
/// off by a comma, a full stop or an exclamation at the start of a
/// command is a name being called.
///
/// Only ever a second attempt at Tier 0, after the sentence as heard has
/// failed: if what is left is not a command either, nothing changes, so a
/// sentence that merely starts with "Well," is never cut short on its
/// way to the LLM.
pub(crate) fn without_vocative(text: &str) -> Option<&str> {
    let t = text.trim_start();
    // A full stop too: "Nice. Turn off the light." is how Whisper wrote
    // a misheard name, and the rest is only kept if it is a command.
    let end = t.find([',', '!', '?', '.'])?;
    let word = &t[..end];
    if word.is_empty()
        || word.len() > 12
        || !word
            .chars()
            .all(|c| c.is_alphabetic() || c == '\'' || c == '’')
    {
        return None;
    }
    let rest = t[end + 1..].trim_start();
    (!rest.is_empty()).then_some(rest)
}

/// The command, without the name that summoned it.
///
/// The satellite sends the second before its wake word as well as what
/// follows, so Whisper hears "Niles, stop" rather than "stop". Every
/// Tier 0 pattern is anchored at both ends, and a name in front of the
/// command is enough to miss all of them — which sends "stop" to the
/// LLM, and a ringing timer on ringing.
///
/// Whisper spells the name several ways. The name itself and its
/// wake-word spelling go whatever follows; the look-alikes only with a
/// comma, because "Miles to kilometres" is a question.
pub(crate) const NAMES: [&str; 3] = ["niles", "nyles", "nyle"];
pub(crate) const LOOK_ALIKES: [&str; 6] = ["miles", "myles", "nile", "nile's", "neil", "niels"];

pub(crate) fn strip_wake_word(text: &str) -> &str {
    let t = text.trim_start();
    let word_end = t
        .find(|c: char| !(c.is_alphanumeric() || c == '\''))
        .unwrap_or(t.len());
    let (word, rest) = t.split_at(word_end);
    let word = word.to_lowercase();
    let comma = rest.starts_with(',');
    if NAMES.contains(&word.as_str()) || (comma && LOOK_ALIKES.contains(&word.as_str())) {
        rest.trim_start_matches(|c: char| c == ',' || c == '.' || c == '!' || c.is_whitespace())
    } else {
        text
    }
}

/// A short confirmation with the speaker's form of address at the end.
///
/// "Living room lights off." becomes "Living room lights off Sir." —
/// the address goes before the closing mark, and a reply of several
/// sentences gets it on the last one only.
///
/// No comma, though a comma is what the grammar wants: this text is only
/// ever spoken, and Piper pauses at every comma, which made ", Sir" sound
/// bolted on. Run on, it is said the way a person says it.
pub(crate) fn addressed(reply: &str, address: Option<&str>) -> String {
    let Some(address) = address.map(str::trim).filter(|a| !a.is_empty()) else {
        return reply.to_string();
    };
    let trimmed = reply.trim_end();
    match trimmed.chars().last() {
        Some(mark @ ('.' | '!' | '?')) => {
            format!(
                "{} {address}{mark}",
                &trimmed[..trimmed.len() - mark.len_utf8()]
            )
        }
        Some(_) => format!("{trimmed} {address}."),
        None => reply.to_string(),
    }
}

/// Whether the transcript begins by calling Niles.
///
/// The satellite sends the wake word with the command, so a real wake
/// starts with the name — or with the handful of ways a transcriber
/// writes it. The television that set the satellite off does not.
pub(crate) fn heard_its_name(text: &str) -> bool {
    let first: String = text
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphabetic() || *c == '\'')
        .collect::<String>()
        .to_lowercase();
    NAMES.contains(&first.as_str()) || LOOK_ALIKES.contains(&first.as_str())
}

/// Whether this is more likely something the room said than something
/// said to Niles.
///
/// The satellite listens to a room, so a wake word is an invitation to
/// transcribe whatever follows — including the television. One such
/// turn transcribed "Dance." and the LLM, given a bare noun and an
/// instruction to act rather than describe, picked a light and set an
/// effect on it. The house changed because a programme said a word.
///
/// A lone word that Tier 0 did not recognise is far more likely to be
/// overheard than addressed. Being wrong costs a "sorry?" and a repeat;
/// being right avoids acting on the television. A question is exempt —
/// "weather?" is a real thing to say — and Tier 0 has already had its
/// turn, so "stop" and the rest still work.
pub(crate) fn is_overheard(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.ends_with('?') {
        return false;
    }
    trimmed
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
        <= 1
}

#[cfg(test)]
mod noise_transcript_tests {
    use super::*;
    use crate::profile;
    use crate::prompt::{HEARD_NOT_READ, SpeakerContext, now_playing_section, speaker_context};
    use crate::wiring::presence::retry_after;
    use chrono::Utc;
    use std::net::SocketAddr;
    use std::time::Duration;

    #[test]
    fn what_plays_is_listed_and_silence_adds_nothing() {
        assert_eq!(now_playing_section(&[]), "");
        let section = now_playing_section(&["living room: playing Chariot by Gavin DeGraw".into()]);
        assert!(section.contains("# Playing now"));
        assert!(section.contains("- living room: playing Chariot by Gavin DeGraw"));
    }

    #[test]
    fn a_spoken_turn_is_told_it_may_have_been_misheard() {
        assert!(HEARD_NOT_READ.contains("mishears"));
        assert!(HEARD_NOT_READ.contains("does not"));
        assert!(!HEARD_NOT_READ.contains("  "), "a line continuation lost");
    }

    #[test]
    fn the_name_in_front_of_a_command_is_not_part_of_it() {
        assert_eq!(strip_wake_word("Niles, stop."), "stop.");
        assert_eq!(strip_wake_word("Niles stop"), "stop");
        assert_eq!(
            strip_wake_word("Nyles, what time is it?"),
            "what time is it?"
        );
        assert_eq!(
            strip_wake_word("Miles, turn off the lights."),
            "turn off the lights."
        );
    }

    #[test]
    fn a_name_that_is_the_sentence_is_left_alone() {
        // "Miles to kilometres" is a question, not a wake word.
        assert_eq!(
            strip_wake_word("Miles to kilometres?"),
            "Miles to kilometres?"
        );
        assert_eq!(strip_wake_word("turn on the lights"), "turn on the lights");
        assert_eq!(strip_wake_word("Nilesh is here"), "Nilesh is here");
    }

    #[test]
    fn stop_gets_through_while_an_alarm_rings() {
        let timers = TimerStore::new();
        let origin: SocketAddr = "10.0.0.5:1234".parse().unwrap();
        let id = timers.set(std::time::Duration::from_secs(60), None, origin, Utc::now());
        assert!(
            !is_stopping_an_alarm("stop", &timers),
            "nothing ringing yet"
        );
        timers.mark_ringing(id);
        assert!(is_stopping_an_alarm("stop", &timers));
        assert!(is_stopping_an_alarm(
            strip_wake_word("Niles, stop."),
            &timers
        ));
        assert!(is_stopping_an_alarm("stop the alarm", &timers));
        assert!(
            !is_stopping_an_alarm("turn off the lights", &timers),
            "only stopping gets past the gates"
        );
    }

    #[test]
    fn any_spelling_of_the_name_gives_way_to_the_command() {
        let router = IntentRouter::new();
        for heard in [
            "Myles, turn off the kitchen lights.",
            "Charles, what time is it?",
            "Nodels! What time is it?",
            "Nice. Turn off the kitchen lights.",
        ] {
            let rest = without_vocative(heard).expect(heard);
            assert!(
                router.parse(rest).is_some(),
                "{heard:?} -> {rest:?} should be a command"
            );
        }
    }

    #[test]
    fn only_one_word_is_ever_a_name() {
        assert_eq!(without_vocative("Well then, turn it off"), None);
        assert_eq!(without_vocative("turn off the lights"), None);
        assert_eq!(without_vocative("Myles,"), None);
    }

    #[test]
    fn a_misheard_name_still_stops_an_alarm() {
        let timers = TimerStore::new();
        let origin: SocketAddr = "10.0.0.5:1234".parse().unwrap();
        let id = timers.set(std::time::Duration::from_secs(60), None, origin, Utc::now());
        timers.mark_ringing(id);
        assert!(is_stopping_an_alarm("Myles, stop.", &timers));
    }

    #[test]
    fn a_refused_login_is_not_asked_again_for_half_an_hour() {
        let five = Duration::from_secs(300);
        assert_eq!(retry_after(true, five), Duration::from_secs(1800));
        assert_eq!(
            retry_after(false, five),
            five,
            "a passing failure keeps the pace"
        );
        assert_eq!(
            retry_after(true, Duration::from_secs(3600)),
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn a_confirmation_ends_with_the_form_of_address() {
        assert_eq!(
            addressed("Living room lights off.", Some("Sir")),
            "Living room lights off Sir."
        );
        assert_eq!(addressed("Which room?", Some("Sir")), "Which room Sir?");
        assert_eq!(
            addressed("12 degrees now. Expect rain.", Some("Sir")),
            "12 degrees now. Expect rain Sir."
        );
        assert_eq!(addressed("Done", Some("Miss")), "Done Miss.");
    }

    #[test]
    fn nobody_addressed_is_left_as_it_was() {
        assert_eq!(addressed("Lights off.", None), "Lights off.");
        assert_eq!(addressed("Lights off.", Some("  ")), "Lights off.");
    }

    #[test]
    fn the_language_model_is_told_how_to_address_them() {
        let prompt = speaker_context(
            &SpeakerContext::Identified {
                name: "Mark".into(),
                known: profile::Known {
                    address: Some("Sir".into()),
                    ..Default::default()
                },
            },
            None,
        )
        .unwrap();
        assert!(prompt.contains("Address them as \"Sir\""), "{prompt}");
        assert!(prompt.contains("Their name is still Mark"), "{prompt}");
    }

    #[test]
    fn the_language_model_is_shown_their_notes_and_birthday() {
        let prompt = speaker_context(
            &SpeakerContext::Identified {
                name: "Mark".into(),
                known: profile::Known {
                    notes: Some("- Prefers tea to coffee".into()),
                    birthday: Some("10-03".into()),
                    ..Default::default()
                },
            },
            chrono::NaiveDate::from_ymd_opt(2026, 10, 3),
        )
        .unwrap();
        assert!(prompt.contains("Prefers tea to coffee"), "{prompt}");
        assert!(prompt.contains("Their birthday is 3 October"), "{prompt}");
        assert!(prompt.contains("Today is their birthday"), "{prompt}");
    }

    #[test]
    fn nothing_known_adds_nothing() {
        let prompt = speaker_context(
            &SpeakerContext::Identified {
                name: "Mark".into(),
                known: profile::Known::default(),
            },
            None,
        )
        .unwrap();
        assert!(!prompt.contains("birthday"), "{prompt}");
        assert!(!prompt.contains("What you know"), "{prompt}");
    }

    #[test]
    fn a_command_that_calls_niles_is_addressed() {
        for heard in [
            "Niles, turn off the light.",
            "Myles, what time is it?",
            "nyles stop",
            "Niles.",
        ] {
            assert!(heard_its_name(heard), "{heard:?}");
        }
    }

    #[test]
    fn the_television_is_not_addressed() {
        for heard in [
            "What's up y'all man, today we're checking out this Danish comedian",
            "Yeah, I mean, I think it's a good idea.",
            "Thank you.",
            "",
            "Nilesh is here",
        ] {
            assert!(!heard_its_name(heard), "{heard:?}");
        }
    }

    #[test]
    fn the_name_alone_leaves_nothing_to_do() {
        assert_eq!(strip_wake_word("Niles."), "");
        assert_eq!(strip_wake_word("Niles!"), "");
    }

    #[test]
    fn drops_wordless_output() {
        assert!(is_noise_transcript(""));
        assert!(is_noise_transcript("..."));
        assert!(is_noise_transcript("   "));
        assert!(is_noise_transcript("♪"));
        assert!(is_noise_transcript("!?"));
        assert!(is_noise_transcript("123 - 456"));
    }

    #[test]
    fn keeps_social_phrases_so_niles_can_be_thanked() {
        // The satellite VAD now keeps the silence/click noise that used to
        // produce these out of STT, so they're real words again — no longer
        // denylisted. Saying "thank you" should earn a "you're welcome".
        assert!(!is_noise_transcript("you"));
        assert!(!is_noise_transcript("Thank you."));
        assert!(!is_noise_transcript("Thanks for watching!"));
        assert!(!is_noise_transcript("Bye."));
    }

    #[test]
    fn keeps_real_commands() {
        assert!(!is_noise_transcript("turn on the office light"));
        assert!(!is_noise_transcript("what's the weather tomorrow"));
        assert!(!is_noise_transcript("stop"));
    }

    #[test]
    fn drops_cjk_hallucinations_in_non_cjk_household() {
        // The exact phrase Whisper invented on the satellite's quiet audio.
        assert!(is_foreign_script_hallucination(
            "MBC 뉴스 김성현입니다.",
            "en"
        ));
        // Other common Whisper CJK hallucinations.
        assert!(is_foreign_script_hallucination(
            "ご視聴ありがとうございました",
            "en"
        ));
        assert!(is_foreign_script_hallucination("请不吝点赞", "da"));
        // A lone ideograph is alphabetic (so is_noise_transcript keeps it) but
        // is still a hallucination here.
        assert!(is_foreign_script_hallucination("中", "en"));
    }

    #[test]
    fn keeps_real_commands_in_non_cjk_household() {
        assert!(!is_foreign_script_hallucination(
            "turn on the office light",
            "en"
        ));
        assert!(!is_foreign_script_hallucination(
            "sluk lyset i kontoret",
            "da"
        ));
        assert!(!is_foreign_script_hallucination("stop", "en"));
        assert!(!is_foreign_script_hallucination("", "en"));
    }

    #[test]
    fn keeps_cjk_when_household_speaks_cjk() {
        // A Korean-speaking household must still be understood.
        assert!(!is_foreign_script_hallucination("불 꺼", "ko"));
        assert!(!is_foreign_script_hallucination(
            "MBC 뉴스 김성현입니다.",
            "ja"
        ));
    }
}
