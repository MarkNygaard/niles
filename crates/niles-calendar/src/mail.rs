//! Mail, as the tools read and send it: the same for every provider.
//!
//! Niles keeps no copy of anybody's mail. It reads what a question
//! needs, from the provider, when asked.

use base64::Engine;
use chrono::{DateTime, NaiveDate, Utc};

/// What a question about mail is looking for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Search {
    /// The provider's own search words. With them the whole mailbox is
    /// searched; without, the inbox is listed.
    pub query: Option<String>,
    pub from: Option<String>,
    pub since: Option<NaiveDate>,
    pub unread_only: bool,
    pub limit: usize,
}

/// The most a search answers with.
pub const MAX_RESULTS: usize = 20;

/// The longest a message's text is read out to the model.
pub const MAX_TEXT: usize = 16 * 1024;

/// One message as a search finds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The provider's id, which only means anything in its account.
    pub id: String,
    pub from: String,
    pub subject: String,
    pub received: DateTime<Utc>,
    pub unread: bool,
    pub excerpt: String,
}

/// One message, read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub received: Option<DateTime<Utc>>,
    /// The text, quoted history cut off, at most [`MAX_TEXT`].
    pub text: String,
    /// Attachments, by name: named, never opened.
    pub attachments: Vec<String>,
    /// What a reply needs to stay in its thread.
    pub message_id: Option<String>,
    pub references: Option<String>,
    pub thread_id: Option<String>,
}

/// A mail to send, exactly as the person approved it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Draft {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// "Re: " once, however many replies deep.
pub fn reply_subject(subject: &str) -> String {
    let trimmed = subject.trim();
    if trimmed.to_ascii_lowercase().starts_with("re:") {
        trimmed.to_string()
    } else {
        format!("Re: {trimmed}")
    }
}

/// HTML as readable text.
pub fn html_to_text(html: &str) -> String {
    html2text::from_read(html.as_bytes(), 100).unwrap_or_default()
}

/// The new part of a message: what follows the first line that opens
/// quoted history is the conversation so far, already read.
pub fn trim_quoted(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        // "On Mon, 12 Oct 2026 at 09:00, Jakob <j@x.dk> wrote:", and
        // Danish mail's "Den 12. okt. 2026 kl. 09.00 skrev Jakob <j@x.dk>:".
        let opens_history = (t.starts_with("On ") && t.ends_with("wrote:"))
            || (t.starts_with("Den ") && t.contains(" skrev ") && t.ends_with(':'))
            || t.starts_with("-----Original Message-----")
            || t.starts_with("-----Oprindelig meddelelse-----")
            || (t.starts_with("From:") && kept.last().is_some_and(|l| l.trim().is_empty()))
            || t.starts_with("________________________________");
        if opens_history && !kept.iter().all(|l| l.trim().is_empty()) {
            break;
        }
        if t.starts_with('>') {
            continue;
        }
        kept.push(line);
    }
    let text = kept.join("\n");
    cap(text.trim())
}

fn cap(text: &str) -> String {
    if text.len() <= MAX_TEXT {
        return text.to_string();
    }
    let mut cut = MAX_TEXT;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n[Cut off here: the message goes on.]", &text[..cut])
}

/// A plain-text message as RFC 5322, for a provider that takes it raw:
/// Gmail. `reply` threads it under the message it answers.
pub fn rfc822(from: &str, draft: &Draft, reply: Option<&Message>) -> String {
    let mut out = String::new();
    out.push_str(&format!("From: {from}\r\n"));
    out.push_str(&format!("To: {}\r\n", draft.to.join(", ")));
    if !draft.cc.is_empty() {
        out.push_str(&format!("Cc: {}\r\n", draft.cc.join(", ")));
    }
    out.push_str(&format!("Subject: {}\r\n", header_text(&draft.subject)));
    if let Some(original) = reply
        && let Some(id) = &original.message_id
    {
        out.push_str(&format!("In-Reply-To: {id}\r\n"));
        let references = match &original.references {
            Some(earlier) => format!("{earlier} {id}"),
            None => id.clone(),
        };
        out.push_str(&format!("References: {references}\r\n"));
    }
    out.push_str("MIME-Version: 1.0\r\n");
    out.push_str("Content-Type: text/plain; charset=utf-8\r\n");
    out.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
    let body = base64::engine::general_purpose::STANDARD.encode(draft.body.as_bytes());
    for chunk in body.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push_str("\r\n");
    }
    out
}

/// A header value, encoded when it is not plain ASCII (RFC 2047).
fn header_text(value: &str) -> String {
    let one_line = value.replace(['\r', '\n'], " ");
    if one_line.is_ascii() {
        return one_line;
    }
    format!(
        "=?UTF-8?B?{}?=",
        base64::engine::general_purpose::STANDARD.encode(one_line.as_bytes())
    )
}

/// An address list as people write it: "a@x, b@y; c@z".
pub fn addresses(list: &str) -> Vec<String> {
    list.split([',', ';'])
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether `address` looks like one: something, an @, a dotted domain.
pub fn plausible(address: &str) -> bool {
    let bare = address
        .rsplit_once('<')
        .map(|(_, rest)| rest.trim_end_matches('>'))
        .unwrap_or(address)
        .trim();
    match bare.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !bare.contains(char::is_whitespace)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_new_part_of_a_reply() {
        let text = "Sounds good, see you at ten.\n\nOn Mon, 12 Oct 2026 at 09:00, Jakob <j@x.dk> wrote:\n> Shall we meet at ten?\n> J";
        assert_eq!(trim_quoted(text), "Sounds good, see you at ten.");
        let outlook = "Yes.\n\nFrom: Jakob\nSent: Monday\nSubject: Ten?";
        assert_eq!(trim_quoted(outlook), "Yes.");
        let danish = "Ja tak.\nDen 12. okt. 2026 kl. 09.00 skrev Jakob <j@x.dk>:\n> Kaffe?";
        assert_eq!(trim_quoted(danish), "Ja tak.");
    }

    #[test]
    fn a_message_that_is_all_quote_keeps_what_it_has() {
        assert_eq!(trim_quoted("> only this"), "");
        assert_eq!(trim_quoted("Hello\nthere"), "Hello\nthere");
    }

    #[test]
    fn says_re_once() {
        assert_eq!(reply_subject("Ten?"), "Re: Ten?");
        assert_eq!(reply_subject("RE: Ten?"), "RE: Ten?");
    }

    #[test]
    fn writes_a_reply_into_its_thread() {
        let draft = Draft {
            to: vec!["jakob@example.com".into()],
            cc: vec![],
            subject: "Re: Kaffe på torsdag".into(),
            body: "Ja tak!".into(),
        };
        let original = Message {
            message_id: Some("<a@x>".into()),
            references: Some("<z@x>".into()),
            ..Message::default()
        };
        let raw = rfc822("mark@gmail.com", &draft, Some(&original));
        assert!(raw.contains("To: jakob@example.com\r\n"));
        assert!(
            raw.contains("Subject: =?UTF-8?B?"),
            "non-ASCII is encoded: {raw}"
        );
        assert!(raw.contains("In-Reply-To: <a@x>\r\n"));
        assert!(raw.contains("References: <z@x> <a@x>\r\n"));
        assert!(raw.ends_with("SmEgdGFrIQ==\r\n"));
    }

    #[test]
    fn reads_an_address_list() {
        assert_eq!(
            addresses("a@x.dk, b@y.com; "),
            ["a@x.dk".to_string(), "b@y.com".to_string()]
        );
        assert!(plausible("Jakob <jakob@example.com>"));
        assert!(plausible("jakob@example.com"));
        assert!(!plausible("jakob"));
        assert!(!plausible("jakob@localhost"));
        assert!(!plausible("ja kob@example.com"));
    }

    #[test]
    fn reads_html_as_text() {
        let text = html_to_text("<p>Hi <b>Mark</b></p><p>See you</p>");
        assert!(text.contains("Hi"), "{text}");
        assert!(text.contains("See you"), "{text}");
        assert!(!text.contains('<'), "{text}");
    }
}
