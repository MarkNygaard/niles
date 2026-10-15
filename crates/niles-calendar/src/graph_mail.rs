//! Mail through Microsoft Graph: Outlook.com, Hotmail, and work or
//! school accounts.

use crate::error::{Error, Result};
use crate::graph::{GRAPH, first_line, get, http_error, send};
use crate::mail::{Draft, Message, Search, Summary, html_to_text, trim_quoted};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

const SUMMARY: &str = "id,subject,from,receivedDateTime,isRead,bodyPreview";

/// Messages matching `search`, newest first. Filtering by sender, date
/// and unread is done here rather than in `$filter`, which Graph will
/// not combine with ordering by date or with `$search`.
pub async fn search(
    http: &reqwest::Client,
    access_token: &str,
    search: &Search,
) -> Result<Vec<Summary>> {
    // Enough to filter down from without paging.
    let top = 50;
    let url = match &search.query {
        Some(query) => format!(
            "{GRAPH}/me/messages?$search={}&$top={top}&$select={SUMMARY}",
            urlencoding::encode(&format!("\"{}\"", query.replace('"', "")))
        ),
        None => format!(
            "{GRAPH}/me/mailFolders/inbox/messages?$top={top}&$select={SUMMARY}&$orderby=receivedDateTime%20desc"
        ),
    };
    let body = get(http, access_token, &url).await?;
    let mut found: Vec<Summary> = body
        .get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(summary_of)
        .filter(|s| keeps(search, s))
        .collect();
    found.sort_by_key(|s| std::cmp::Reverse(s.received));
    found.truncate(search.limit);
    Ok(found)
}

/// Whether a message is one the search is for, beyond its words.
pub(crate) fn keeps(search: &Search, s: &Summary) -> bool {
    let from_ok = search
        .from
        .as_deref()
        .is_none_or(|from| s.from.to_lowercase().contains(&from.trim().to_lowercase()));
    let since_ok = search
        .since
        .is_none_or(|day| s.received.date_naive() >= day);
    from_ok && since_ok && (!search.unread_only || s.unread)
}

fn summary_of(value: &Value) -> Option<Summary> {
    Some(Summary {
        id: value.get("id")?.as_str()?.to_string(),
        from: who(value.get("from")),
        subject: text(value, "subject"),
        received: value
            .get("receivedDateTime")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc))?,
        unread: !value.get("isRead").and_then(Value::as_bool).unwrap_or(true),
        excerpt: text(value, "bodyPreview"),
    })
}

/// "Jakob Hansen <jakob@example.com>", from a Graph recipient.
fn who(recipient: Option<&Value>) -> String {
    let Some(address) = recipient.and_then(|r| r.get("emailAddress")) else {
        return String::new();
    };
    let email = text(address, "address");
    match address.get("name").and_then(Value::as_str) {
        Some(name) if !name.is_empty() && name != email => format!("{name} <{email}>"),
        _ => email,
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// One message, its text as plain text: Graph converts it when asked.
pub async fn read(http: &reqwest::Client, access_token: &str, id: &str) -> Result<Message> {
    let url = format!(
        "{GRAPH}/me/messages/{}?$select=id,subject,from,toRecipients,ccRecipients,receivedDateTime,body,hasAttachments,internetMessageId,conversationId",
        urlencoding::encode(id)
    );
    let response = http
        .get(&url)
        .bearer_auth(access_token)
        .header("Prefer", "outlook.body-content-type=\"text\"")
        .send()
        .await
        .map_err(http_error)?;
    let status = response.status();
    let raw = response.text().await.map_err(http_error)?;
    if !status.is_success() {
        let reason = serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .unwrap_or(raw);
        return Err(Error::Microsoft {
            status: status.as_u16(),
            reason: first_line(&reason),
        });
    }
    let value: Value = serde_json::from_str(&raw).map_err(|e| Error::Microsoft {
        status: status.as_u16(),
        reason: format!("an answer that would not read: {e}"),
    })?;
    let mut message = message_of(&value);
    if value.get("hasAttachments").and_then(Value::as_bool) == Some(true) {
        let attachments = get(
            http,
            access_token,
            &format!(
                "{GRAPH}/me/messages/{}/attachments?$select=name",
                urlencoding::encode(id)
            ),
        )
        .await?;
        message.attachments = attachments
            .get("value")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| a.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
    }
    Ok(message)
}

pub(crate) fn message_of(value: &Value) -> Message {
    let list = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|r| who(Some(r)))
            .collect()
    };
    let body = value.get("body");
    let content = body.map(|b| text(b, "content")).unwrap_or_default();
    let html = body
        .and_then(|b| b.get("contentType"))
        .and_then(Value::as_str)
        .is_some_and(|t| t.eq_ignore_ascii_case("html"));
    let plain = if html {
        html_to_text(&content)
    } else {
        content
    };
    Message {
        id: text(value, "id"),
        from: who(value.get("from")),
        to: list("toRecipients"),
        cc: list("ccRecipients"),
        subject: text(value, "subject"),
        received: value
            .get("receivedDateTime")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc)),
        text: trim_quoted(&plain),
        attachments: Vec::new(),
        message_id: value
            .get("internetMessageId")
            .and_then(Value::as_str)
            .map(str::to_string),
        references: None,
        thread_id: value
            .get("conversationId")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn recipients(addresses: &[String]) -> Value {
    Value::Array(
        addresses
            .iter()
            .map(|a| json!({ "emailAddress": { "address": a } }))
            .collect(),
    )
}

/// Send a new message, kept in Sent Items as one sent by hand would be.
pub async fn send_new(http: &reqwest::Client, access_token: &str, draft: &Draft) -> Result<()> {
    let body = json!({
        "message": {
            "subject": draft.subject,
            "body": { "contentType": "Text", "content": draft.body },
            "toRecipients": recipients(&draft.to),
            "ccRecipients": recipients(&draft.cc),
        },
        "saveToSentItems": true,
    });
    send(
        http,
        access_token,
        reqwest::Method::POST,
        &format!("{GRAPH}/me/sendMail"),
        Some(body),
    )
    .await
    .map(|_| ())
}

/// Reply to message `id`, in its thread: Graph sets the subject and the
/// threading headers, and quotes the original under `draft.body`.
pub async fn reply(
    http: &reqwest::Client,
    access_token: &str,
    id: &str,
    draft: &Draft,
) -> Result<()> {
    let body = json!({
        "message": {
            "toRecipients": recipients(&draft.to),
            "ccRecipients": recipients(&draft.cc),
        },
        "comment": draft.body,
    });
    send(
        http,
        access_token,
        reqwest::Method::POST,
        &format!("{GRAPH}/me/messages/{}/reply", urlencoding::encode(id)),
        Some(body),
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_message_graph_sent_as_text() {
        let message = message_of(&json!({
            "id": "AAMk1",
            "subject": "Kaffe?",
            "from": { "emailAddress": { "name": "Jakob", "address": "jakob@example.com" } },
            "toRecipients": [{ "emailAddress": { "name": "", "address": "mark@outlook.com" } }],
            "ccRecipients": [],
            "receivedDateTime": "2026-10-15T08:00:00Z",
            "body": { "contentType": "text", "content": "Torsdag?\n\nOn Wed, Mark wrote:\n> Hej" },
            "internetMessageId": "<a@b>",
            "conversationId": "c1",
        }));
        assert_eq!(message.from, "Jakob <jakob@example.com>");
        assert_eq!(message.to, ["mark@outlook.com"]);
        assert_eq!(message.text, "Torsdag?");
        assert_eq!(message.message_id.as_deref(), Some("<a@b>"));
    }

    #[test]
    fn filters_by_sender_date_and_unread_here() {
        let mail = Summary {
            id: "1".into(),
            from: "Jakob <jakob@example.com>".into(),
            subject: "Kaffe".into(),
            received: "2026-10-15T08:00:00Z".parse().unwrap(),
            unread: false,
            excerpt: String::new(),
        };
        let mut search = Search {
            from: Some("jakob".into()),
            limit: 10,
            ..Search::default()
        };
        assert!(keeps(&search, &mail));
        search.unread_only = true;
        assert!(!keeps(&search, &mail));
        search.unread_only = false;
        search.since = Some("2026-10-16".parse().unwrap());
        assert!(!keeps(&search, &mail));
    }
}
