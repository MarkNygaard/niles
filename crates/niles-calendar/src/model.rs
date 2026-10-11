use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// What a connection may do — and so what Microsoft is asked to grant.
/// A read-only connection holds a token that cannot write at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Read,
    Write,
}

impl Access {
    /// The scopes asked for. `offline_access` is what yields a refresh
    /// token; `User.Read` names the account.
    pub fn scopes(self) -> String {
        let calendars = match self {
            Access::Read => "Calendars.Read",
            Access::Write => "Calendars.ReadWrite",
        };
        format!("offline_access User.Read {calendars}")
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Access::Read),
            "write" => Some(Access::Write),
            _ => None,
        }
    }
}

/// One calendar account a person connected.
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub id: String,
    /// The speaker it belongs to, the slug voice and the profile share.
    /// Nothing reads it for anybody else.
    pub person: String,
    /// What the person called it: "Private", "Work".
    pub name: String,
    /// The account's address, as Microsoft gave it.
    pub account: String,
    pub access: Access,
    pub refresh_token: String,
}

/// An event, in the house's time zone.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub subject: String,
    /// Wall-clock time in the house; for an all-day event, midnight of
    /// its first day.
    pub start: NaiveDateTime,
    /// For an all-day event, midnight after its last day.
    pub end: NaiveDateTime,
    pub all_day: bool,
    pub location: Option<String>,
}

/// An event to make, or what an event is to become.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub subject: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub all_day: bool,
    pub location: Option<String>,
}
