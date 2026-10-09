//! Answering Niles's question without the wake word.
//!
//! When a reply ends in a question, the satellite is told to listen once
//! more as soon as it has finished speaking, and the answer arrives
//! marked as one. The microphone opening without its name is exactly
//! what went wrong the day a television woke it fifty-three times, so an
//! answer is only taken when Niles really is waiting for one, from the
//! voice it asked, and only for a few rounds in a row.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Long enough for the reply to be spoken and the answer to begin. The
/// satellite gives up on its own after six seconds of nobody talking.
const WAITING: Duration = Duration::from_secs(60);

/// Questions answered in a row before the wake word is needed again: a
/// conversation, not an open microphone.
pub(crate) const MAX_ROUNDS: u8 = 3;

/// A question asked through one satellite.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Asked {
    /// Who was asked, when the voice was recognised.
    pub asker: Option<String>,
    /// Which question in a row this is, from 1.
    pub round: u8,
    at: Instant,
}

/// What to make of a session the satellite says is an answer.
#[derive(Debug, PartialEq)]
pub(crate) enum Verdict {
    /// Take it, as the answer to question `round`.
    Answer { round: u8 },
    /// No question is waiting: nothing Niles should act on.
    Unasked,
    /// Somebody else answered — or the television.
    OtherVoice,
}

/// The questions waiting for an answer, one per satellite.
///
/// Keyed by the satellite's address without the port: every wake is a
/// new connection from a new port, and the answer is one too.
#[derive(Default)]
pub(crate) struct FollowUps {
    waiting: Mutex<HashMap<IpAddr, Asked>>,
}

impl FollowUps {
    /// Wait for an answer at `peer`, from `asker` if the voice was known.
    pub(crate) fn expect(&self, peer: SocketAddr, asker: Option<String>, round: u8) {
        self.lock().insert(
            peer.ip(),
            Asked {
                asker,
                round,
                at: Instant::now(),
            },
        );
    }

    /// The question waiting at `peer`, if there is one still worth
    /// answering. Taken: one question, one answer.
    pub(crate) fn take(&self, peer: SocketAddr) -> Option<Asked> {
        self.take_at(peer, Instant::now())
    }

    fn take_at(&self, peer: SocketAddr, now: Instant) -> Option<Asked> {
        self.lock()
            .remove(&peer.ip())
            .filter(|asked| now.duration_since(asked.at) <= WAITING)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, Asked>> {
        self.waiting.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Whether an answer is the one Niles is waiting for.
///
/// `speaker` is who the answer sounds like, and `recognising` whether
/// voices are being recognised at all. With recognition on, an answer
/// in somebody else's voice is not taken; a question asked of somebody
/// Niles did not know is not opened to the room in the first place.
///
/// A voice nobody is recognised in is taken. Over music it is the usual
/// case, not the odd one: Mark asked, then answered with Gavin DeGraw
/// playing, scored 0.29 against his own voice, and the answer he gave
/// was thrown away as somebody else's. The window is seconds long and
/// opens only after a question, which is all the guard the television
/// needs.
pub(crate) fn judge(asked: Option<Asked>, speaker: Option<&str>, recognising: bool) -> Verdict {
    let Some(asked) = asked else {
        return Verdict::Unasked;
    };
    if recognising
        && let Some(speaker) = speaker
        && asked.asker.as_deref() != Some(speaker)
    {
        return Verdict::OtherVoice;
    }
    Verdict::Answer { round: asked.round }
}

/// Whether a reply is a question worth listening for an answer to.
///
/// Not "Sorry?": that is said to whatever was not understood, which is
/// as often the television as a person, and listening after it would
/// let the television carry on a conversation with itself.
pub(crate) fn asks(reply: &str) -> bool {
    let reply = reply.trim_end();
    reply.ends_with('?') && reply != crate::response::didnt_catch_that()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> SocketAddr {
        "192.168.69.188:51833".parse().unwrap()
    }

    fn asked(asker: Option<&str>) -> Option<Asked> {
        Some(Asked {
            asker: asker.map(String::from),
            round: 1,
            at: Instant::now(),
        })
    }

    #[test]
    fn a_question_is_asked_and_a_statement_is_not() {
        assert!(asks("Shall I put some of them out?"));
        assert!(asks("Which room, Sir?  "));
        assert!(!asks("Done."));
        assert!(!asks("Sorry, I didn't recognise your voice."));
    }

    #[test]
    fn sorry_is_not_a_question_to_wait_on() {
        assert!(!asks(&crate::response::didnt_catch_that()));
    }

    #[test]
    fn the_voice_that_was_asked_may_answer() {
        assert_eq!(
            judge(asked(Some("Mark")), Some("Mark"), true),
            Verdict::Answer { round: 1 }
        );
    }

    #[test]
    fn another_voice_may_not() {
        assert_eq!(
            judge(asked(Some("Mark")), Some("Majse"), true),
            Verdict::OtherVoice
        );
    }

    #[test]
    fn a_voice_drowned_by_music_may() {
        // Nobody recognised: over music, that is the person who was asked.
        assert_eq!(
            judge(asked(Some("Mark")), None, true),
            Verdict::Answer { round: 1 }
        );
    }

    #[test]
    fn without_recognition_the_answer_is_taken() {
        assert_eq!(
            judge(asked(None), None, false),
            Verdict::Answer { round: 1 }
        );
    }

    #[test]
    fn an_answer_nobody_asked_for_is_not_acted_on() {
        assert_eq!(judge(None, Some("Mark"), true), Verdict::Unasked);
    }

    #[test]
    fn a_question_is_answered_once() {
        let waiting = FollowUps::default();
        waiting.expect(peer(), Some("Mark".into()), 1);
        assert!(waiting.take(peer()).is_some());
        assert!(waiting.take(peer()).is_none());
    }

    #[test]
    fn a_question_left_too_long_lapses() {
        let waiting = FollowUps::default();
        waiting.expect(peer(), None, 1);
        let later = Instant::now() + WAITING + Duration::from_secs(1);
        assert!(waiting.take_at(peer(), later).is_none());
    }

    #[test]
    fn the_answer_comes_from_a_new_connection() {
        let waiting = FollowUps::default();
        waiting.expect(peer(), None, 1);
        let next: SocketAddr = "192.168.69.188:51834".parse().unwrap();
        assert!(waiting.take(next).is_some());
    }

    #[test]
    fn each_satellite_waits_for_its_own_answer() {
        let waiting = FollowUps::default();
        waiting.expect(peer(), None, 1);
        let other: SocketAddr = "192.168.69.189:40000".parse().unwrap();
        assert!(waiting.take(other).is_none());
        assert!(waiting.take(peer()).is_some());
    }
}
