//! What Niles knows about each person, and the tools that let it learn.
//!
//! One page of notes per voice — their own USER.md — and a birthday. Kept
//! on the enrolled voice, beside the name it says and the way it addresses
//! them, so the same identity carries all of it: the voice Niles hears,
//! and the person signed in to the app.
//!
//! The tools write only to whoever is speaking. Which voice that is comes
//! from [`SPEAKER`], set for the turn by the dispatcher — never from an
//! argument the model could fill in with somebody else's name.

use niles_recognition::{NOTES_LIMIT, VoiceRoster};
use niles_tools::{Error, Result, Tool, ToolDescriptor, ToolRegistry};
use serde_json::{Value, json};
use std::sync::Arc;

tokio::task_local! {
    /// The slug of the voice this turn belongs to, if Niles knows it.
    pub static SPEAKER: Option<String>;
}

fn speaker(tool: &str) -> Result<String> {
    SPEAKER
        .try_with(Clone::clone)
        .ok()
        .flatten()
        .ok_or_else(|| Error::InvalidArgs {
            tool: tool.into(),
            reason: "I don't know whose voice this is, so there is nobody to remember it for"
                .into(),
        })
}

async fn notes_of(roster: &dyn VoiceRoster, speaker: &str) -> Result<String> {
    let voices = roster
        .voices()
        .await
        .map_err(|e| Error::Memory(e.to_string()))?;
    Ok(voices
        .into_iter()
        .find(|v| v.speaker == speaker)
        .and_then(|v| v.notes)
        .unwrap_or_default())
}

/// One fact per line. Adding the same line twice is a no-op; removing
/// takes out every line containing the text.
pub fn edit_notes(notes: &str, action: &str, text: &str) -> std::result::Result<String, String> {
    let text = text.trim().trim_start_matches("- ").trim();
    let mut lines: Vec<String> = notes
        .lines()
        .map(str::to_string)
        .filter(|l| !l.trim().is_empty())
        .collect();
    match action {
        "add" => {
            if text.is_empty() {
                return Err("nothing to remember".into());
            }
            let line = format!("- {text}");
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
        "remove" => {
            let before = lines.len();
            let needle = text.to_lowercase();
            lines.retain(|l| !l.to_lowercase().contains(&needle));
            if lines.len() == before {
                return Err(format!("nothing in the notes mentions {text:?}"));
            }
        }
        other => return Err(format!("action must be add, remove or view, not {other:?}")),
    }
    let out = lines.join("\n");
    if out.chars().count() > NOTES_LIMIT {
        return Err(format!(
            "the notes would be over {NOTES_LIMIT} characters; remove something first"
        ));
    }
    Ok(out)
}

/// What a turn knows about who is speaking, beyond their name.
#[derive(Debug, Clone, Default)]
pub struct Known {
    /// The key their voice is enrolled under — what the tools write to.
    pub slug: Option<String>,
    /// How they like to be addressed ("Sir"), when they have said.
    pub address: Option<String>,
    pub notes: Option<String>,
    /// "MM-DD".
    pub birthday: Option<String>,
}

/// "10-03" as "3 October".
pub fn spoken_birthday(birthday: &str) -> Option<String> {
    let (m, d) = birthday.split_once('-')?;
    let date = chrono::NaiveDate::from_ymd_opt(2024, m.parse().ok()?, d.parse().ok()?)?;
    Some(date.format("%-d %B").to_string())
}

/// Whether `today` is the birthday. Somebody born on 29 February has it
/// on the 28th in the years without one, rather than not at all.
pub fn is_birthday(birthday: Option<&str>, today: chrono::NaiveDate) -> bool {
    use chrono::Datelike;
    let Some(b) = birthday else { return false };
    let leap_born = b == "02-29";
    let mmdd = today.format("%m-%d").to_string();
    mmdd == b || (leap_born && mmdd == "02-28" && today.with_day(29).is_none())
}

/// "MM-DD" from a month and day that exist in some year.
pub fn birthday(month: u32, day: u32) -> Option<String> {
    chrono::NaiveDate::from_ymd_opt(2024, month, day).map(|_| format!("{month:02}-{day:02}"))
}

pub struct RememberAboutMe {
    roster: Arc<dyn VoiceRoster>,
}

#[async_trait::async_trait]
impl Tool for RememberAboutMe {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "remember_about_me".into(),
            description: "Keep or forget a fact about the person speaking, in their own notes, \
                which you are shown whenever they speak to you. Use it when they ask you to \
                remember or forget something about themselves, or tell you something about \
                themselves worth knowing next time. One short fact per call. Facts about one \
                person go here rather than in memory, which is for the household as a whole. \
                For a birthday use set_my_birthday instead."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": { "type": "string", "enum": ["add", "remove", "view"] },
                    "text": { "type": "string", "description": "The fact to add, or text identifying the one to remove." }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let who = speaker("remember_about_me")?;
        let action = args.get("action").and_then(Value::as_str).unwrap_or("view");
        let notes = notes_of(self.roster.as_ref(), &who).await?;
        if action == "view" {
            return Ok(json!({ "notes": notes }));
        }
        let text = args.get("text").and_then(Value::as_str).unwrap_or("");
        let next = edit_notes(&notes, action, text).map_err(|reason| Error::InvalidArgs {
            tool: "remember_about_me".into(),
            reason,
        })?;
        self.roster
            .set_notes(&who, (!next.is_empty()).then_some(next.as_str()))
            .await
            .map_err(|e| Error::Memory(e.to_string()))?;
        tracing::info!("{action} in {who}'s notes: {text:?}");
        Ok(json!({ "ok": true, "notes": next }))
    }
}

pub struct SetMyBirthday {
    roster: Arc<dyn VoiceRoster>,
}

#[async_trait::async_trait]
impl Tool for SetMyBirthday {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "set_my_birthday".into(),
            description: "Remember the birthday of the person speaking, so you can wish them a \
                happy birthday on the day. Month and day only."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["month", "day"],
                "properties": {
                    "month": { "type": "integer", "minimum": 1, "maximum": 12 },
                    "day": { "type": "integer", "minimum": 1, "maximum": 31 }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let who = speaker("set_my_birthday")?;
        let num = |k: &str| args.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        let date = birthday(num("month"), num("day")).ok_or_else(|| Error::InvalidArgs {
            tool: "set_my_birthday".into(),
            reason: "that is not a date".into(),
        })?;
        self.roster
            .set_birthday(&who, Some(&date))
            .await
            .map_err(|e| Error::Memory(e.to_string()))?;
        tracing::info!("{who}'s birthday is {date}");
        Ok(json!({ "ok": true, "birthday": date }))
    }
}

pub fn register(reg: &mut ToolRegistry, roster: Arc<dyn VoiceRoster>) {
    reg.register(Box::new(RememberAboutMe {
        roster: roster.clone(),
    }));
    reg.register(Box::new(SetMyBirthday { roster }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fact_is_a_line() {
        let n = edit_notes("", "add", "Prefers tea to coffee").unwrap();
        assert_eq!(n, "- Prefers tea to coffee");
        let n = edit_notes(&n, "add", "Supports Arsenal").unwrap();
        assert_eq!(n, "- Prefers tea to coffee\n- Supports Arsenal");
    }

    #[test]
    fn the_same_fact_twice_is_once() {
        let n = edit_notes("- Supports Arsenal", "add", "Supports Arsenal").unwrap();
        assert_eq!(n, "- Supports Arsenal");
    }

    #[test]
    fn forgetting_takes_out_the_line() {
        let n = edit_notes("- Prefers tea\n- Supports Arsenal", "remove", "arsenal").unwrap();
        assert_eq!(n, "- Prefers tea");
        assert!(edit_notes(&n, "remove", "chess").is_err());
    }

    #[test]
    fn the_notes_have_a_limit() {
        let long = "x".repeat(NOTES_LIMIT);
        assert!(edit_notes(&long, "add", "one more").is_err());
    }

    #[test]
    fn a_birthday_is_a_day_that_exists() {
        assert_eq!(birthday(10, 3).as_deref(), Some("10-03"));
        assert_eq!(birthday(2, 29).as_deref(), Some("02-29"));
        assert_eq!(birthday(2, 30), None);
        assert_eq!(birthday(13, 1), None);
    }

    #[test]
    fn a_birthday_is_said_as_a_day() {
        assert_eq!(spoken_birthday("10-03").as_deref(), Some("3 October"));
        assert_eq!(spoken_birthday("nonsense"), None);
    }

    #[test]
    fn a_birthday_comes_once_a_year() {
        let day = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert!(is_birthday(Some("10-03"), day(2026, 10, 3)));
        assert!(is_birthday(Some("10-03"), day(2027, 10, 3)));
        assert!(!is_birthday(Some("10-03"), day(2026, 10, 4)));
        assert!(!is_birthday(None, day(2026, 10, 3)));
    }

    #[test]
    fn a_leap_day_birthday_is_kept_in_other_years() {
        let day = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert!(is_birthday(Some("02-29"), day(2028, 2, 29)));
        assert!(!is_birthday(Some("02-29"), day(2028, 2, 28)));
        assert!(is_birthday(Some("02-29"), day(2027, 2, 28)));
    }

    #[tokio::test]
    async fn nobody_speaking_is_nobody_to_remember_for() {
        assert!(speaker("remember_about_me").is_err());
        let who = SPEAKER
            .scope(Some("mark".into()), async { speaker("x") })
            .await;
        assert_eq!(who.unwrap(), "mark");
    }
}
