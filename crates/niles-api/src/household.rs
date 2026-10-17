//! Who is in the household: everybody `[auth] allowed` names, by the key
//! their own things are kept under, with the name they go by.
//!
//! A shared artifact keeps only keys: who it is shared with, who changed
//! it last. The names are read from here each time it is shown, so a
//! voice renamed in Settings shows its new name everywhere at once, and
//! a key nobody in the household has any more (someone taken off the
//! list, or whose voice was unlinked, which changes their key) shows as
//! nobody rather than as an address.

use niles_recognition::{EnrolledSpeaker, VoiceRoster};
use serde::Serialize;

/// Someone in the household.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Member {
    /// `person:{slug}` or `email:{address}`, as their chat is kept under.
    pub person: String,
    pub name: String,
    /// The address they sign in with, to know them by when someone
    /// names them so. Not for showing anyone.
    #[serde(skip)]
    pub email: String,
}

/// Everybody `auth` allows in, each named by their voice's display name
/// when they are linked to a voice Niles knows, and otherwise by the key
/// itself ([`name_from_key`]).
pub fn members(auth: &niles_config::AuthConfig, voices: &[EnrolledSpeaker]) -> Vec<Member> {
    auth.allowed
        .iter()
        .map(|p| {
            let person = niles_training::person_key(p.speaker.as_deref(), &p.email);
            let voice = p
                .speaker
                .as_deref()
                .and_then(|s| voices.iter().find(|v| v.speaker == s))
                .map(|v| v.display_name.clone());
            let name = voice.unwrap_or_else(|| name_from_key(&person));
            Member {
                person,
                name,
                email: p.email.clone(),
            }
        })
        .collect()
}

/// Why nobody, or more than one, answers to a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    Nobody,
    /// Each one it could be, by name.
    Several(Vec<String>),
}

/// Who in `members` someone means by `said`, as a person would say it:
/// the name they go by ("Majse", "majse", "Majse's"), its first word
/// ("Mark" for "Mark Leth"), their voice's slug, or their address. A
/// whole name, slug or address is taken before a first word, so "Mark"
/// is Mark even beside a "Mark Jensen" whose name only starts so.
pub fn resolve<'a>(members: &'a [Member], said: &str) -> Result<&'a Member, Unresolved> {
    let said = bare(said);
    if said.is_empty() {
        return Err(Unresolved::Nobody);
    }
    let whole: Vec<&Member> = members
        .iter()
        .filter(|m| {
            m.name.to_lowercase() == said
                || m.person.strip_prefix("person:") == Some(said.as_str())
                || m.email.to_lowercase() == said
                || m.email.split('@').next().map(str::to_lowercase).as_deref() == Some(&said)
        })
        .collect();
    let found = if whole.is_empty() {
        members
            .iter()
            .filter(|m| {
                m.name
                    .split_whitespace()
                    .next()
                    .map(str::to_lowercase)
                    .as_deref()
                    == Some(&said)
            })
            .collect()
    } else {
        whole
    };
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(Unresolved::Nobody),
        several => Err(Unresolved::Several(
            several.iter().map(|m| m.name.clone()).collect(),
        )),
    }
}

/// A name as said, without its case, the space around it, or a
/// possessive: "Majse's" and "James'" are Majse and James.
fn bare(said: &str) -> String {
    let said = said.trim().to_lowercase();
    for ending in ["'s", "’s", "'", "’"] {
        if let Some(stem) = said.strip_suffix(ending) {
            return stem.trim().to_string();
        }
    }
    said
}

/// The household as the config and the voices have it now. Nobody,
/// without a config; and a voice roster that cannot be read costs the
/// display names, not the people.
pub async fn household(
    config: Option<&niles_config::ConfigStore>,
    voices: Option<&dyn VoiceRoster>,
) -> Vec<Member> {
    let Some(config) = config else {
        return Vec::new();
    };
    let enrolled = match voices {
        Some(roster) => roster.voices().await.unwrap_or_default(),
        None => Vec::new(),
    };
    members(&config.current().auth, &enrolled)
}

/// A name from a key alone: the voice's slug, or the address before its
/// @, with its first letter upper-cased. "person:majse" is "Majse".
pub fn name_from_key(key: &str) -> String {
    let bare = key
        .strip_prefix("person:")
        .or_else(|| {
            key.strip_prefix("email:")
                .map(|a| a.split('@').next().unwrap_or(a))
        })
        .unwrap_or(key);
    let mut chars = bare.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What `person` is called in the household; `None` for a key nobody in
/// it has.
pub fn name_of<'a>(members: &'a [Member], person: &str) -> Option<&'a str> {
    members
        .iter()
        .find(|m| m.person == person)
        .map(|m| m.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> niles_config::AuthConfig {
        toml::from_str(
            r#"
allowed = [
  { email = "mark@example.com", speaker = "mark" },
  { email = "anna.berg@example.com" },
]
"#,
        )
        .unwrap()
    }

    fn voice(speaker: &str, name: &str) -> EnrolledSpeaker {
        serde_json::from_value(serde_json::json!({
            "speaker": speaker,
            "display_name": name,
            "created_at": "2026-01-01T00:00:00Z",
            "last_seen_at": null,
            "clip_count": 0,
            "embeddings": [],
        }))
        .unwrap()
    }

    #[test]
    fn everybody_allowed_in_is_named_by_their_voice_or_their_key() {
        let people = members(&auth(), &[voice("mark", "Mark Leth")]);
        assert_eq!(
            people,
            vec![
                Member {
                    person: "person:mark".into(),
                    name: "Mark Leth".into(),
                    email: "mark@example.com".into(),
                },
                Member {
                    person: "email:anna.berg@example.com".into(),
                    name: "Anna.berg".into(),
                    email: "anna.berg@example.com".into(),
                },
            ]
        );
        let unknown_voice = members(&auth(), &[]);
        assert_eq!(unknown_voice[0].name, "Mark");
    }

    #[test]
    fn a_key_nobody_has_any_more_is_nobody() {
        let people = members(&auth(), &[]);
        assert_eq!(name_of(&people, "person:mark"), Some("Mark"));
        assert_eq!(name_of(&people, "person:gone"), None);
    }

    fn member(person: &str, name: &str, email: &str) -> Member {
        Member {
            person: person.into(),
            name: name.into(),
            email: email.into(),
        }
    }

    fn house() -> Vec<Member> {
        vec![
            member("person:mark", "Mark Leth", "mark@example.com"),
            member("person:majse", "Majse", "majse@example.com"),
            member("email:anna@example.com", "Anna", "anna@example.com"),
            member("person:anna-b", "Anna Berg", "berg@example.com"),
        ]
    }

    #[test]
    fn someone_is_found_by_what_a_person_would_call_them() {
        let house = house();
        let who = |said: &str| resolve(&house, said).map(|m| m.person.as_str());
        assert_eq!(who("Majse"), Ok("person:majse"));
        assert_eq!(who("  majse "), Ok("person:majse"), "any case");
        assert_eq!(who("Majse's"), Ok("person:majse"), "a possessive");
        assert_eq!(who("MAJSE’S"), Ok("person:majse"));
        assert_eq!(who("mark leth"), Ok("person:mark"), "the whole name");
        assert_eq!(who("Mark"), Ok("person:mark"), "its first word");
        assert_eq!(who("mark"), Ok("person:mark"), "the voice's slug");
        assert_eq!(who("majse@example.com"), Ok("person:majse"), "an address");
        assert_eq!(
            who("Anna"),
            Ok("email:anna@example.com"),
            "a whole name before a first word"
        );
        assert_eq!(who("Anna Berg"), Ok("person:anna-b"));
    }

    #[test]
    fn nobody_or_several_is_said_as_such() {
        assert_eq!(resolve(&house(), "Bob"), Err(Unresolved::Nobody));
        assert_eq!(resolve(&house(), " "), Err(Unresolved::Nobody));
        let house = vec![
            member("person:ml", "Mark Leth", "ml@example.com"),
            member("person:mj", "Mark Jensen", "mj@example.com"),
        ];
        assert_eq!(
            resolve(&house, "mark"),
            Err(Unresolved::Several(vec![
                "Mark Leth".into(),
                "Mark Jensen".into()
            ]))
        );
    }

    #[test]
    fn a_name_from_a_key_alone() {
        assert_eq!(name_from_key("person:majse"), "Majse");
        assert_eq!(name_from_key("email:anna@example.com"), "Anna");
        assert_eq!(name_from_key("anyone"), "Anyone");
    }
}
