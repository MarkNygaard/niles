//! Short-term conversation memory: a small rolling buffer of recent
//! turns, kept per room for voice and per person for the app's chat, so
//! the LLM can resolve follow-ups like "turn it off again" or "make it
//! warmer" against what was just said.
//!
//! This is the short-term, in-context half of conversation memory
//! (`niles-memory` is the long-term half). It holds only the last few turns, scoped to one room and expired by a
//! short idle TTL so an unrelated command minutes later starts fresh.
//!
//! Every change is also handed on whole, to be kept in Postgres: a
//! restart — every deploy — used to forget what was said a minute ago.
//! Times are wall-clock for the same reason; an `Instant` means nothing
//! to the next process.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use niles_core::RoomName;
use niles_llm::Message;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

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
#[derive(Clone, Serialize, Deserialize)]
struct Turn {
    user: String,
    assistant: String,
    /// What wrote the reply when it was not Niles's own models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    via: Option<String>,
}

struct History {
    turns: VecDeque<Turn>,
    last_at: SystemTime,
}

/// One thread as it is kept.
#[derive(Serialize, Deserialize)]
struct Kept {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    room: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chat: Option<String>,
    /// Seconds since the Unix epoch.
    last_at: u64,
    turns: VecDeque<Turn>,
}

/// Recent turns keyed by thread. Satellites with no room mapping share
/// a single bucket (`Room(None)`), which is fine for a single-room
/// install and keeps unmapped devices from bleeding context into a named
/// room. Chat never shares a bucket with a room.
pub(crate) struct ConversationMemory {
    by_thread: Mutex<HashMap<Thread, History>>,
    max_turns: usize,
    ttl: Duration,
    sink: Option<UnboundedSender<String>>,
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
            sink: None,
        }
    }

    /// Pick up the conversations kept by an earlier run. Threads gone
    /// quiet past their TTL are left behind; a document that will not
    /// parse is reported and dropped — it is a few minutes of talk.
    pub(crate) fn with_document(self, document: &str) -> Self {
        let kept: Vec<Kept> = match serde_json::from_str(document) {
            Ok(kept) => kept,
            Err(e) => {
                tracing::warn!("[conversation] the kept conversations would not parse: {e}");
                return self;
            }
        };
        let now = SystemTime::now();
        let mut map = self.lock();
        for k in kept {
            let thread = match (k.room, k.chat) {
                (_, Some(who)) => Thread::Chat(who),
                (Some(room), None) => match RoomName::parse(room) {
                    Ok(room) => Thread::Room(Some(room)),
                    Err(_) => continue,
                },
                (None, None) => Thread::Room(None),
            };
            let last_at = UNIX_EPOCH + Duration::from_secs(k.last_at);
            if elapsed(last_at, now) > self.limits(&thread).1 {
                continue;
            }
            map.insert(
                thread,
                History {
                    turns: k.turns,
                    last_at,
                },
            );
        }
        drop(map);
        self
    }

    /// Hand every change on, whole, to be kept.
    pub(crate) fn with_sink(mut self, sink: UnboundedSender<String>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Thread, History>> {
        self.by_thread.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Prior turns in `thread` as alternating user/assistant messages,
    /// oldest first — ready to splice between the system prompt and the
    /// current utterance. Empty when there's no live history (nothing
    /// recorded, or the last turn is older than the TTL).
    pub(crate) fn recent_messages(&self, thread: &Thread) -> Vec<Message> {
        self.recent_messages_at(thread, SystemTime::now())
    }

    /// The live turns in `thread` as (what was said, what Niles replied,
    /// what wrote the reply), oldest first.
    #[cfg(test)]
    pub(crate) fn turns(&self, thread: &Thread) -> Vec<(String, String, Option<String>)> {
        self.live(thread, SystemTime::now(), |turns| {
            turns
                .iter()
                .map(|t| (t.user.clone(), t.assistant.clone(), t.via.clone()))
                .collect()
        })
    }

    /// Record a completed exchange so the next turn in the same thread
    /// can see it. A turn arriving after the TTL starts a fresh history.
    pub(crate) fn record(&self, thread: &Thread, user: &str, assistant: &str) {
        self.record_at(thread, user, assistant, None, SystemTime::now());
    }

    /// [`Self::record`], saying what wrote the reply.
    pub(crate) fn record_via(
        &self,
        thread: &Thread,
        user: &str,
        assistant: &str,
        via: Option<&str>,
    ) {
        self.record_at(thread, user, assistant, via, SystemTime::now());
    }

    /// Make `thread` exactly these turns — a kept conversation picked up
    /// again, every turn of it, however long ago the last one was. The
    /// next turn recorded trims it to the usual length.
    pub(crate) fn seed(&self, thread: &Thread, turns: Vec<(String, String, Option<String>)>) {
        let mut map = self.lock();
        map.insert(
            thread.clone(),
            History {
                turns: turns
                    .into_iter()
                    .map(|(user, assistant, via)| Turn {
                        user,
                        assistant,
                        via,
                    })
                    .collect(),
                last_at: SystemTime::now(),
            },
        );
        self.keep(&map, SystemTime::now());
    }

    /// Start `thread` over.
    pub(crate) fn forget(&self, thread: &Thread) {
        let mut map = self.lock();
        map.remove(thread);
        self.keep(&map, SystemTime::now());
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
        now: SystemTime,
        read: impl FnOnce(&VecDeque<Turn>) -> T,
    ) -> T {
        let map = self.lock();
        let Some(history) = map.get(thread) else {
            return T::default();
        };
        if elapsed(history.last_at, now) > self.limits(thread).1 {
            return T::default();
        }
        read(&history.turns)
    }

    fn recent_messages_at(&self, thread: &Thread, now: SystemTime) -> Vec<Message> {
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
        via: Option<&str>,
        now: SystemTime,
    ) {
        let (max_turns, ttl) = self.limits(thread);
        let mut map = self.lock();
        let entry = map.entry(thread.clone()).or_insert_with(|| History {
            turns: VecDeque::new(),
            last_at: now,
        });
        // An idle gap past the TTL means this is a new conversation, not a
        // follow-up — drop the stale turns rather than splicing them in.
        if elapsed(entry.last_at, now) > ttl {
            entry.turns.clear();
        }
        entry.turns.push_back(Turn {
            user: user.to_string(),
            assistant: assistant.to_string(),
            via: via.map(str::to_string),
        });
        while entry.turns.len() > max_turns {
            entry.turns.pop_front();
        }
        entry.last_at = now;
        self.keep(&map, now);
    }

    /// Hand the live threads on. Called with the lock held, so what is
    /// kept arrives in the order it was said.
    fn keep(&self, map: &HashMap<Thread, History>, now: SystemTime) {
        let Some(sink) = &self.sink else { return };
        let kept: Vec<Kept> = map
            .iter()
            .filter(|(thread, history)| elapsed(history.last_at, now) <= self.limits(thread).1)
            .map(|(thread, history)| {
                let (room, chat) = match thread {
                    Thread::Room(room) => (room.as_ref().map(|r| r.as_str().to_string()), None),
                    Thread::Chat(who) => (None, Some(who.clone())),
                };
                Kept {
                    room,
                    chat,
                    last_at: history
                        .last_at
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                    turns: history.turns.clone(),
                }
            })
            .collect();
        match serde_json::to_string(&kept) {
            Ok(document) => {
                if sink.send(document).is_err() {
                    tracing::warn!(
                        "[conversation] nothing is keeping it any more; this turn is in memory only"
                    );
                }
            }
            Err(e) => tracing::warn!("[conversation] could not be written down: {e}"),
        }
    }
}

/// How long since `then`. A clock stepped backwards reads as no time
/// at all, which keeps the conversation rather than dropping it.
fn elapsed(then: SystemTime, now: SystemTime) -> Duration {
    now.duration_since(then).unwrap_or_default()
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
        let t0 = SystemTime::now();
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
        let t0 = SystemTime::now();
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
        assert_eq!(vias, vec![Some("claude".to_string()), None]);
    }

    #[test]
    fn a_picked_up_conversation_is_all_there_until_the_next_turn() {
        let mem = ConversationMemory::default();
        let mark = Thread::Chat("mark@example.com".into());
        mem.record(&mark, "stale", "gone");
        let kept: Vec<_> = (0..20)
            .map(|i| (format!("u{i}"), format!("a{i}"), None))
            .collect();
        mem.seed(&mark, kept);
        let turns = mem.turns(&mark);
        assert_eq!(turns.len(), 20);
        assert_eq!(turns[0].0, "u0");
        mem.record(&mark, "next", "reply");
        assert_eq!(mem.turns(&mark).len(), CHAT_MAX_TURNS);
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
        let t0 = SystemTime::now();
        mem.record_at(&office, "u1", "a1", None, t0);
        // Recording again after the TTL clears the stale turn first.
        let later = t0 + Duration::from_secs(300);
        mem.record_at(&office, "u2", "a2", None, later);
        assert_eq!(
            texts(&mem.recent_messages_at(&office, later)),
            vec!["u2", "a2"]
        );
    }

    fn kept_by(
        mem: ConversationMemory,
    ) -> (
        ConversationMemory,
        tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (mem.with_sink(tx), rx)
    }

    fn last(rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>) -> String {
        let mut last = None;
        while let Ok(document) = rx.try_recv() {
            last = Some(document);
        }
        last.expect("something was kept")
    }

    #[test]
    fn a_conversation_outlasts_a_restart() {
        let (mem, mut rx) = kept_by(ConversationMemory::default());
        let office = room("office");
        let mark = Thread::Chat("mark@example.com".into());
        mem.record(&office, "turn on the office light", "Done.");
        mem.record_via(&mark, "u1", "a1", Some("claude"));
        mem.record(&Thread::Room(None), "what time is it", "Ten past nine.");

        let next = ConversationMemory::default().with_document(&last(&mut rx));
        assert_eq!(
            texts(&next.recent_messages(&office)),
            vec!["turn on the office light", "Done."]
        );
        assert_eq!(
            next.turns(&mark),
            vec![("u1".into(), "a1".into(), Some("claude".into()))]
        );
        assert_eq!(
            texts(&next.recent_messages(&Thread::Room(None))),
            vec!["what time is it", "Ten past nine."]
        );
    }

    #[test]
    fn a_conversation_gone_quiet_is_not_picked_up_again() {
        let (mem, mut rx) = kept_by(ConversationMemory::default());
        let office = room("office");
        let mark = Thread::Chat("mark@example.com".into());
        let ten_minutes_ago = SystemTime::now() - Duration::from_secs(10 * 60);
        mem.record_at(&office, "u1", "a1", None, ten_minutes_ago);
        mem.record_at(&mark, "u2", "a2", None, ten_minutes_ago);

        let next = ConversationMemory::default().with_document(&last(&mut rx));
        assert!(next.recent_messages(&office).is_empty());
        assert_eq!(texts(&next.recent_messages(&mark)), vec!["u2", "a2"]);
    }

    #[test]
    fn forgetting_is_kept_too() {
        let (mem, mut rx) = kept_by(ConversationMemory::default());
        let mark = Thread::Chat("mark@example.com".into());
        mem.record(&mark, "u1", "a1");
        mem.forget(&mark);

        let next = ConversationMemory::default().with_document(&last(&mut rx));
        assert!(next.turns(&mark).is_empty());
    }

    #[test]
    fn an_unreadable_document_starts_empty() {
        let mem = ConversationMemory::default().with_document("not json");
        assert!(mem.recent_messages(&room("office")).is_empty());
    }
}
