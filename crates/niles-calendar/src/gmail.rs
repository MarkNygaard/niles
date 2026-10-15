//! Mail through the Gmail API: Gmail and Google Workspace.

use crate::error::Result;
use crate::google::send;
use crate::graph_mail::keeps;
use crate::mail::{Draft, Message, Search, Summary, html_to_text, rfc822, trim_quoted};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

const GMAIL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// Gmail's own search words for `search`: the inbox unless words were
/// given, in which case the whole mailbox.
pub fn query_of(search: &Search) -> String {
    let mut parts: Vec<String> = Vec::new();
    match &search.query {
        Some(words) => parts.push(words.trim().to_string()),
        None => parts.push("in:inbox".into()),
    }
    if let Some(from) = &search.from {
        parts.push(format!("from:({})", from.trim()));
    }
    if let Some(since) = search.since {
        parts.push(format!("after:{}", since.format("%Y/%m/%d")));
    }
    if search.unread_only {
        parts.push("is:unread".into());
    }
    parts.join(" ")
}

pub async fn search(
    http: &reqwest::Client,
    access_token: &str,
    search: &Search,
) -> Result<Vec<Summary>> {
    let list = send(
        http,
        access_token,
        reqwest::Method::GET,
        &format!(
            "{GMAIL}/messages?q={}&maxResults={}",
            urlencoding::encode(&query_of(search)),
            search.limit
        ),
        None,
    )
    .await?;
    let ids: Vec<String> = list
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut found = Vec::with_capacity(ids.len());
    for id in ids {
        let meta = send(
            http,
            access_token,
            reqwest::Method::GET,
            &format!(
                "{GMAIL}/messages/{id}?format=metadata&metadataHeaders=From&metadataHeaders=Subject"
            ),
            None,
        )
        .await?;
        if let Some(summary) = summary_of(&meta)
            && keeps(search, &summary)
        {
            found.push(summary);
        }
    }
    found.sort_by_key(|s| std::cmp::Reverse(s.received));
    Ok(found)
}

fn header(value: &Value, name: &str) -> Option<String> {
    value
        .pointer("/payload/headers")
        .and_then(Value::as_array)?
        .iter()
        .find(|h| {
            h.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|h| h.get("value").and_then(Value::as_str))
        .map(str::to_string)
}

fn received(value: &Value) -> Option<DateTime<Utc>> {
    let ms: i64 = value.get("internalDate")?.as_str()?.parse().ok()?;
    DateTime::from_timestamp_millis(ms)
}

fn summary_of(value: &Value) -> Option<Summary> {
    Some(Summary {
        id: value.get("id")?.as_str()?.to_string(),
        from: header(value, "From").unwrap_or_default(),
        subject: header(value, "Subject").unwrap_or_default(),
        received: received(value)?,
        unread: value
            .get("labelIds")
            .and_then(Value::as_array)
            .is_some_and(|l| l.iter().any(|x| x.as_str() == Some("UNREAD"))),
        excerpt: value
            .get("snippet")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

pub async fn read(http: &reqwest::Client, access_token: &str, id: &str) -> Result<Message> {
    let value = send(
        http,
        access_token,
        reqwest::Method::GET,
        &format!("{GMAIL}/messages/{}?format=full", urlencoding::encode(id)),
        None,
    )
    .await?;
    Ok(message_of(&value))
}

pub(crate) fn message_of(value: &Value) -> Message {
    let list = |name: &str| -> Vec<String> {
        header(value, name)
            .map(|h| crate::mail::addresses(&h))
            .unwrap_or_default()
    };
    let payload = value.get("payload").cloned().unwrap_or(Value::Null);
    let mut plain = None;
    let mut html = None;
    let mut attachments = Vec::new();
    walk(&payload, &mut plain, &mut html, &mut attachments);
    let text = plain
        .or_else(|| html.map(|h| html_to_text(&h)))
        .unwrap_or_default();
    Message {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        from: header(value, "From").unwrap_or_default(),
        to: list("To"),
        cc: list("Cc"),
        subject: header(value, "Subject").unwrap_or_default(),
        received: received(value),
        text: trim_quoted(&text),
        attachments,
        message_id: header(value, "Message-ID").or_else(|| header(value, "Message-Id")),
        references: header(value, "References"),
        thread_id: value
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// The first plain and HTML parts, and every attachment's name.
fn walk(
    part: &Value,
    plain: &mut Option<String>,
    html: &mut Option<String>,
    attachments: &mut Vec<String>,
) {
    let mime = part.get("mimeType").and_then(Value::as_str).unwrap_or("");
    let filename = part.get("filename").and_then(Value::as_str).unwrap_or("");
    if !filename.is_empty() {
        attachments.push(filename.to_string());
    } else if let Some(data) = part.pointer("/body/data").and_then(Value::as_str) {
        let decoded = decode(data);
        if mime.eq_ignore_ascii_case("text/plain") && plain.is_none() {
            *plain = decoded;
        } else if mime.eq_ignore_ascii_case("text/html") && html.is_none() {
            *html = decoded;
        }
    }
    for child in part
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        walk(child, plain, html, attachments);
    }
}

fn decode(data: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data.trim_end_matches('='))
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Send `draft` from `from`, in `reply`'s thread when it answers one.
pub async fn send_mail(
    http: &reqwest::Client,
    access_token: &str,
    from: &str,
    draft: &Draft,
    reply: Option<&Message>,
) -> Result<()> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(rfc822(from, draft, reply).as_bytes());
    let mut body = json!({ "raw": raw });
    if let Some(thread) = reply.and_then(|m| m.thread_id.as_deref()) {
        body["threadId"] = json!(thread);
    }
    send(
        http,
        access_token,
        reqwest::Method::POST,
        &format!("{GMAIL}/messages/send"),
        Some(body),
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_gmail_in_its_own_words() {
        let search = Search {
            from: Some("jakob".into()),
            since: Some("2026-10-01".parse().unwrap()),
            unread_only: true,
            limit: 10,
            ..Search::default()
        };
        assert_eq!(
            query_of(&search),
            "in:inbox from:(jakob) after:2026/10/01 is:unread"
        );
        let words = Search {
            query: Some("school trip".into()),
            limit: 10,
            ..Search::default()
        };
        assert_eq!(query_of(&words), "school trip");
    }

    #[test]
    fn reads_the_plain_part_and_names_attachments() {
        let plain = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode("Ses torsdag.\n\nDen 1. okt. skrev Jakob:\n> Kaffe?");
        let value = json!({
            "id": "m1",
            "threadId": "t1",
            "internalDate": "1792051200000",
            "payload": {
                "mimeType": "multipart/mixed",
                "headers": [
                    { "name": "From", "value": "Jakob <jakob@example.com>" },
                    { "name": "To", "value": "mark@gmail.com" },
                    { "name": "Subject", "value": "Kaffe" },
                    { "name": "Message-ID", "value": "<x@y>" },
                ],
                "parts": [
                    { "mimeType": "multipart/alternative", "parts": [
                        { "mimeType": "text/plain", "body": { "data": plain } },
                        { "mimeType": "text/html", "body": { "data": "PGI-aGk8L2I-" } },
                    ]},
                    { "mimeType": "application/pdf", "filename": "menu.pdf", "body": { "attachmentId": "a1" } },
                ],
            },
        });
        let message = message_of(&value);
        assert_eq!(message.text, "Ses torsdag.");
        assert_eq!(message.attachments, ["menu.pdf"]);
        assert_eq!(message.to, ["mark@gmail.com"]);
        assert_eq!(message.message_id.as_deref(), Some("<x@y>"));
        assert_eq!(message.thread_id.as_deref(), Some("t1"));
    }
}
