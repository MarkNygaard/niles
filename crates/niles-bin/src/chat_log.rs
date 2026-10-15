//! Every conversation with Niles, spoken or typed, kept for the person
//! who had it, so the app's chat shows what was said to a satellite as
//! well as what was typed — and nobody else's.
//!
//! A conversation ends when nobody has said anything for a few minutes;
//! the next turn starts a new one. A spoken turn is somebody's only when
//! the voice was recognised: what an unknown voice said belongs to no
//! one's chat.

use niles_api::chat::{Conversation, Summary, Turn, Where};
use niles_db::PostgresConversationLog;

/// How long a conversation waits for its next turn.
const GAP_SECONDS: i64 = 5 * 60;

/// How much of the first thing said names a conversation in the list.
const TITLE_CHARS: usize = 60;

pub(crate) struct ChatLog {
    table: PostgresConversationLog,
    /// One write at a time: two turns arriving together would each find
    /// no conversation going and start one apiece.
    order: tokio::sync::Mutex<()>,
}

/// Whose conversations these are. The voice an address is linked to,
/// when it is, so a spoken turn and a typed one meet in the same chat.
/// The training data is kept under the same key.
pub(crate) fn person(speaker: Option<&str>, email: &str) -> String {
    niles_training::person_key(speaker, email)
}

impl ChatLog {
    pub(crate) fn new(table: PostgresConversationLog) -> Self {
        Self {
            table,
            order: tokio::sync::Mutex::new(()),
        }
    }

    /// The conversation `into` names, with its turns: `None` for one yet
    /// to be started. An id that is not theirs is an error, not a new
    /// conversation — a reply meant for one must not land in another.
    pub(crate) async fn target(
        &self,
        person: &str,
        into: Where,
    ) -> Result<Option<(i64, Vec<Turn>)>, String> {
        let id = match into {
            Where::New => return Ok(None),
            Where::Continue(id) => id,
            Where::Latest => match self.table.active(person, GAP_SECONDS).await? {
                Some(id) => id,
                None => return Ok(None),
            },
        };
        match self.conversation(person, id).await? {
            Some(c) => Ok(Some((id, c.turns))),
            None => Err("no such conversation".into()),
        }
    }

    /// Add `turn` to conversation `into`, or to a new one. Answers which
    /// it went into.
    pub(crate) async fn record(
        &self,
        person: &str,
        into: Option<i64>,
        turn: &Turn,
    ) -> Result<Option<i64>, String> {
        let json = serde_json::to_string(turn).map_err(|e| e.to_string())?;
        let spoken = turn.via == "voice";
        let _order = self.order.lock().await;
        self.table
            .append(person, into, &title(&turn.said), &json, spoken)
            .await
    }

    /// A spoken turn: into the conversation going on, or a new one.
    pub(crate) async fn record_spoken(&self, person: &str, turn: &Turn) {
        let _order = self.order.lock().await;
        let outcome = async {
            let into = self.table.active(person, GAP_SECONDS).await?;
            let json = serde_json::to_string(turn).map_err(|e| e.to_string())?;
            self.table
                .append(person, into, &title(&turn.said), &json, true)
                .await
        }
        .await;
        if let Err(e) = outcome {
            tracing::warn!("[chat-log] a spoken turn was not kept: {e}");
        }
    }

    pub(crate) async fn list(&self, person: &str) -> Result<Vec<Summary>, String> {
        Ok(self
            .table
            .list(person)
            .await?
            .into_iter()
            .map(|r| Summary {
                id: r.id,
                title: r.title,
                started_at: r.started_at,
                last_at: r.last_at,
                turns: r.turns,
                spoken: r.spoken,
            })
            .collect())
    }

    pub(crate) async fn conversation(
        &self,
        person: &str,
        id: i64,
    ) -> Result<Option<Conversation>, String> {
        let Some((row, turns)) = self.table.get(person, id).await? else {
            return Ok(None);
        };
        let turns: Vec<Turn> = serde_json::from_str(&turns)
            .map_err(|e| format!("conversation {id} would not parse: {e}"))?;
        Ok(Some(Conversation {
            id: row.id,
            title: row.title,
            started_at: row.started_at,
            last_at: row.last_at,
            turns,
        }))
    }

    pub(crate) async fn delete(&self, person: &str, id: i64) -> Result<bool, String> {
        self.table.delete(person, id).await
    }
}

/// The first thing said, cut at a word if it runs long.
fn title(said: &str) -> String {
    let said = said.trim();
    if said.chars().count() <= TITLE_CHARS {
        return said.to_string();
    }
    let cut: String = said.chars().take(TITLE_CHARS).collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > TITLE_CHARS / 2 => &cut[..space],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end_matches([',', '.', ' ']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_and_the_address_linked_to_it_are_the_same_person() {
        assert_eq!(
            person(Some("majse"), "majse@example.com"),
            person(Some("majse"), "someone-else@example.com")
        );
        assert_ne!(
            person(None, "mark@example.com"),
            person(None, "majse@example.com")
        );
    }

    #[test]
    fn a_short_first_line_is_the_title_as_said() {
        assert_eq!(title("  turn on the lights "), "turn on the lights");
    }

    #[test]
    fn a_long_first_line_is_cut_at_a_word() {
        let t = title(
            "every weekday at half past six turn on the bedroom light and then the kitchen one too",
        );
        assert!(t.ends_with('…'), "{t}");
        assert!(t.chars().count() <= TITLE_CHARS + 1, "{t}");
        assert!(!t.contains("kitch"), "{t}");
    }
}
