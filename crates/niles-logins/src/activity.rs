//! What a saved login was used for, as Vault shows it: when, what,
//! on which site and for which conversation. Never a value: no
//! username, password, code or key, and no page text.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// How long a login's activity is kept.
pub const ACTIVITY_DAYS: i64 = 90;

/// How many of a login's latest uses Vault is given.
pub const ACTIVITY_SHOWN: usize = 200;

/// One kind of use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Used {
    Added,
    Edited,
    SignedOut,
    /// Its browser profile was forgotten: 30 days unused, or the
    /// account or its sites changed.
    ProfileDropped,
    /// A sign-in allowed on its card, typed, and the page asked for
    /// nothing more.
    SignedIn,
    /// Typed, and the page still asked for the login.
    SignInFailed,
    /// Typed, and then the page couldn't be read.
    SignInUncertain,
    /// The browser profile kept from last time was put back.
    ProfileRestored,
    /// A code the person typed on its card was entered.
    CodeEntered,
    /// A code Niles made from the saved authenticator key was entered.
    CodeMade,
}

impl Used {
    pub fn as_str(self) -> &'static str {
        match self {
            Used::Added => "added",
            Used::Edited => "edited",
            Used::SignedOut => "signed_out",
            Used::ProfileDropped => "profile_dropped",
            Used::SignedIn => "signed_in",
            Used::SignInFailed => "sign_in_failed",
            Used::SignInUncertain => "sign_in_uncertain",
            Used::ProfileRestored => "profile_restored",
            Used::CodeEntered => "code_entered",
            Used::CodeMade => "code_made",
        }
    }

    /// The kind a stored name names; `None` for one this Niles doesn't
    /// know, which is left out of what Vault is shown.
    pub fn parse(name: &str) -> Option<Used> {
        Some(match name {
            "added" => Used::Added,
            "edited" => Used::Edited,
            "signed_out" => Used::SignedOut,
            "profile_dropped" => Used::ProfileDropped,
            "signed_in" => Used::SignedIn,
            "sign_in_failed" => Used::SignInFailed,
            "sign_in_uncertain" => Used::SignInUncertain,
            "profile_restored" => Used::ProfileRestored,
            "code_entered" => Used::CodeEntered,
            "code_made" => Used::CodeMade,
            _ => return None,
        })
    }
}

/// Where a use happened: the site's origin and the conversation it was
/// for, when there was one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    pub site: Option<String>,
    pub conversation: Option<i64>,
}

/// One use, as Vault lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Activity {
    pub at: DateTime<Utc>,
    pub used: Used,
    pub site: Option<String>,
    pub conversation: Option<i64>,
}
