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

use chrono::{Datelike, NaiveDate};
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
    /// "MM-DD", or "YYYY-MM-DD" when they have said the year.
    pub birthday: Option<String>,
}

/// What the prompt says about a birthday: the day, how old they are when
/// the year is known, and whether it is today. Worked out here rather than
/// left to the model, which gets date arithmetic wrong often enough to
/// wish somebody a happy 35th on their 36th.
pub fn birthday_line(birthday: &str, today: Option<NaiveDate>) -> Option<String> {
    let (year, month, day) = niles_recognition::birthday::parse(birthday)?;
    let date = NaiveDate::from_ymd_opt(year.unwrap_or(2024), month, day)?;
    let mut line = format!("Their birthday is {}", date.format("%-d %B"));
    if let Some(year) = year {
        line.push_str(&format!(" {year}"));
    }
    let age = today.and_then(|t| niles_recognition::birthday::age(birthday, t));
    let is_today = today.is_some_and(|t| niles_recognition::birthday::is_today(birthday, t));
    line.push_str(&match (is_today, age) {
        (true, Some(age)) => format!(". Today is their birthday: they are {age} today."),
        (true, None) => ". Today is their birthday.".to_string(),
        (false, Some(age)) => format!("; they are {age}."),
        (false, None) => ".".to_string(),
    });
    Some(line)
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
                happy birthday on the day. Give the year only if they said it; with it, \
                you will know their age."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["month", "day"],
                "properties": {
                    "month": { "type": "integer", "minimum": 1, "maximum": 12 },
                    "day": { "type": "integer", "minimum": 1, "maximum": 31 },
                    "year": { "type": "integer", "minimum": 1900 }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let who = speaker("set_my_birthday")?;
        let num = |k: &str| args.get(k).and_then(Value::as_u64);
        let given = match num("year") {
            Some(year) => format!(
                "{year}-{}-{}",
                num("month").unwrap_or(0),
                num("day").unwrap_or(0)
            ),
            None => format!("{}-{}", num("month").unwrap_or(0), num("day").unwrap_or(0)),
        };
        let date = niles_recognition::birthday::normalise(&given, chrono::Utc::now().year())
            .map_err(|reason| Error::InvalidArgs {
                tool: "set_my_birthday".into(),
                reason,
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

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn the_model_is_told_their_age_rather_than_left_to_work_it_out() {
        assert_eq!(
            birthday_line("1990-10-03", Some(day(2026, 10, 3))).unwrap(),
            "Their birthday is 3 October 1990. Today is their birthday: they are 36 today."
        );
        assert_eq!(
            birthday_line("1990-10-03", Some(day(2026, 6, 1))).unwrap(),
            "Their birthday is 3 October 1990; they are 35."
        );
    }

    #[test]
    fn without_a_year_there_is_no_age() {
        assert_eq!(
            birthday_line("10-03", Some(day(2026, 10, 3))).unwrap(),
            "Their birthday is 3 October. Today is their birthday."
        );
        assert_eq!(
            birthday_line("10-03", None).unwrap(),
            "Their birthday is 3 October."
        );
        assert_eq!(birthday_line("nonsense", None), None);
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
