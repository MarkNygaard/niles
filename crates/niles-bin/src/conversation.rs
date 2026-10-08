//! Short-term conversation memory: a small rolling buffer of recent
//! turns, kept per room for voice and per person for the app's chat, so
//! the LLM can resolve follow-ups like "turn it off again" or "make it
//! warmer" against what was just said.
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

/// A typed conversation is read back on screen and picked up again after
/// a look at something else, so it is longer and outlives a pause that
/// would end a spoken one.
const CHAT_MAX_TURNS: usize = 12;
const CHAT_TTL: Duration = Duration::from_secs(30 * 60);

/// Whose conversation a turn belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Thread {
    /// Spoken, in a room.
    Room(Option<RoomName>),
    /// Typed in the app, by one signed-in person, keyed by their address.
    Chat(String),
}

/// One completed exchange: what the user said and how niles replied.
#[derive(Clone)]
struct Turn {
    user: String,
    assistant: String,
    /// What wrote the reply when it was not Niles's own models.
    via: Option<&'static str>,
}

struct History {
    turns: VecDeque<Turn>,
    last_at: Instant,
}

/// Recent turns keyed by thread. Satellites with no room mapping share
/// a single bucket (`Room(None)`), which is fine for a single-room
/// install and keeps unmapped devices from bleeding context into a named
/// room. Chat never shares a bucket with a room.
pub(crate) struct ConversationMemory {
    by_thread: Mutex<HashMap<Thread, History>>,
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
            by_thread: Mutex::new(HashMap::new()),
            max_turns,
            ttl,
        }
    }

    /// Prior turns in `thread` as alternating user/assistant messages,
    /// oldest first — ready to splice between the system prompt and the
    /// current utterance. Empty when there's no live history (nothing
    /// recorded, or the last turn is older than the TTL).
    pub(crate) fn recent_messages(&self, thread: &Thread) -> Vec<Message> {
        self.recent_messages_at(thread, Instant::now())
    }

    /// The live turns in `thread` as (what was said, what Niles replied,
    /// what wrote the reply), oldest first — what the app shows when the
    /// chat is reopened.
    pub(crate) fn turns(&self, thread: &Thread) -> Vec<(String, String, Option<&'static str>)> {
        self.live(thread, Instant::now(), |turns| {
            turns
                .iter()
                .map(|t| (t.user.clone(), t.assistant.clone(), t.via))
                .collect()
        })
    }

    /// Record a completed exchange so the next turn in the same thread
    /// can see it. A turn arriving after the TTL starts a fresh history.
    pub(crate) fn record(&self, thread: &Thread, user: &str, assistant: &str) {
        self.record_at(thread, user, assistant, None, Instant::now());
    }

    /// [`Self::record`], saying what wrote the reply.
    pub(crate) fn record_via(
        &self,
        thread: &Thread,
        user: &str,
        assistant: &str,
        via: Option<&'static str>,
    ) {
        self.record_at(thread, user, assistant, via, Instant::now());
    }

    /// Start `thread` over.
    pub(crate) fn forget(&self, thread: &Thread) {
        let mut map = self.by_thread.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(thread);
    }

    fn limits(&self, thread: &Thread) -> (usize, Duration) {
        match thread {
            Thread::Room(_) => (self.max_turns, self.ttl),
            Thread::Chat(_) => (CHAT_MAX_TURNS, CHAT_TTL),
        }
    }

    fn live<T: Default>(
        &self,
        thread: &Thread,
        now: Instant,
        read: impl FnOnce(&VecDeque<Turn>) -> T,
    ) -> T {
        let map = self.by_thread.lock().unwrap_or_else(|e| e.into_inner());
        let Some(history) = map.get(thread) else {
            return T::default();
        };
        if now.duration_since(history.last_at) > self.limits(thread).1 {
            return T::default();
        }
        read(&history.turns)
    }

    fn recent_messages_at(&self, thread: &Thread, now: Instant) -> Vec<Message> {
        self.live(thread, now, |turns| {
            let mut out = Vec::with_capacity(turns.len() * 2);
            for turn in turns {
                out.push(Message::User {
                    content: turn.user.clone(),
                });
                out.push(Message::Assistant {
                    content: Some(turn.assistant.clone()),
                    tool_calls: None,
                });
            }
            out
        })
    }

    fn record_at(
        &self,
        thread: &Thread,
        user: &str,
        assistant: &str,
        via: Option<&'static str>,
        now: Instant,
    ) {
        let (max_turns, ttl) = self.limits(thread);
        let mut map = self.by_thread.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(thread.clone()).or_insert_with(|| History {
            turns: VecDeque::new(),
            last_at: now,
        });
        // An idle gap past the TTL means this is a new conversation, not a
        // follow-up — drop the stale turns rather than splicing them in.
        if now.duration_since(entry.last_at) > ttl {
            entry.turns.clear();
        }
        entry.turns.push_back(Turn {
            user: user.to_string(),
            assistant: assistant.to_string(),
            via,
        });
        while entry.turns.len() > max_turns {
            entry.turns.pop_front();
        }
        entry.last_at = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(name: &str) -> Thread {
        Thread::Room(Some(RoomName::parse(name).unwrap()))
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
        assert!(mem.recent_messages(&room("office")).is_empty());
    }

    #[test]
    fn records_turns_as_alternating_user_assistant() {
        let mem = ConversationMemory::default();
        let office = room("office");
        mem.record(
            &office,
            "turn on the office light",
            "Turned on the office light.",
        );
        let msgs = mem.recent_messages(&office);
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
        mem.record(&office, "u1", "a1");
        mem.record(&office, "u2", "a2");
        mem.record(&office, "u3", "a3");
        // u1/a1 evicted; only the two most recent exchanges remain.
        assert_eq!(
            texts(&mem.recent_messages(&office)),
            vec!["u2", "a2", "u3", "a3"]
        );
    }

    #[test]
    fn stale_history_is_dropped_after_ttl() {
        let mem = ConversationMemory::new(4, Duration::from_secs(180));
        let office = room("office");
        let t0 = Instant::now();
        mem.record_at(&office, "u1", "a1", None, t0);
        // Just inside the window: still there.
        assert!(
            !mem.recent_messages_at(&office, t0 + Duration::from_secs(60))
                .is_empty()
        );
        // Past the TTL: a fresh conversation, nothing carried over.
        assert!(
            mem.recent_messages_at(&office, t0 + Duration::from_secs(300))
                .is_empty()
        );
    }

    #[test]
    fn rooms_do_not_share_context() {
        let mem = ConversationMemory::default();
        let office = room("office");
        let kitchen = room("kitchen");
        mem.record(&office, "u-office", "a-office");
        assert!(mem.recent_messages(&kitchen).is_empty());
        assert_eq!(
            texts(&mem.recent_messages(&office)),
            vec!["u-office", "a-office"]
        );
    }

    #[test]
    fn a_chat_is_its_own_thread() {
        let mem = ConversationMemory::default();
        let office = room("office");
        let mark = Thread::Chat("mark@example.com".into());
        mem.record(&office, "u-office", "a-office");
        mem.record(&mark, "u-chat", "a-chat");
        assert_eq!(texts(&mem.recent_messages(&mark)), vec!["u-chat", "a-chat"]);
        assert!(
            mem.recent_messages(&Thread::Chat("majse@example.com".into()))
                .is_empty()
        );
    }

    #[test]
    fn a_chat_outlasts_a_pause_that_ends_a_spoken_conversation() {
        let mem = ConversationMemory::default();
        let mark = Thread::Chat("mark@example.com".into());
        let t0 = Instant::now();
        mem.record_at(&mark, "u1", "a1", None, t0);
        let later = t0 + Duration::from_secs(10 * 60);
        assert_eq!(
            texts(&mem.recent_messages_at(&mark, later)),
            vec!["u1", "a1"]
        );
    }

    #[test]
    fn a_chat_keeps_more_turns() {
        let mem = ConversationMemory::default();
        let mark = Thread::Chat("mark@example.com".into());
        for i in 0..10 {
            mem.record(&mark, &format!("u{i}"), &format!("a{i}"));
        }
        assert_eq!(mem.turns(&mark).len(), 10);
    }

    #[test]
    fn a_turn_remembers_what_wrote_the_reply() {
        let mem = ConversationMemory::default();
        let mark = Thread::Chat("mark@example.com".into());
        mem.record_via(&mark, "u1", "a1", Some("claude"));
        mem.record(&mark, "u2", "a2");
        let vias: Vec<_> = mem.turns(&mark).into_iter().map(|t| t.2).collect();
        assert_eq!(vias, vec![Some("claude"), None]);
    }

    #[test]
    fn forgetting_starts_over() {
        let mem = ConversationMemory::default();
        let mark = Thread::Chat("mark@example.com".into());
        mem.record(&mark, "u1", "a1");
        mem.forget(&mark);
        assert!(mem.turns(&mark).is_empty());
    }

    #[test]
    fn a_new_turn_after_ttl_resets_history() {
        let mem = ConversationMemory::new(4, Duration::from_secs(180));
        let office = room("office");
        let t0 = Instant::now();
        mem.record_at(&office, "u1", "a1", None, t0);
        // Recording again after the TTL clears the stale turn first.
        let later = t0 + Duration::from_secs(300);
        mem.record_at(&office, "u2", "a2", None, later);
        assert_eq!(
            texts(&mem.recent_messages_at(&office, later)),
            vec!["u2", "a2"]
        );
    }
}
