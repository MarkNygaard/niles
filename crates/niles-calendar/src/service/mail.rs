//! A person's mail, across every account of theirs that serves it.
//!
//! As with events, a message is named to the model by a short reference
//! (`m1`, `m2`) that only means anything to the person whose list it
//! came from: the model cannot read somebody else's mail by guessing an
//! id.

use super::{Apps, Calendars};
use crate::error::{Error, Result};
use crate::mail::{Draft, MAX_RESULTS, Message, Search, Summary};
use crate::model::{Connection, MailAccess};

pub(super) struct KnownMail {
    reference: String,
    connection: String,
    id: String,
}

/// A message as a search lists it, under the account it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct MailListed {
    pub reference: String,
    /// The account's name: "Private", "Work".
    pub account: String,
    pub summary: Summary,
}

/// A message, read.
#[derive(Debug, Clone, PartialEq)]
pub struct MailRead {
    pub reference: String,
    pub account: String,
    pub message: Message,
}

impl Calendars {
    /// The accounts of `person`'s that serve Mail.
    pub async fn mail_accounts(&self, person: &str) -> Result<Vec<Connection>> {
        Ok(self
            .store
            .list(person)
            .await?
            .into_iter()
            .filter(|c| c.mail.is_some())
            .collect())
    }

    /// Messages matching `search`, every mail account of `person`'s
    /// merged, newest first. An account that fails is said in the second
    /// list rather than hiding the others.
    pub async fn mail_search(
        &self,
        apps: &Apps,
        person: &str,
        search: &Search,
    ) -> Result<(Vec<MailListed>, Vec<String>)> {
        let limit = search.limit.clamp(1, MAX_RESULTS);
        let search = Search {
            limit,
            ..search.clone()
        };
        let mut found: Vec<(Connection, Summary)> = Vec::new();
        let mut problems = Vec::new();
        for account in self.mail_accounts(person).await? {
            let listed = match self.auth(apps, &account).await {
                Ok(auth) => self.backend.mail_search(&account, &auth, &search).await,
                Err(e) => Err(e),
            };
            match listed {
                Ok(listed) => found.extend(listed.into_iter().map(|s| (account.clone(), s))),
                Err(e) => problems.push(format!("{}: {e}", account.name)),
            }
        }
        found.sort_by_key(|(_, s)| std::cmp::Reverse(s.received));
        found.truncate(limit);
        let mut known = self.lock_known_mail();
        let shown = known.entry(person.to_string()).or_default();
        shown.clear();
        let listed = found
            .into_iter()
            .enumerate()
            .map(|(i, (account, summary))| {
                let reference = format!("m{}", i + 1);
                shown.push(KnownMail {
                    reference: reference.clone(),
                    connection: account.id.clone(),
                    id: summary.id.clone(),
                });
                MailListed {
                    reference,
                    account: account.name,
                    summary,
                }
            })
            .collect();
        Ok((listed, problems))
    }

    /// One message `person` was shown, by its reference.
    pub async fn mail_read(&self, apps: &Apps, person: &str, reference: &str) -> Result<MailRead> {
        let (account, id) = self.recall_mail(person, reference).await?;
        let auth = self.auth(apps, &account).await?;
        let message = self.backend.mail_read(&account, &auth, &id).await?;
        Ok(MailRead {
            reference: reference.to_string(),
            account: account.name,
            message,
        })
    }

    /// The account a mail would go from: the one that received the
    /// message it replies to, the one named, or the only one that can
    /// send. Refused when it cannot send, or when the choice is not
    /// clear.
    pub async fn mail_sender(
        &self,
        person: &str,
        account: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<Connection> {
        let chosen = if let Some(reference) = reply_to {
            self.recall_mail(person, reference).await?.0
        } else {
            let all = self.mail_accounts(person).await?;
            match account {
                Some(name) => all
                    .into_iter()
                    .find(|c| c.name.eq_ignore_ascii_case(name.trim()))
                    .ok_or_else(|| Error::NoSuchConnection {
                        name: name.to_string(),
                    })?,
                None => {
                    let mut senders: Vec<Connection> = all
                        .into_iter()
                        .filter(|c| c.mail == Some(MailAccess::Send))
                        .collect();
                    match senders.len() {
                        1 => senders.remove(0),
                        0 => {
                            return Err(Error::Invalid {
                                reason: "none of this person's mail accounts may send; that is chosen on their profile".into(),
                            });
                        }
                        _ => {
                            let names: Vec<String> =
                                senders.iter().map(|c| c.name.clone()).collect();
                            return Err(Error::Invalid {
                                reason: format!(
                                    "which account should it go from: {}?",
                                    names.join(" or ")
                                ),
                            });
                        }
                    }
                }
            }
        };
        if chosen.mail != Some(MailAccess::Send) {
            return Err(Error::CannotSend { name: chosen.name });
        }
        Ok(chosen)
    }

    /// The message a reply answers, read again for its thread.
    pub async fn mail_original(
        &self,
        apps: &Apps,
        person: &str,
        reference: &str,
    ) -> Result<Message> {
        Ok(self.mail_read(apps, person, reference).await?.message)
    }

    /// Send `draft` from the account `account_id`, as approved. Only
    /// ever called with what the person was shown.
    pub async fn mail_send(
        &self,
        apps: &Apps,
        person: &str,
        account_id: &str,
        draft: &Draft,
        reply: Option<&Message>,
    ) -> Result<()> {
        let account = self
            .mail_accounts(person)
            .await?
            .into_iter()
            .find(|c| c.id == account_id)
            .ok_or_else(|| Error::NoSuchConnection {
                name: account_id.to_string(),
            })?;
        if account.mail != Some(MailAccess::Send) {
            return Err(Error::CannotSend { name: account.name });
        }
        let auth = self.auth(apps, &account).await?;
        self.backend.mail_send(&account, &auth, draft, reply).await
    }

    async fn recall_mail(&self, person: &str, reference: &str) -> Result<(Connection, String)> {
        let (connection, id) = {
            let known = self.lock_known_mail();
            let found = known
                .get(person)
                .and_then(|all| all.iter().find(|k| k.reference == reference.trim()))
                .ok_or_else(|| Error::NoSuchMail {
                    reference: reference.to_string(),
                })?;
            (found.connection.clone(), found.id.clone())
        };
        let account = self
            .mail_accounts(person)
            .await?
            .into_iter()
            .find(|c| c.id == connection)
            .ok_or_else(|| Error::NoSuchMail {
                reference: reference.to_string(),
            })?;
        Ok((account, id))
    }

    fn lock_known_mail(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, Vec<KnownMail>>> {
        self.known_mail.lock().unwrap_or_else(|e| e.into_inner())
    }
}
