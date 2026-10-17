//! Recording: the script's lines said, two at a time, in their hosts'
//! voices, then joined with the lead-in and pauses.

use crate::error::{Error, Result};
use crate::{Speech, mp3};
use futures_util::StreamExt;
use futures_util::stream;
use std::time::Duration;

/// Half a second of silence before the first line: the player is
/// found, the room goes quiet.
pub const LEAD_IN: Duration = Duration::from_millis(500);
/// Between two turns: a breath, like a host handing over.
pub const PAUSE: Duration = Duration::from_millis(400);
/// Two lines said at once keeps a slow voice from idling; more would
/// arrive out of order.
const SAYING_AT_ONCE: usize = 2;
/// How long a play address lasts: long enough for an evening, short
/// enough to be meaningless tomorrow.
pub const PLAY_FOR: Duration = Duration::from_secs(4 * 60 * 60);
/// The script promises 8 minutes; over that is logged, never cut.
const MOST_SECONDS: f64 = 8.0 * 60.0;

/// The bucket key for one episode's recording.
pub fn episode_key(artifact: i64) -> String {
    format!("podcasts/{artifact}.mp3")
}

/// Say every line in its host's voice and join the answers, in the
/// script's order. A line that fails says which it was.
pub async fn record(
    speech: &dyn Speech,
    voices: &[String; 2],
    lines: &[(usize, String)],
) -> Result<mp3::Joined> {
    let total = lines.len();
    // Materialized before `buffered`: the futures share one lifetime
    // (`speech`'s), which the higher-ranked closure the stream adapter
    // wants cannot be talked into.
    let said: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(i, (host, text))| {
            let n = i + 1;
            let voice = voices[*host].clone();
            let text = text.clone();
            async move {
                speech.say(&voice, &text).await.map_err(|e| match e {
                    Error::Speech { reason } => Error::Speech {
                        reason: format!("line {n} of {total} was not recorded: {reason}"),
                    },
                    other => other,
                })
            }
        })
        .collect();
    let mut answers = stream::iter(said).buffered(SAYING_AT_ONCE);
    let mut segments = Vec::with_capacity(total);
    let mut bytes = 0usize;
    while let Some(answer) = answers.next().await {
        let segment = answer?;
        if segment.len() > mp3::MOST_JOINED - bytes {
            return Err(Error::Audio {
                reason: "the recording came out bigger than an episode can be".into(),
            });
        }
        bytes += segment.len();
        segments.push(segment);
    }
    let joined = mp3::join(&segments, LEAD_IN, PAUSE)?;
    if joined.seconds > MOST_SECONDS {
        // The script's budget said this; a runaway says it again here,
        // and that is all: cutting audio would leave the script lying.
        tracing::info!(
            seconds = joined.seconds,
            "an episode came out longer than eight minutes; it is kept whole"
        );
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    /// The three silent frames a fake voice answers with: Kokoro's own
    /// format (24 kHz mono MPEG-2), enough for `mp3::join`.
    pub(crate) fn silent_answer() -> Vec<u8> {
        let frame = [0xFF, 0xF3, 0x84, 0xC0]
            .into_iter()
            .chain(std::iter::repeat_n(0u8, 188))
            .collect::<Vec<_>>();
        frame.repeat(3)
    }

    #[derive(Default)]
    struct FakeSpeech {
        said: Mutex<Vec<(String, String)>>,
        fail: bool,
        delay_first: bool,
    }

    impl FakeSpeech {
        fn said(&self) -> Vec<(String, String)> {
            self.said.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl Speech for FakeSpeech {
        async fn say(&self, voice: &str, text: &str) -> Result<Vec<u8>> {
            if self.delay_first && self.said.lock().is_empty() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            self.said.lock().push((voice.to_string(), text.to_string()));
            if self.fail {
                return Err(Error::Speech {
                    reason: "Kokoro did not answer within 120 s".into(),
                });
            }
            Ok(silent_answer())
        }
    }

    fn script() -> Vec<(usize, String)> {
        [
            (0, "Welcome to the show.".to_string()),
            (1, "Thanks, good to be here.".to_string()),
            (0, "Let's begin.".to_string()),
        ]
        .to_vec()
    }

    #[tokio::test]
    async fn host_zero_gets_the_first_voice_and_host_one_the_second() {
        let speech = FakeSpeech::default();
        let voices = ["af_heart".to_string(), "am_michael".to_string()];
        record(&speech, &voices, &script()).await.expect("recorded");
        assert_eq!(
            speech.said(),
            [
                ("af_heart", "Welcome to the show."),
                ("am_michael", "Thanks, good to be here."),
                ("af_heart", "Let's begin."),
            ]
            .map(|(v, t)| (v.to_string(), t.to_string()))
        );
    }

    #[tokio::test]
    async fn segments_join_in_the_scripts_order_even_when_a_later_line_answers_first() {
        let speech = FakeSpeech {
            delay_first: true,
            ..Default::default()
        };
        let joined = record(
            &speech,
            &["af_heart".to_string(), "am_michael".to_string()],
            &script(),
        )
        .await
        .expect("recorded");
        // 3 lines × 3 frames each, plus the lead-in and two pauses:
        // the order they were said in is the order they are kept in.
        assert_eq!(joined.frames, 21 + 9 + 17 + 17);
        let said = speech.said();
        assert_eq!(said[0].1, "Welcome to the show.");
        assert_eq!(said[1].1, "Thanks, good to be here.");
    }

    #[tokio::test]
    async fn a_failing_line_says_which_it_was() {
        let speech = FakeSpeech {
            fail: true,
            ..Default::default()
        };
        let error = record(
            &speech,
            &["af_heart".to_string(), "am_michael".to_string()],
            &script(),
        )
        .await
        .expect_err("failed");
        // The first line is named, and the total with it.
        assert!(
            error.to_string().contains("line 1 of 3 was not recorded"),
            "unexpected: {error}"
        );
    }
}
