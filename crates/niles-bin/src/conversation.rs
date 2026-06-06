//! Short-term conversation memory: a small rolling buffer of recent
//! voice turns, kept per room, so the LLM can resolve follow-ups like
//! "turn it off again" or "make it warmer" against what was just said.
//!
//! This is the in-context half of ARCHITECTURE.md Phase 12's
//! "conversation memory (short-term in context, long-term in Postgres)".
//! Long-term persistence is a separate, later concern — here we only
//! hold the last few turns in memory, scoped to one room and expired by
//! a short idle TTL so an unrelated command minutes later starts fresh.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use niles_core::RoomName;
use niles_llm::Message;

/// Production tuning: how many exchanges to keep, and how long after the
/// last turn a follow-up still counts as the same conversation.
const DEFAULT_MAX_TURNS: usize = 4;
const DEFAULT_TTL: Duration = Duration::from_secs(180);

/// One completed exchange: what the user said and how niles replied.
#[derive(Clone)]
struct Turn {
    user: String,
    assistant: String,
}

struct RoomHistory {
    turns: VecDeque<Turn>,
    last_at: Instant,
}

/// Recent turns keyed by origin room. Satellites with no room mapping
/// share a single bucket (the `None` key), which is fine for a
/// single-room install and keeps unmapped devices from bleeding context
/// into a named room.
pub(crate) struct ConversationMemory {
    by_room: Mutex<HashMap<Option<RoomName>, RoomHistory>>,
    max_turns: usize,
    ttl: Duration,
}

impl Default for ConversationMemory {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_TURNS, DEFAULT_TTL)
    }
}

impl ConversationMemory {
    pub(crate) fn new(max_turns: usize, ttl: Duration) -> Self {
        Self {
            by_room: Mutex::new(HashMap::new()),
            max_turns,
            ttl,
        }
    }

    /// Prior turns for `room` as alternating user/assistant messages,
    /// oldest first — ready to splice between the system prompt and the
    /// current utterance. Empty when there's no live history (nothing
    /// recorded, or the last turn is older than the TTL).
    pub(crate) fn recent_messages(&self, room: Option<&RoomName>) -> Vec<Message> {
        self.recent_messages_at(room, Instant::now())
    }

    /// Record a completed exchange so the next turn in the same room can
    /// see it. A turn arriving after the TTL starts a fresh history.
    pub(crate) fn record(&self, room: Option<&RoomName>, user: &str, assistant: &str) {
        self.record_at(room, user, assistant, Instant::now());
    }

    fn recent_messages_at(&self, room: Option<&RoomName>, now: Instant) -> Vec<Message> {
        let map = self.by_room.lock().unwrap_or_else(|e| e.into_inner());
        let Some(history) = map.get(&room.cloned()) else {
            return Vec::new();
        };
        if now.duration_since(history.last_at) > self.ttl {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(history.turns.len() * 2);
        for turn in &history.turns {
            out.push(Message::User {
                content: turn.user.clone(),
            });
            out.push(Message::Assistant {
                content: Some(turn.assistant.clone()),
                tool_calls: None,
            });
        }
        out
    }

    fn record_at(&self, room: Option<&RoomName>, user: &str, assistant: &str, now: Instant) {
        let mut map = self.by_room.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(room.cloned()).or_insert_with(|| RoomHistory {
            turns: VecDeque::new(),
            last_at: now,
        });
        // An idle gap past the TTL means this is a new conversation, not a
        // follow-up — drop the stale turns rather than splicing them in.
        if now.duration_since(entry.last_at) > self.ttl {
            entry.turns.clear();
        }
        entry.turns.push_back(Turn {
            user: user.to_string(),
            assistant: assistant.to_string(),
        });
        while entry.turns.len() > self.max_turns {
            entry.turns.pop_front();
        }
        entry.last_at = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(name: &str) -> RoomName {
        RoomName::parse(name).unwrap()
    }

    fn texts(messages: &[Message]) -> Vec<&str> {
        messages
            .iter()
            .map(|m| match m {
                Message::User { content } => content.as_str(),
                Message::Assistant { content, .. } => content.as_deref().unwrap_or(""),
                _ => "",
            })
            .collect()
    }

    #[test]
    fn empty_history_yields_no_messages() {
        let mem = ConversationMemory::default();
        assert!(mem.recent_messages(Some(&room("office"))).is_empty());
    }

    #[test]
    fn records_turns_as_alternating_user_assistant() {
        let mem = ConversationMemory::default();
        let office = room("office");
        mem.record(
            Some(&office),
            "turn on the office light",
            "Turned on the office light.",
        );
        let msgs = mem.recent_messages(Some(&office));
        assert_eq!(
            texts(&msgs),
            vec!["turn on the office light", "Turned on the office light."]
        );
        assert!(matches!(msgs[0], Message::User { .. }));
        assert!(matches!(msgs[1], Message::Assistant { .. }));
    }

    #[test]
    fn caps_at_max_turns_dropping_oldest() {
        let mem = ConversationMemory::new(2, Duration::from_secs(180));
        let office = room("office");
        mem.record(Some(&office), "u1", "a1");
        mem.record(Some(&office), "u2", "a2");
        mem.record(Some(&office), "u3", "a3");
        // u1/a1 evicted; only the two most recent exchanges remain.
        assert_eq!(
            texts(&mem.recent_messages(Some(&office))),
            vec!["u2", "a2", "u3", "a3"]
        );
    }

    #[test]
    fn stale_history_is_dropped_after_ttl() {
        let mem = ConversationMemory::new(4, Duration::from_secs(180));
        let office = room("office");
        let t0 = Instant::now();
        mem.record_at(Some(&office), "u1", "a1", t0);
        // Just inside the window: still there.
        assert!(
            !mem.recent_messages_at(Some(&office), t0 + Duration::from_secs(60))
                .is_empty()
        );
        // Past the TTL: a fresh conversation, nothing carried over.
        assert!(
            mem.recent_messages_at(Some(&office), t0 + Duration::from_secs(300))
                .is_empty()
        );
    }

    #[test]
    fn rooms_do_not_share_context() {
        let mem = ConversationMemory::default();
        let office = room("office");
        let kitchen = room("kitchen");
        mem.record(Some(&office), "u-office", "a-office");
        assert!(mem.recent_messages(Some(&kitchen)).is_empty());
        assert_eq!(
            texts(&mem.recent_messages(Some(&office))),
            vec!["u-office", "a-office"]
        );
    }

    #[test]
    fn a_new_turn_after_ttl_resets_history() {
        let mem = ConversationMemory::new(4, Duration::from_secs(180));
        let office = room("office");
        let t0 = Instant::now();
        mem.record_at(Some(&office), "u1", "a1", t0);
        // Recording again after the TTL clears the stale turn first.
        let later = t0 + Duration::from_secs(300);
        mem.record_at(Some(&office), "u2", "a2", later);
        assert_eq!(
            texts(&mem.recent_messages_at(Some(&office), later)),
            vec!["u2", "a2"]
        );
    }
}
