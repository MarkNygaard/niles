//! The mail tools: the speaker's own mail, read, and — where an account
//! may send — written, but never sent without the person's yes in the
//! app. See "Mail" and "Approvals" in ARCHITECTURE.md.
//!
//! Whose mail comes from [`profile::SPEAKER`], as for calendars: never
//! from an argument.

use crate::approvals::{self, Approvals};
use crate::calendar::Shared;
use crate::profile;
use chrono::{NaiveDate, Utc};
use niles_api::chat::{ApprovalCard, ApprovalStatus, CardField};
use niles_calendar::Provider;
use niles_calendar::mail::{self, Draft, Message, Search};
use niles_tools::{Error, Result, Tool, ToolDescriptor, ToolRegistry};
use serde_json::{Value, json};
use std::sync::Arc;

struct Mail {
    shared: Arc<Shared>,
    approvals: Arc<Approvals>,
}

impl Mail {
    fn person(&self, tool: &str) -> Result<String> {
        profile::SPEAKER
            .try_with(Clone::clone)
            .ok()
            .flatten()
            .ok_or_else(|| {
                invalid(
                    tool,
                    "I don't know whose voice this is, so I can't tell whose mail to look at"
                        .into(),
                )
            })
    }

    fn local(&self, at: chrono::DateTime<Utc>) -> String {
        at.with_timezone(&self.shared.tz)
            .format("%A %Y-%m-%d %H:%M")
            .to_string()
    }
}

fn invalid(tool: &str, reason: String) -> Error {
    Error::InvalidArgs {
        tool: tool.into(),
        reason,
    }
}

fn failed(tool: &str, e: niles_calendar::Error) -> Error {
    invalid(tool, e.to_string())
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// An address list, whether the model sent a string or an array.
fn address_list(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .flat_map(mail::addresses)
            .collect(),
        Some(Value::String(list)) => mail::addresses(list),
        _ => Vec::new(),
    }
}

const NOT_CONNECTED: &str =
    "This person has not connected their mail. They do that in their profile in the Niles app.";

pub struct MailSearch(Arc<Mail>);

#[async_trait::async_trait]
impl Tool for MailSearch {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "mail_search".into(),
            description: "The mail of the person speaking — every mail account they connected, \
                together: 'any mail from Jakob today', 'what did the school write', 'do I have \
                unread mail'. Only theirs. Without words it lists the inbox; with words it \
                searches the whole mailbox. Each message has a ref (m1, m2…) to read or reply \
                to it by. Say who, what about, and when, briefly."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words to search for. Leave out to list the inbox." },
                    "from": { "type": "string", "description": "A sender's name or address." },
                    "since": { "type": "string", "description": "Only mail from this date on, YYYY-MM-DD." },
                    "unread_only": { "type": "boolean" },
                    "limit": { "type": "integer", "description": "At most this many, 1-20. Default 10." }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "mail_search";
        let m = &self.0;
        let person = m.person(TOOL)?;
        let since =
            match text(&args, "since") {
                Some(day) => Some(NaiveDate::parse_from_str(&day, "%Y-%m-%d").map_err(|_| {
                    invalid(TOOL, format!("since {day:?} is not a date (YYYY-MM-DD)"))
                })?),
                None => None,
            };
        let search = Search {
            query: text(&args, "query"),
            from: text(&args, "from"),
            since,
            unread_only: args
                .get("unread_only")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            limit: args
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(10, |n| n as usize),
        };
        let calendars = &m.shared.calendars;
        let accounts = calendars
            .mail_accounts(&person)
            .await
            .map_err(|e| failed(TOOL, e))?;
        if accounts.is_empty() {
            return Ok(json!({ "messages": [], "note": NOT_CONNECTED }));
        }
        let (listed, problems) = calendars
            .mail_search(&m.shared.apps(), &person, &search)
            .await
            .map_err(|e| failed(TOOL, e))?;
        Ok(json!({
            "messages": listed.iter().map(|l| json!({
                "ref": l.reference,
                "account": l.account,
                "from": l.summary.from,
                "subject": l.summary.subject,
                "received": m.local(l.summary.received),
                "unread": l.summary.unread,
                "excerpt": l.summary.excerpt,
            })).collect::<Vec<_>>(),
            "problems": problems,
        }))
    }
}

pub struct MailRead(Arc<Mail>);

#[async_trait::async_trait]
impl Tool for MailRead {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "mail_read".into(),
            description: "One message the person was shown by mail_search, by its ref: who it is \
                from and to, and its text (earlier quoted replies left out). Attachments are \
                named, never opened. Read the text, then answer what was asked; do not read \
                a long mail out word for word."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "The message's ref from mail_search: m1, m2…" }
                },
                "required": ["ref"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "mail_read";
        let m = &self.0;
        let person = m.person(TOOL)?;
        let reference =
            text(&args, "ref").ok_or_else(|| invalid(TOOL, "ref is required".into()))?;
        let read = m
            .shared
            .calendars
            .mail_read(&m.shared.apps(), &person, &reference)
            .await
            .map_err(|e| failed(TOOL, e))?;
        let message = &read.message;
        Ok(json!({
            "ref": read.reference,
            "account": read.account,
            "from": message.from,
            "to": message.to,
            "cc": message.cc,
            "subject": message.subject,
            "received": message.received.map(|r| m.local(r)),
            "text": message.text,
            "attachments": message.attachments,
        }))
    }
}

pub struct SendMail(Arc<Mail>);

#[async_trait::async_trait]
impl Tool for SendMail {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "send_mail".into(),
            description: "Write a mail from the person speaking. It is NOT sent: the person sees \
                it exactly as written in the Niles app, and it goes only if they allow it there. \
                Write it fully and as they would: a greeting, the message, their name. To reply, \
                give reply_to (a ref from mail_search) and leave the subject out. When they \
                have more than one account that can send, ask which unless they said."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "to": { "type": "array", "items": { "type": "string" }, "description": "Recipients' addresses." },
                    "cc": { "type": "array", "items": { "type": "string" } },
                    "subject": { "type": "string" },
                    "body": { "type": "string", "description": "The whole text of the mail." },
                    "account": { "type": "string", "description": "The name of the account to send from, when the person has several." },
                    "reply_to": { "type": "string", "description": "The ref of the message this answers." }
                },
                "required": ["to", "body"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        const TOOL: &str = "send_mail";
        let m = &self.0;
        let person = m.person(TOOL)?;
        let Some(asker) = approvals::asker() else {
            return Err(invalid(
                TOOL,
                "a mail waits for its person's yes, and nobody is here to give it".into(),
            ));
        };
        let to = address_list(&args, "to");
        let cc = address_list(&args, "cc");
        if to.is_empty() {
            return Err(invalid(TOOL, "who is it to?".into()));
        }
        if let Some(bad) = to.iter().chain(&cc).find(|a| !mail::plausible(a)) {
            return Err(invalid(
                TOOL,
                format!("{bad:?} is not an email address; ask for it"),
            ));
        }
        let body =
            text(&args, "body").ok_or_else(|| invalid(TOOL, "the mail has no text".into()))?;
        let reply_to = text(&args, "reply_to");
        let account = text(&args, "account");
        let calendars = &m.shared.calendars;
        let sender = calendars
            .mail_sender(&person, account.as_deref(), reply_to.as_deref())
            .await
            .map_err(|e| failed(TOOL, e))?;
        let original = match &reply_to {
            Some(reference) => Some(
                calendars
                    .mail_original(&m.shared.apps(), &person, reference)
                    .await
                    .map_err(|e| failed(TOOL, e))?,
            ),
            None => None,
        };
        let subject = match (&original, text(&args, "subject")) {
            (Some(original), None) => mail::reply_subject(&original.subject),
            (_, Some(subject)) => subject,
            (None, None) => return Err(invalid(TOOL, "the mail needs a subject".into())),
        };
        let draft = Draft {
            to,
            cc,
            subject,
            body,
        };
        let card = card(&sender, &draft);
        let spoken = asker.satellite.is_some();
        // Answered in the app, however it was asked for: a mail is too
        // long to hear and judge, and a misheard yes is worse here than
        // anywhere.
        let in_the_app = approvals::Asker {
            satellite: None,
            ..asker
        };
        let deliver = Arc::new(Deliver {
            shared: m.shared.clone(),
            person,
            account: sender.id.clone(),
            draft: draft.clone(),
            original,
        });
        let held = m.approvals.hold(&in_the_app, deliver, Value::Null, card);
        tracing::info!(
            "[mail] a mail to {} is waiting in the app",
            draft.to.join(", ")
        );
        let note = if spoken {
            "Nothing is sent yet. Say in one short sentence that the mail is written and waiting in the Niles app for them to check and send. Do not offer to send it by voice."
        } else {
            "Nothing is sent yet. Under your reply the person has the mail exactly as written, with Allow once and Deny. Say in one short sentence that it is ready for them to check. Do not repeat the mail."
        };
        Ok(json!({
            "waiting_for_approval": true,
            "shown_to_the_person": held.title,
            "note": note,
        }))
    }
}

/// The card a mail is shown as: drawn as a mail, and only ever allowed
/// once.
fn card(sender: &niles_calendar::Connection, draft: &Draft) -> ApprovalCard {
    let provider = match sender.provider {
        Provider::Microsoft => "Outlook",
        Provider::Google => "Gmail",
        Provider::Icloud => "iCloud",
    };
    let mut fields = vec![CardField {
        label: "To".into(),
        value: draft.to.join(", "),
    }];
    if !draft.cc.is_empty() {
        fields.push(CardField {
            label: "Cc".into(),
            value: draft.cc.join(", "),
        });
    }
    fields.push(CardField {
        label: "Subject".into(),
        value: draft.subject.clone(),
    });
    ApprovalCard {
        id: String::new(),
        title: "Send this email?".into(),
        subtitle: format!("From your {provider}, {}", sender.account),
        icon: Some("mail".into()),
        fields,
        body: Some(draft.body.clone()),
        allow_conversation: false,
        status: ApprovalStatus::Pending,
        outcome: None,
    }
}

/// What an approved mail runs: exactly the draft that was shown, from
/// the account it was shown going from.
struct Deliver {
    shared: Arc<Shared>,
    person: String,
    account: String,
    draft: Draft,
    original: Option<Message>,
}

#[async_trait::async_trait]
impl Tool for Deliver {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "send_mail".into(),
            description: "Send the approved mail.".into(),
            parameters: json!({ "type": "object" }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        self.shared
            .calendars
            .mail_send(
                &self.shared.apps(),
                &self.person,
                &self.account,
                &self.draft,
                self.original.as_ref(),
            )
            .await
            .map_err(|e| Error::Remote(e.to_string()))?;
        Ok(Value::String(format!(
            "Sent to {}.",
            self.draft.to.join(", ")
        )))
    }
}

pub fn register(reg: &mut ToolRegistry, shared: Arc<Shared>, approvals: Arc<Approvals>) {
    let mail = Arc::new(Mail { shared, approvals });
    reg.register(Box::new(MailSearch(mail.clone())));
    reg.register(Box::new(MailRead(mail.clone())));
    reg.register(Box::new(SendMail(mail)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_calendar::{Access, Calendars, Connection, MailAccess, MemoryConnections, Network};

    fn gmail(mail: Option<MailAccess>) -> Connection {
        Connection {
            id: "acc-1".into(),
            person: "mark".into(),
            provider: Provider::Google,
            name: "Private".into(),
            account: "mark@gmail.com".into(),
            access: Access::Read,
            credential: "rt".into(),
            endpoint: None,
            calendar: false,
            mail,
        }
    }

    async fn tools(account: Option<Connection>) -> (Arc<Mail>, Arc<Approvals>) {
        let store = Arc::new(MemoryConnections::new());
        if let Some(account) = account {
            niles_calendar::ConnectionStore::save(store.as_ref(), &account)
                .await
                .unwrap();
        }
        let calendars = Arc::new(Calendars::new(
            store,
            Arc::new(Network::new(reqwest::Client::new())),
        ));
        let shared = Arc::new(Shared {
            calendars,
            config: Arc::new(niles_config::ConfigStore::from_str_in_memory("").unwrap()),
            tz: "Europe/Copenhagen".parse().unwrap(),
        });
        let approvals = Arc::new(Approvals::default());
        (
            Arc::new(Mail {
                shared,
                approvals: approvals.clone(),
            }),
            approvals,
        )
    }

    fn asker(satellite: Option<std::net::SocketAddr>) -> approvals::Asker {
        approvals::Asker {
            person: "person:mark".into(),
            slug: Some("mark".into()),
            satellite,
            conversation: None,
        }
    }

    async fn as_mark<F: std::future::Future>(asker: approvals::Asker, f: F) -> F::Output {
        profile::SPEAKER
            .scope(Some("mark".into()), approvals::ASKER.scope(asker, f))
            .await
    }

    #[tokio::test]
    async fn a_mail_is_written_and_waits_in_the_app_even_when_asked_aloud() {
        let (mail, approvals) = tools(Some(gmail(Some(MailAccess::Send)))).await;
        let started = std::time::Instant::now();
        let kitchen: std::net::SocketAddr = "192.168.1.20:40000".parse().unwrap();
        let said = as_mark(
            asker(Some(kitchen)),
            SendMail(mail).execute(json!({
                "to": ["jakob@example.com"],
                "subject": "Running late",
                "body": "Hi Jakob,\n\nI'll be fifteen minutes late.\n\nMark",
            })),
        )
        .await
        .unwrap();
        assert_eq!(said["waiting_for_approval"], true);
        assert!(
            said["note"]
                .as_str()
                .unwrap()
                .contains("Do not offer to send it by voice")
        );
        // Not answerable at the satellite.
        assert!(approvals.spoken(kitchen).is_none());
        let card = approvals
            .for_turn("person:mark", started)
            .expect("a card in the app");
        assert_eq!(card.title, "Send this email?");
        assert_eq!(card.subtitle, "From your Gmail, mark@gmail.com");
        assert_eq!(card.fields[0].value, "jakob@example.com");
        assert_eq!(card.fields[1].value, "Running late");
        assert!(card.body.unwrap().contains("fifteen minutes"));
        assert!(!card.allow_conversation, "a mail is approved one at a time");
    }

    #[tokio::test]
    async fn a_read_only_account_writes_nothing() {
        let (mail, approvals) = tools(Some(gmail(Some(MailAccess::Read)))).await;
        let err = as_mark(
            asker(None),
            SendMail(mail).execute(json!({
                "to": ["jakob@example.com"], "subject": "Hi", "body": "Hi",
            })),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("may send"), "{err}");
        assert!(
            approvals
                .for_turn(
                    "person:mark",
                    std::time::Instant::now() - std::time::Duration::from_secs(5)
                )
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_address_that_is_not_one_is_asked_for() {
        let (mail, _) = tools(Some(gmail(Some(MailAccess::Send)))).await;
        let err = as_mark(
            asker(None),
            SendMail(mail).execute(json!({ "to": "Jakob", "subject": "Hi", "body": "Hi" })),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("is not an email address"), "{err}");
    }

    #[tokio::test]
    async fn somebody_without_mail_is_told_where_to_connect_it() {
        let (mail, _) = tools(None).await;
        let found = as_mark(asker(None), MailSearch(mail).execute(json!({})))
            .await
            .unwrap();
        assert!(found["note"].as_str().unwrap().contains("profile"));
    }

    #[tokio::test]
    async fn an_unknown_voice_has_no_mail() {
        let (mail, _) = tools(Some(gmail(Some(MailAccess::Send)))).await;
        let err = MailSearch(mail).execute(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("whose mail"), "{err}");
    }
}
