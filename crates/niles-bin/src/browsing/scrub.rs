//! The backstop: what Niles's browser sends the model, the log and
//! tracing is scrubbed of the person's saved usernames and passwords.
//! Every form a value can take on a page or in a URL is masked, so a
//! value the page shows back — in text, an address, a card — reads as
//! `•••`. See ARCHITECTURE.md's "What the model gets".

use niles_api::chat::{ApprovalCard, CardField};
use niles_logins::Credentials;
use serde_json::Value;

/// What a saved value reads as after scrubbing.
const MASK: &str = "•••";

/// Each saved username and password in every form it can appear in on
/// a page or in a URL. Values under four characters aren't kept:
/// cutting them out of every page would corrupt prices and words.
#[derive(Debug, Default)]
pub(crate) struct Scrub {
    /// Longest first, so a longer value that contains another is
    /// masked whole.
    values: Vec<String>,
}

impl Scrub {
    /// Nothing to scrub: a browsing without a store.
    pub(crate) fn none() -> Scrub {
        Scrub::default()
    }

    /// The person's saved values, in their page forms: exact,
    /// whitespace-squeezed, form-urlencoded (`+` for a space, `*`
    /// literal) and percent-encoded.
    pub(crate) fn new(credentials: Vec<Credentials>) -> Scrub {
        let mut values: Vec<String> = Vec::new();
        for creds in &credentials {
            for value in [creds.username.expose(), creds.password.expose()] {
                push_forms(value, &mut values);
            }
        }
        values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        values.dedup();
        Scrub { values }
    }

    /// `s` with every saved value replaced. A byte scan with
    /// ASCII-case-insensitive compares: matches fall on char boundaries
    /// because the needles are whole UTF-8 strings, and the rest is
    /// copied one char at a time.
    pub(crate) fn text(&self, s: &str) -> String {
        if self.values.is_empty() {
            return s.to_string();
        }
        let bytes = s.as_bytes();
        let mut out = String::with_capacity(s.len());
        let mut at = 0;
        while at < bytes.len() {
            let hit = self.values.iter().find(|needle| {
                let needle = needle.as_bytes();
                at + needle.len() <= bytes.len()
                    && bytes[at..at + needle.len()].eq_ignore_ascii_case(needle)
            });
            if let Some(needle) = hit {
                out.push_str(MASK);
                at += needle.len();
            } else {
                let len = s[at..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&s[at..at + len]);
                at += len;
            }
        }
        out
    }

    /// `value` scrubbed through arrays and object values; keys, which
    /// Niles writes itself, are left alone.
    pub(crate) fn value(&self, value: Value) -> Value {
        match value {
            Value::Array(items) => Value::Array(items.into_iter().map(|v| self.value(v)).collect()),
            Value::Object(map) => {
                Value::Object(map.into_iter().map(|(k, v)| (k, self.value(v))).collect())
            }
            Value::String(s) => Value::String(self.text(&s)),
            other => other,
        }
    }

    /// The error with every text it carries scrubbed; variants without
    /// page text pass unchanged.
    pub(crate) fn error(&self, error: niles_tools::Error) -> niles_tools::Error {
        use niles_tools::Error;
        match error {
            Error::InvalidArgs { tool, reason } => Error::InvalidArgs {
                tool,
                reason: self.text(&reason),
            },
            Error::Remote(reason) => Error::Remote(self.text(&reason)),
            Error::Lapsed(reason) => Error::Lapsed(self.text(&reason)),
            Error::Internal(reason) => Error::Internal(self.text(&reason)),
            other => other,
        }
    }

    /// The result, scrubbed either way.
    pub(crate) fn result(&self, result: niles_tools::Result<Value>) -> niles_tools::Result<Value> {
        result.map(|v| self.value(v)).map_err(|e| self.error(e))
    }

    /// The card with its title, subtitle, field values, body and
    /// outcome scrubbed, before it is held: the ledger, the panel and
    /// the chat log get the scrubbed card.
    pub(crate) fn card(&self, card: ApprovalCard) -> ApprovalCard {
        ApprovalCard {
            title: self.text(&card.title),
            subtitle: self.text(&card.subtitle),
            fields: card
                .fields
                .into_iter()
                .map(|field| CardField {
                    label: field.label,
                    value: self.text(&field.value),
                })
                .collect(),
            body: card.body.map(|body| self.text(&body)),
            outcome: card.outcome.map(|outcome| self.text(&outcome)),
            ..card
        }
    }
}

/// Every form `value` can take on a page or in a URL, pushed when it is
/// at least four characters.
fn push_forms(value: &str, out: &mut Vec<String>) {
    if value.chars().count() < 4 {
        return;
    }
    out.push(value.to_string());
    let squeezed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if squeezed != value {
        out.push(squeezed);
    }
    // As a form posts it: the alphanumerics and `*-._` stay, spaces
    // become '+' (url::form_urlencoded::byte_serialize's set).
    let form: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
    if form != *value {
        out.push(form);
    }
    // Percent-encoded as a URL writes it: unreserved bytes stay, the
    // rest go uppercase %XX.
    let encoded: String = value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            b => format!("%{b:02X}"),
        })
        .collect();
    out.push(encoded);
    let plus = value.replace(' ', "+");
    if plus != value {
        out.push(plus);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_logins::Secret;

    fn scrub_with(username: &str, password: &str) -> Scrub {
        Scrub::new(vec![Credentials {
            username: Secret::new(username),
            password: Secret::new(password),
        }])
    }

    #[test]
    fn a_username_is_masked_case_insensitively() {
        let scrub = scrub_with("Mark.Scott+TellNoOne@Example.com", "irrelevant");
        let page = "Signed in as MARK.SCOTT+TELLNOONE@EXAMPLE.COM, welcome back.";
        assert_eq!(
            scrub.text(page),
            format!("Signed in as {MASK}, welcome back.")
        );
    }

    #[test]
    fn a_password_is_masked_exact_and_a_squeezed_username_too() {
        let scrub = scrub_with("mark  scott@example.com", "correct horse battery staple");
        assert_eq!(scrub.text("p: correct horse battery staple"), "p: •••");
        // A page squeezes whitespace runs to one space: the squeezed
        // form of a double-spaced username is masked too.
        assert_eq!(
            scrub.text("Signed in as mark scott@example.com"),
            "Signed in as •••"
        );
    }

    #[test]
    fn url_forms_are_masked() {
        let scrub = scrub_with("mark scott@example.com", "p+ss");
        // A query value percent-encodes the space %20 and the @ %40;
        // unreserved bytes stay.
        assert_eq!(scrub.text("u=mark%20scott%40example.com"), "u=•••");
        // A form post turns the space into a plus.
        assert_eq!(scrub.text("u=mark+scott@example.com"), "u=•••");
        // A plus inside the value encodes as %2B.
        assert_eq!(scrub.text("p%2Bss"), "•••");
        // Form-encoding leaves a '*' literal where component-encoding
        // goes %2A (and encodes '@' like the component form), so it is
        // a form of its own.
        let starred = scrub_with("jo*star@example.com", "irrelevant");
        assert_eq!(starred.text("u=jo*star%40example.com"), "u=•••");
        assert_eq!(starred.text("u=jo%2Astar%40example.com"), "u=•••");
    }

    #[test]
    fn short_values_are_untouched_but_four_characters_mask() {
        let scrub = scrub_with("ab", "cd3");
        assert_eq!(scrub.text("ab and cd3 stay"), "ab and cd3 stay");
        let four = scrub_with("irrelevant", "c123");
        assert_eq!(four.text("the c123 value"), "the ••• value");
    }

    #[test]
    fn nested_json_is_scrubbed_and_keys_kept() {
        let scrub = scrub_with("majse.private@example.com", "s3cret");
        let value = serde_json::json!({
            "login": "majse.private@example.com",
            "nested": ["s3cret", { "keep": "MAJSE.PRIVATE@EXAMPLE.COM" }]
        });
        let out = scrub.value(value);
        assert_eq!(out["login"], MASK);
        assert_eq!(out["nested"][0], MASK);
        assert_eq!(out["nested"][1]["keep"], MASK);
        assert!(out.get("login").is_some());
    }

    #[test]
    fn an_errors_text_is_scrubbed() {
        let scrub = scrub_with("majse.private@example.com", "irrelevant");
        let error = niles_tools::Error::InvalidArgs {
            tool: "browser_read".into(),
            reason: "the page named majse.private@example.com".into(),
        };
        let scrubbed = scrub.error(error);
        assert!(scrubbed.to_string().contains("the page named •••"));
        // A fixed sentence without a value passes byte for byte.
        let fixed = niles_tools::Error::Lapsed("Nothing is open in the browser yet.".into());
        assert_eq!(
            scrub.error(fixed).to_string(),
            "Nothing is open in the browser yet."
        );
    }

    #[test]
    fn a_cards_fields_body_and_outcome_are_scrubbed() {
        let scrub = scrub_with("majse.private@example.com", "s3cret");
        let card = ApprovalCard {
            id: "c1".into(),
            title: "Sign in for majse.private@example.com?".into(),
            subtitle: "s3cret".into(),
            icon: Some("browser".into()),
            fields: vec![
                CardField {
                    label: "Page".into(),
                    value: "account".into(),
                },
                CardField {
                    label: "Login".into(),
                    value: "Shop".into(),
                },
            ],
            body: Some("the body holds s3cret".into()),
            allow_conversation: false,
            picture: true,
            status: niles_api::chat::ApprovalStatus::Pending,
            outcome: Some("s3cret went wrong".into()),
        };
        let scrubbed = scrub.card(card);
        assert_eq!(scrubbed.title, format!("Sign in for {MASK}?"));
        assert_eq!(scrubbed.subtitle, MASK);
        assert_eq!(scrubbed.fields[0].value, "account");
        assert_eq!(scrubbed.body.as_deref(), Some("the body holds •••"));
        assert_eq!(scrubbed.outcome.as_deref(), Some("••• went wrong"));
    }

    #[test]
    fn a_longer_value_containing_another_is_masked_whole() {
        let mut values = Vec::new();
        push_forms("hunter2", &mut values);
        push_forms("hunter2extra", &mut values);
        values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        values.dedup();
        let scrub = Scrub { values };
        assert_eq!(scrub.text("hunter2extra"), MASK);
        assert_eq!(scrub.text("hunter2"), MASK);
    }
}
