//! iCloud mail: IMAP to read, SMTP to send, both with the account's
//! app-specific password — the same one its calendar uses.
//!
//! Only the inbox is read: IMAP searches one mailbox at a time, and the
//! inbox is where a question about mail is nearly always about. Reading
//! never marks anything read (`BODY.PEEK`).

use crate::caldav::Login;
use crate::error::{Error, Result};
use crate::mail::{Draft, Message, Search, Summary, html_to_text, rfc822, trim_quoted};
use base64::Engine;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use mail_parser::{MessageParser, MimeHeaders};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const IMAP_HOST: &str = "imap.mail.me.com";
const IMAP_PORT: u16 = 993;
const SMTP_HOST: &str = "smtp.mail.me.com";
const SMTP_PORT: u16 = 587;
/// Where iCloud keeps what was sent.
const SENT: &str = "Sent Messages";
const TIMEOUT: Duration = Duration::from_secs(30);

type Tls = tokio_rustls::client::TlsStream<TcpStream>;
type Session = async_imap::Session<Tls>;

fn connector() -> tokio_rustls::TlsConnector {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring supports the default TLS versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(config))
}

fn unreachable(what: &str, e: impl std::fmt::Display) -> Error {
    Error::Http {
        reason: format!("{what}: {e}"),
    }
}

async fn tls(host: &'static str, tcp: TcpStream) -> Result<Tls> {
    let name = rustls::pki_types::ServerName::try_from(host).map_err(|e| unreachable(host, e))?;
    tokio::time::timeout(TIMEOUT, connector().connect(name, tcp))
        .await
        .map_err(|_| unreachable(host, "timed out"))?
        .map_err(|e| unreachable(host, e))
}

async fn imap(login: &Login) -> Result<Session> {
    let tcp = tokio::time::timeout(TIMEOUT, TcpStream::connect((IMAP_HOST, IMAP_PORT)))
        .await
        .map_err(|_| unreachable(IMAP_HOST, "timed out"))?
        .map_err(|e| unreachable(IMAP_HOST, e))?;
    let client = async_imap::Client::new(tls(IMAP_HOST, tcp).await?);
    let mut session = client
        .login(&login.user, &login.password)
        .await
        .map_err(|(e, _)| Error::Provider {
            provider: "iCloud",
            status: 401,
            reason: format!("the Apple ID or app-specific password was refused ({e})"),
        })?;
    session
        .select("INBOX")
        .await
        .map_err(|e| unreachable("iCloud inbox", e))?;
    Ok(session)
}

/// IMAP's `SEARCH` for what a search asks: `ALL` when it asks nothing.
pub fn criteria(search: &Search) -> String {
    let quoted = |s: &str| format!("\"{}\"", s.replace(['\\', '"'], ""));
    let mut parts = Vec::new();
    if let Some(words) = &search.query {
        parts.push(format!("TEXT {}", quoted(words.trim())));
    }
    if let Some(from) = &search.from {
        parts.push(format!("FROM {}", quoted(from.trim())));
    }
    if let Some(since) = search.since {
        parts.push(format!("SINCE {}", since.format("%-d-%b-%Y")));
    }
    if search.unread_only {
        parts.push("UNSEEN".into());
    }
    if parts.is_empty() {
        "ALL".into()
    } else {
        parts.join(" ")
    }
}

pub async fn search(login: &Login, search: &Search) -> Result<Vec<Summary>> {
    let mut session = imap(login).await?;
    let mut uids: Vec<u32> = session
        .uid_search(criteria(search))
        .await
        .map_err(|e| unreachable("iCloud search", e))?
        .into_iter()
        .collect();
    // Newest first: UIDs only grow.
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.truncate(search.limit);
    let mut found = Vec::new();
    if !uids.is_empty() {
        let set: Vec<String> = uids.iter().map(u32::to_string).collect();
        let mut fetches = session
            .uid_fetch(set.join(","), "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER])")
            .await
            .map_err(|e| unreachable("iCloud fetch", e))?;
        while let Some(fetch) = fetches.next().await {
            let fetch = fetch.map_err(|e| unreachable("iCloud fetch", e))?;
            let (Some(uid), Some(header)) = (fetch.uid, fetch.header()) else {
                continue;
            };
            let unread = !fetch
                .flags()
                .any(|f| matches!(f, async_imap::types::Flag::Seen));
            let received = fetch.internal_date().map(|d| d.with_timezone(&Utc));
            if let Some(summary) = summary_of(uid, header, unread, received) {
                found.push(summary);
            }
        }
    }
    let _ = session.logout().await;
    found.sort_by_key(|s| std::cmp::Reverse(s.received));
    Ok(found)
}

fn summary_of(
    uid: u32,
    header: &[u8],
    unread: bool,
    received: Option<DateTime<Utc>>,
) -> Option<Summary> {
    let parsed = MessageParser::default().parse(header)?;
    let received = received.or_else(|| {
        parsed
            .date()
            .and_then(|d| DateTime::from_timestamp(d.to_timestamp(), 0))
    })?;
    Some(Summary {
        id: uid.to_string(),
        from: parsed.from().map(addresses).unwrap_or_default().join(", "),
        subject: parsed.subject().unwrap_or("").to_string(),
        received,
        unread,
        excerpt: String::new(),
    })
}

fn addresses(list: &mail_parser::Address<'_>) -> Vec<String> {
    list.iter()
        .filter_map(|a| {
            let address = a.address()?;
            Some(match a.name() {
                Some(name) if !name.is_empty() && name != address => format!("{name} <{address}>"),
                _ => address.to_string(),
            })
        })
        .collect()
}

pub async fn read(login: &Login, id: &str) -> Result<Message> {
    let uid: u32 = id.parse().map_err(|_| Error::NoSuchMail {
        reference: id.to_string(),
    })?;
    let mut session = imap(login).await?;
    let raw = {
        let mut fetches = session
            .uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")
            .await
            .map_err(|e| unreachable("iCloud fetch", e))?;
        let mut raw = None;
        while let Some(fetch) = fetches.next().await {
            let fetch = fetch.map_err(|e| unreachable("iCloud fetch", e))?;
            if fetch.uid == Some(uid) {
                raw = fetch.body().map(<[u8]>::to_vec);
            }
        }
        raw
    };
    let _ = session.logout().await;
    let raw = raw.ok_or_else(|| Error::NoSuchMail {
        reference: id.to_string(),
    })?;
    message_of(id, &raw).ok_or_else(|| Error::Invalid {
        reason: "that message would not read".into(),
    })
}

pub(crate) fn message_of(id: &str, raw: &[u8]) -> Option<Message> {
    let parsed = MessageParser::default().parse(raw)?;
    let text = parsed
        .body_text(0)
        .map(|t| t.into_owned())
        .or_else(|| parsed.body_html(0).map(|h| html_to_text(&h)))
        .unwrap_or_default();
    let references = parsed
        .references()
        .as_text_list()
        .map(|refs| {
            refs.iter()
                .map(|r| format!("<{r}>"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .or_else(|| parsed.references().as_text().map(|r| format!("<{r}>")));
    Some(Message {
        id: id.to_string(),
        from: parsed.from().map(addresses).unwrap_or_default().join(", "),
        to: parsed.to().map(addresses).unwrap_or_default(),
        cc: parsed.cc().map(addresses).unwrap_or_default(),
        subject: parsed.subject().unwrap_or("").to_string(),
        received: parsed
            .date()
            .and_then(|d| DateTime::from_timestamp(d.to_timestamp(), 0)),
        text: trim_quoted(&text),
        attachments: parsed
            .attachments()
            .filter_map(|a| a.attachment_name().map(str::to_string))
            .collect(),
        message_id: parsed.message_id().map(|m| format!("<{m}>")),
        references,
        thread_id: None,
    })
}

/// Send `draft` from the account, through iCloud's SMTP, then keep a
/// copy in Sent Messages as a mail sent from the phone would be.
pub async fn send(login: &Login, draft: &Draft, reply: Option<&Message>) -> Result<()> {
    let message = rfc822(&login.user, draft, reply);
    let recipients: Vec<String> = draft.to.iter().chain(&draft.cc).map(|a| bare(a)).collect();
    smtp(login, &recipients, &message).await?;
    // The mail went; a copy that did not is not worth failing over.
    match imap(login).await {
        Ok(mut session) => {
            if let Err(e) = session
                .append(SENT, Some("(\\Seen)"), None, message.as_bytes())
                .await
            {
                tracing::warn!("[mail] sent, but not kept in iCloud's Sent Messages: {e}");
            }
            let _ = session.logout().await;
        }
        Err(e) => tracing::warn!("[mail] sent, but not kept in iCloud's Sent Messages: {e}"),
    }
    Ok(())
}

/// The address alone, from "Name <address>".
fn bare(address: &str) -> String {
    address
        .rsplit_once('<')
        .map(|(_, rest)| rest.trim_end_matches('>').trim().to_string())
        .unwrap_or_else(|| address.trim().to_string())
}

/// One reply from an SMTP server: its code and the last line's text.
async fn answer<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<(u16, String)> {
    loop {
        let mut line = String::new();
        let n = tokio::time::timeout(TIMEOUT, reader.read_line(&mut line))
            .await
            .map_err(|_| unreachable(SMTP_HOST, "timed out"))?
            .map_err(|e| unreachable(SMTP_HOST, e))?;
        if n == 0 {
            return Err(unreachable(SMTP_HOST, "the server hung up"));
        }
        let code: u16 = line.get(..3).and_then(|c| c.parse().ok()).ok_or_else(|| {
            unreachable(SMTP_HOST, format!("a reply that made no sense: {line:?}"))
        })?;
        // "250-" is a line of a longer reply; "250 " is its last.
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok((code, line[3..].trim().to_string()));
        }
    }
}

async fn expect<S>(stream: &mut BufReader<S>, command: Option<&str>, want: u16) -> Result<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    if let Some(command) = command {
        stream
            .get_mut()
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .map_err(|e| unreachable(SMTP_HOST, e))?;
    }
    let (code, text) = answer(stream).await?;
    if code != want {
        let shown = command
            .map(|c| c.split_whitespace().next().unwrap_or(c))
            .unwrap_or("greeting");
        return Err(Error::Provider {
            provider: "iCloud",
            status: code,
            reason: format!("{shown}: {text}"),
        });
    }
    Ok(text)
}

/// The message's lines with a leading dot doubled, ended by a lone dot.
pub(crate) fn dot_stuffed(message: &str) -> String {
    let mut out = String::with_capacity(message.len() + 8);
    for line in message.split("\r\n") {
        if line.starts_with('.') {
            out.push('.');
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    while out.ends_with("\r\n\r\n") {
        out.truncate(out.len() - 2);
    }
    out.push_str(".\r\n");
    out
}

async fn smtp(login: &Login, recipients: &[String], message: &str) -> Result<()> {
    let tcp = tokio::time::timeout(TIMEOUT, TcpStream::connect((SMTP_HOST, SMTP_PORT)))
        .await
        .map_err(|_| unreachable(SMTP_HOST, "timed out"))?
        .map_err(|e| unreachable(SMTP_HOST, e))?;
    let mut plain = BufReader::new(tcp);
    expect(&mut plain, None, 220).await?;
    expect(&mut plain, Some("EHLO niles"), 250).await?;
    expect(&mut plain, Some("STARTTLS"), 220).await?;
    let mut secure = BufReader::new(tls(SMTP_HOST, plain.into_inner()).await?);
    expect(&mut secure, Some("EHLO niles"), 250).await?;
    let credentials = base64::engine::general_purpose::STANDARD
        .encode(format!("\0{}\0{}", login.user, login.password));
    expect(&mut secure, Some(&format!("AUTH PLAIN {credentials}")), 235)
        .await
        .map_err(|e| match e {
            Error::Provider { status: 535, .. } => Error::Provider {
                provider: "iCloud",
                status: 535,
                reason: "the Apple ID or app-specific password was refused".into(),
            },
            other => other,
        })?;
    expect(
        &mut secure,
        Some(&format!("MAIL FROM:<{}>", login.user)),
        250,
    )
    .await?;
    for to in recipients {
        expect(&mut secure, Some(&format!("RCPT TO:<{to}>")), 250).await?;
    }
    expect(&mut secure, Some("DATA"), 354).await?;
    secure
        .get_mut()
        .write_all(dot_stuffed(message).as_bytes())
        .await
        .map_err(|e| unreachable(SMTP_HOST, e))?;
    expect(&mut secure, None, 250).await?;
    let _ = expect(&mut secure, Some("QUIT"), 221).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_the_inbox_in_imap_words() {
        let all = Search {
            limit: 10,
            ..Search::default()
        };
        assert_eq!(criteria(&all), "ALL");
        let some = Search {
            query: Some("school \"trip\"".into()),
            from: Some("Jakob".into()),
            since: Some("2026-10-05".parse().unwrap()),
            unread_only: true,
            limit: 10,
        };
        assert_eq!(
            criteria(&some),
            "TEXT \"school trip\" FROM \"Jakob\" SINCE 5-Oct-2026 UNSEEN"
        );
    }

    #[test]
    fn reads_a_message_from_its_raw_form() {
        let raw = concat!(
            "From: Jakob <jakob@example.com>\r\n",
            "To: mark@icloud.com\r\n",
            "Subject: =?UTF-8?B?S2FmZmUgcMOl?=\r\n",
            "Date: Thu, 15 Oct 2026 08:00:00 +0000\r\n",
            "Message-ID: <a@x>\r\n",
            "References: <z@x>\r\n",
            "Content-Type: multipart/mixed; boundary=b\r\n\r\n",
            "--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n",
            "Torsdag?\r\n\r\nOn Wed, Mark wrote:\r\n> Hej\r\n",
            "--b\r\nContent-Type: application/pdf; name=menu.pdf\r\n",
            "Content-Disposition: attachment; filename=menu.pdf\r\n\r\nJVBERi0=\r\n",
            "--b--\r\n",
        );
        let message = message_of("42", raw.as_bytes()).unwrap();
        assert_eq!(message.from, "Jakob <jakob@example.com>");
        assert_eq!(message.subject, "Kaffe på");
        assert_eq!(message.text, "Torsdag?");
        assert_eq!(message.attachments, ["menu.pdf"]);
        assert_eq!(message.message_id.as_deref(), Some("<a@x>"));
        assert_eq!(message.references.as_deref(), Some("<z@x>"));
    }

    #[test]
    fn doubles_a_leading_dot_and_ends_with_one() {
        assert_eq!(
            dot_stuffed("Subject: x\r\n\r\n.hidden\r\nok\r\n"),
            "Subject: x\r\n\r\n..hidden\r\nok\r\n.\r\n"
        );
    }

    #[tokio::test]
    async fn reads_a_reply_that_runs_to_several_lines() {
        let mut reply =
            BufReader::new(&b"250-smtp.mail.me.com\r\n250-STARTTLS\r\n250 8BITMIME\r\n"[..]);
        assert_eq!(answer(&mut reply).await.unwrap(), (250, "8BITMIME".into()));
    }

    #[test]
    fn sends_to_the_address_alone() {
        assert_eq!(bare("Jakob <jakob@example.com>"), "jakob@example.com");
        assert_eq!(bare(" jakob@example.com "), "jakob@example.com");
    }
}
