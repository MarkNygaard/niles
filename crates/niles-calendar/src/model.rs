use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// Where a calendar lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// Outlook.com and Entra ID, through Microsoft Graph.
    Microsoft,
    /// Gmail and Workspace, through the Google Calendar API.
    Google,
    /// The calendar on an iPhone, through CalDAV.
    Icloud,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Microsoft => "microsoft",
            Provider::Google => "google",
            Provider::Icloud => "icloud",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "microsoft" => Some(Provider::Microsoft),
            "google" => Some(Provider::Google),
            "icloud" => Some(Provider::Icloud),
            _ => None,
        }
    }

    /// Whether the provider itself holds a connection to its access.
    /// iCloud has no credential narrower than the whole account, so
    /// there read only is a rule kept here.
    pub fn enforces_access(self) -> bool {
        !matches!(self, Provider::Icloud)
    }
}

/// What a connection may do. For Microsoft and Google it is also what
/// they are asked to grant, so a read-only token cannot write at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Read,
    Write,
}

impl Access {
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
    pub provider: Provider,
    /// What the person called it: "Private", "Work".
    pub name: String,
    /// The account's address, as the provider gave it.
    pub account: String,
    pub access: Access,
    /// A refresh token, or iCloud's app-specific password.
    pub credential: String,
    /// Where the provider keeps this account's calendars, when that is
    /// not a fixed address: iCloud's calendar home.
    pub endpoint: Option<String>,
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
    /// One occurrence of a repeating event. Changed or removed as part
    /// of its series by some providers, so the tools are told.
    pub recurring: bool,
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
