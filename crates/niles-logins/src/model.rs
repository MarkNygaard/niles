//! The validated shapes of a saved login.

use crate::Origin;
use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

/// The most a label reads.
pub const MAX_LABEL: usize = 60;
/// The most sites one login names.
pub const MAX_ORIGINS: usize = 10;
/// The longest username kept.
pub const MAX_USERNAME: usize = 200;
/// The longest password kept.
pub const MAX_PASSWORD: usize = 500;
/// The longest sign-in code kept.
pub const MAX_CODE: usize = 32;

/// A username or password, exactly as the person typed it.
///
/// There is deliberately no `Serialize` and no `Display`: whatever holds
/// one can't put it in a response, a log line or a chat answer by
/// accident.
#[derive(Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    /// The value itself, for sealing and for filling a form — never for
    /// a response or a log line.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("…")
    }
}

/// What signs in. Both fields print as `…`.
#[derive(Clone, Debug)]
pub struct Credentials {
    pub username: Secret,
    pub password: Secret,
}

/// A saved login as its owner sees it, never the password. `signed_in`
/// means a fresh browser profile is kept: Niles can restore it and the
/// site may not ask for the login again.
#[derive(Clone, PartialEq, Serialize)]
pub struct Login {
    pub id: i64,
    pub label: String,
    pub origins: Vec<Origin>,
    pub username: String,
    pub last_used: Option<DateTime<Utc>>,
    pub signed_in: bool,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("origins", &self.origins)
            .field("username", &"…")
            .field("last_used", &self.last_used)
            .field("signed_in", &self.signed_in)
            .finish()
    }
}

/// All the model may ever know of a login: its id, its label and the
/// site it answers for.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoginRef {
    pub id: i64,
    pub label: String,
    pub site: Origin,
}

/// What the person calls a login, on the page and on a card. Trims
/// surrounding space; refuses an empty, over-long or control-bearing
/// label, with a reason that never quotes it.
pub fn label(given: &str) -> Result<String> {
    let label = given.trim();
    if label.is_empty() {
        return Err(invalid("label", "a label is needed, such as Shop"));
    }
    if label.chars().count() > MAX_LABEL {
        return Err(invalid("label", "a label is at most 60 characters"));
    }
    if label.chars().any(char::is_control) {
        return Err(invalid(
            "label",
            "a label can't hold a line break or another control character",
        ));
    }
    Ok(label.to_string())
}

/// The sites a login answers for: each parsed and normalised, duplicates
/// dropped after normalising while the first keeps its place. Refuses an
/// empty list and more than [`MAX_ORIGINS`] sites.
pub fn origins<S: AsRef<str>>(given: &[S]) -> Result<Vec<Origin>> {
    let mut out: Vec<Origin> = Vec::new();
    for one in given {
        let origin = Origin::parse(one.as_ref())?;
        if !out.contains(&origin) {
            out.push(origin);
        }
    }
    if out.is_empty() {
        return Err(invalid(
            "site",
            "a login needs at least one site, such as https://shop.example",
        ));
    }
    if out.len() > MAX_ORIGINS {
        return Err(invalid("site", "a login names at most 10 sites"));
    }
    Ok(out)
}

/// Surrounding space on a username is never meant; the value inside is
/// kept exactly.
fn username(s: Secret) -> Result<Secret> {
    let name = s.expose().trim();
    if name.is_empty() {
        return Err(invalid("username", "a username is needed"));
    }
    if name.chars().count() > MAX_USERNAME {
        return Err(invalid("username", "a username is at most 200 characters"));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid(
            "username",
            "a username can't hold a line break or another control character",
        ));
    }
    Ok(Secret::new(name))
}

/// A password is kept exactly as pasted: no trimming, so a space at
/// either end is part of it.
fn password(s: Secret) -> Result<Secret> {
    if s.is_empty() {
        return Err(invalid("password", "a password is needed"));
    }
    if s.expose().chars().count() > MAX_PASSWORD {
        return Err(invalid("password", "a password is at most 500 characters"));
    }
    if s.expose().chars().any(char::is_control) {
        return Err(invalid(
            "password",
            "a password can't hold a line break or another control character",
        ));
    }
    Ok(s)
}

/// A sign-in code the site sent, as the person typed it on the card.
/// Whitespace anywhere is dropped — codes come with spaces for reading
/// aloud — then the rest must be 1 to [`MAX_CODE`] characters of ASCII
/// letters, digits and dashes. The reason never quotes the value.
pub fn code(given: Secret) -> Result<Secret> {
    let normalised: String = given
        .expose()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if normalised.is_empty() {
        return Err(invalid("code", "a code is needed"));
    }
    if normalised.chars().count() > MAX_CODE {
        return Err(invalid("code", "a code is at most 32 characters"));
    }
    if !normalised
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(invalid("code", "a code is letters, digits and dashes"));
    }
    Ok(Secret::new(normalised))
}

fn invalid(kind: &'static str, reason: &str) -> Error {
    Error::Invalid {
        kind,
        reason: reason.to_string(),
    }
}

/// A login as it is added. Built with [`NewLogin::new`] so the label,
/// the sites and both secrets are checked before anything is kept.
#[derive(Debug, Clone)]
pub struct NewLogin {
    pub label: String,
    pub origins: Vec<Origin>,
    pub credentials: Credentials,
}

impl NewLogin {
    pub fn new(
        label: &str,
        origins: &[String],
        username: Secret,
        password: Secret,
    ) -> Result<NewLogin> {
        Ok(NewLogin {
            label: crate::model::label(label)?,
            origins: crate::model::origins(origins)?,
            credentials: Credentials {
                username: crate::model::username(username)?,
                password: crate::model::password(password)?,
            },
        })
    }
}

/// A login as it is changed. A `None` password — also what an empty
/// password becomes — keeps the current one; only a `Some` replaces it.
#[derive(Debug, Clone)]
pub struct LoginEdit {
    pub label: String,
    pub origins: Vec<Origin>,
    pub username: Secret,
    pub password: Option<Secret>,
}

impl LoginEdit {
    pub fn new(
        label: &str,
        origins: &[String],
        username: Secret,
        password: Option<Secret>,
    ) -> Result<LoginEdit> {
        Ok(LoginEdit {
            label: crate::model::label(label)?,
            origins: crate::model::origins(origins)?,
            username: crate::model::username(username)?,
            password: password.filter(|p| !p.is_empty()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "majse.private@example.com";
    const PASS: &str = "correct horse battery staple 42";

    fn secrets() -> (Secret, Secret) {
        (Secret::new(USER), Secret::new(PASS))
    }

    #[test]
    fn no_debug_prints_a_username_or_a_password() {
        let (username, password) = secrets();
        let new = NewLogin::new(
            "Shop",
            &["shop.example".to_string()],
            username.clone(),
            password.clone(),
        )
        .unwrap();
        let edit = LoginEdit::new(
            "Shop",
            &["shop.example".to_string()],
            username.clone(),
            Some(password.clone()),
        )
        .unwrap();
        let login = Login {
            id: 1,
            label: "Shop".into(),
            origins: vec![Origin::parse("https://shop.example").unwrap()],
            username: USER.into(),
            last_used: None,
            signed_in: false,
        };
        for printed in [
            format!("{:?}", Credentials { username, password }),
            format!("{new:?}"),
            format!("{edit:?}"),
            format!("{login:?}"),
        ] {
            assert!(!printed.contains(USER), "{printed}");
            assert!(!printed.contains(PASS), "{printed}");
        }
    }

    #[test]
    fn a_reason_never_quotes_a_username_or_a_password() {
        let from_username = username(Secret::new("hunter2\nname")).unwrap_err();
        assert!(!from_username.to_string().contains("hunter2"));
        let from_password = password(Secret::new("hunter2\npass")).unwrap_err();
        assert!(!from_password.to_string().contains("hunter2"));
        let from_label = label("hunter2\nlabel").unwrap_err();
        assert!(!from_label.to_string().contains("hunter2"));
    }

    #[test]
    fn a_pasted_password_with_a_trailing_newline_is_refused() {
        let err = NewLogin::new(
            "Shop",
            &["shop.example".to_string()],
            Secret::new(USER),
            Secret::new(format!("{PASS}\n")),
        )
        .unwrap_err();
        assert!(err.to_string().contains("line break"));
    }

    #[test]
    fn a_password_keeps_its_surrounding_spaces() {
        let (_, password) = secrets();
        let new = NewLogin::new(
            "Shop",
            &["shop.example".to_string()],
            Secret::new(USER),
            password,
        )
        .unwrap();
        assert_eq!(new.credentials.password.expose(), PASS);
    }

    #[test]
    fn a_username_is_trimmed() {
        let new = NewLogin::new(
            "Shop",
            &["shop.example".to_string()],
            Secret::new(format!("  {USER}  ")),
            Secret::new(PASS),
        )
        .unwrap();
        assert_eq!(new.credentials.username.expose(), USER);
    }

    #[test]
    fn an_empty_edit_password_becomes_none() {
        let edit = LoginEdit::new(
            "Shop",
            &["shop.example".to_string()],
            Secret::new(USER),
            Some(Secret::new("")),
        )
        .unwrap();
        assert!(edit.password.is_none());
    }

    #[test]
    fn origins_are_deduped_after_normalising() {
        let new = NewLogin::new(
            "Shop",
            &[
                "shop.example".to_string(),
                "https://SHOP.example/".to_string(),
            ],
            Secret::new(USER),
            Secret::new(PASS),
        )
        .unwrap();
        assert_eq!(new.origins.len(), 1);
        assert_eq!(new.origins[0].as_str(), "https://shop.example");
    }

    #[test]
    fn more_than_ten_sites_is_refused() {
        let sites: Vec<String> = (1..=11).map(|n| format!("shop{n}.example")).collect();
        let err = origins(&sites).unwrap_err();
        assert!(err.to_string().contains("at most 10 sites"));
    }

    #[test]
    fn whitespace_is_dropped_from_a_code() {
        let code = code(Secret::new(" 493 817 ")).unwrap();
        assert_eq!(code.expose(), "493817");
    }

    #[test]
    fn dashes_and_letters_pass_a_code() {
        let code = code(Secret::new("ab-12")).unwrap();
        assert_eq!(code.expose(), "ab-12");
    }

    #[test]
    fn an_empty_code_is_refused() {
        assert!(
            code(Secret::new("   "))
                .unwrap_err()
                .to_string()
                .contains("a code is needed")
        );
    }

    #[test]
    fn a_long_code_is_refused() {
        let too_long = "1".repeat(MAX_CODE + 1);
        assert!(
            code(Secret::new(too_long))
                .unwrap_err()
                .to_string()
                .contains("at most 32 characters")
        );
    }

    #[test]
    fn a_newline_inside_is_whitespace_and_dropped() {
        // Codes arrive split across lines in texts and mails; the
        // whitespace rule drops it, so "12\n34" reads as "1234".
        let code = code(Secret::new("12\n34")).unwrap();
        assert_eq!(code.expose(), "1234");
    }

    #[test]
    fn other_characters_are_refused_and_never_quoted() {
        for bad in ["12;34", "12 34<script>"] {
            let err = code(Secret::new(bad)).unwrap_err();
            assert!(
                err.to_string().contains("letters, digits and dashes"),
                "{err}"
            );
            for part in bad.split_whitespace().flat_map(|s| s.split(';')) {
                assert!(!err.to_string().contains(part), "{err}");
            }
        }
    }
}
