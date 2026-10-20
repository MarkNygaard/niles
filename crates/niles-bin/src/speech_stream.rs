//! Host-owned plugin speech output and conversation priority.
//!
//! Commentary is admitted only while a room has no interactive voice turn. A
//! conversation lease cancels commentary that has not handed audio to the
//! satellite yet; the Wyoming protocol has no cancellation event for frames
//! already accepted by the satellite's outbound writer.

use crate::satellites::SatelliteRegistry;
use crate::speak::{at_volume, for_satellite, wav_to_pcm};
use chrono::Utc;
use niles_core::RoomName;
use niles_notifications::QuietHoursConfig;
use niles_tts::{
    PiperClient,
    speech_session::{Eligibility, PluginOwner, SpeechOutputError},
};
use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;

#[derive(Default)]
pub(crate) struct RoomArbiter {
    rooms: Mutex<HashMap<RoomName, RoomState>>,
}
#[derive(Default)]
struct RoomState {
    conversations: usize,
    commentary: Option<watch::Sender<bool>>,
    notifications: usize,
}

/// Held from wake until the voice dispatch task has finished its reply.
pub(crate) struct ConversationLease {
    arbiter: Arc<RoomArbiter>,
    room: RoomName,
}
impl Drop for ConversationLease {
    fn drop(&mut self) {
        let mut rooms = self.arbiter.rooms.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = rooms.get_mut(&self.room) {
            state.conversations = state.conversations.saturating_sub(1);
        }
    }
}

pub(crate) struct CommentaryLease {
    arbiter: Arc<RoomArbiter>,
    room: RoomName,
    cancelled: watch::Receiver<bool>,
}
impl CommentaryLease {
    async fn cancelled(&mut self) {
        let _ = self.cancelled.changed().await;
    }
}
impl Drop for CommentaryLease {
    fn drop(&mut self) {
        let mut rooms = self.arbiter.rooms.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = rooms.get_mut(&self.room) {
            state.commentary = None;
        }
    }
}

pub(crate) struct NotificationLease {
    arbiter: Arc<RoomArbiter>,
    room: RoomName,
}
impl Drop for NotificationLease {
    fn drop(&mut self) {
        let mut rooms = self.arbiter.rooms.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = rooms.get_mut(&self.room) {
            state.notifications = state.notifications.saturating_sub(1);
        }
    }
}
impl RoomArbiter {
    pub(crate) fn enter_conversation(self: &Arc<Self>, room: RoomName) -> ConversationLease {
        let mut rooms = self.rooms.lock().unwrap_or_else(|e| e.into_inner());
        let state = rooms.entry(room.clone()).or_default();
        state.conversations += 1;
        if let Some(cancel) = &state.commentary {
            let _ = cancel.send(true);
        }
        ConversationLease {
            arbiter: self.clone(),
            room,
        }
    }
    fn try_start_commentary(self: &Arc<Self>, room: &RoomName) -> Result<CommentaryLease, ()> {
        let mut rooms = self.rooms.lock().unwrap_or_else(|e| e.into_inner());
        let state = rooms.entry(room.clone()).or_default();
        if state.conversations != 0 || state.notifications != 0 || state.commentary.is_some() {
            return Err(());
        }
        let (cancel, cancelled) = watch::channel(false);
        state.commentary = Some(cancel);
        Ok(CommentaryLease {
            arbiter: self.clone(),
            room: room.clone(),
            cancelled,
        })
    }
    pub(crate) fn reserve_notification(self: &Arc<Self>, room: &RoomName) -> NotificationLease {
        let mut rooms = self.rooms.lock().unwrap_or_else(|e| e.into_inner());
        let state = rooms.entry(room.clone()).or_default();
        state.notifications += 1;
        if let Some(cancel) = &state.commentary {
            let _ = cancel.send(true);
        }
        NotificationLease {
            arbiter: self.clone(),
            room: room.clone(),
        }
    }
    fn busy(&self, room: &RoomName) -> bool {
        let rooms = self.rooms.lock().unwrap_or_else(|e| e.into_inner());
        // Commentary admission must reach the manager's replacement slot.
        // Exclusive output is still enforced by try_start_commentary.
        rooms
            .get(room)
            .is_some_and(|state| state.conversations != 0 || state.notifications != 0)
    }
}

/// How long a voice turn keeps its room once the reply has been written:
/// while the satellite plays it, and after a question, while it listens for
/// the answer. Commentary waits that long; the music and the turn's timing
/// do not.
pub(crate) fn conversation_hold(playing: Duration, asked: bool) -> Duration {
    if asked {
        playing + crate::follow_up::ANSWER_WINDOW
    } else {
        playing
    }
}

/// Spawn work with its voice reservation owned by the task that performs it.
pub(crate) fn spawn_conversation_task<T: Send + 'static>(
    conversation: Option<ConversationLease>,
    work: impl Future<Output = T> + Send + 'static,
) -> tokio::task::JoinHandle<T> {
    tokio::spawn(async move {
        let _conversation = conversation;
        work.await
    })
}

pub(crate) fn take_conversation_after_audio(
    conversations: &mut HashMap<SocketAddr, ConversationLease>,
    peer: SocketAddr,
    audio_stop: bool,
    accepted: bool,
) -> Option<ConversationLease> {
    if accepted {
        conversations.remove(&peer)
    } else if audio_stop {
        conversations.remove(&peer);
        None
    } else {
        None
    }
}

/// The production policy. It is deliberately separate from notification
/// delivery: it only reuses the configured quiet-hours clock as an explicit
/// suppress/reject policy.
pub(crate) struct CommentaryPolicy {
    arbiter: Arc<RoomArbiter>,
    satellites: Arc<SatelliteRegistry>,
    quiet: QuietHoursConfig,
}
impl CommentaryPolicy {
    pub(crate) fn new(
        arbiter: Arc<RoomArbiter>,
        satellites: Arc<SatelliteRegistry>,
        quiet: QuietHoursConfig,
    ) -> Self {
        Self {
            arbiter,
            satellites,
            quiet,
        }
    }
}
impl CommentaryPolicy {
    pub(crate) async fn check(&self, _: &PluginOwner, room: &RoomName) -> Eligibility {
        if self.quiet.is_active(Utc::now()) {
            Eligibility::QuietHours
        } else if self.satellites.ip_for(room).is_none() {
            Eligibility::NoSatellite
        } else if self.arbiter.busy(room) {
            Eligibility::Busy
        } else {
            Eligibility::Eligible
        }
    }
}

/// Synthesizes and pushes to the configured room route, awaiting the complete
/// transport write. This never invokes notification delivery or a live-peer
/// index, so a disconnected satellite is dialled through `ip_for`.
pub(crate) struct CommentaryOutput {
    piper: Arc<PiperClient>,
    satellites: Arc<SatelliteRegistry>,
    arbiter: Arc<RoomArbiter>,
}
impl CommentaryOutput {
    pub(crate) fn new(
        piper: Arc<PiperClient>,
        satellites: Arc<SatelliteRegistry>,
        arbiter: Arc<RoomArbiter>,
    ) -> Self {
        Self {
            piper,
            satellites,
            arbiter,
        }
    }
}
pub(crate) struct PreparedCommentary {
    ip: std::net::IpAddr,
    pcm: Vec<u8>,
    format: niles_wyoming::AudioFormat,
    lease: CommentaryLease,
}
impl CommentaryOutput {
    pub(crate) async fn prepare(
        &self,
        room: &RoomName,
        text: &str,
    ) -> Result<PreparedCommentary, SpeechOutputError> {
        let mut lease = self
            .arbiter
            .try_start_commentary(room)
            .map_err(|()| SpeechOutputError::Busy)?;
        let Some(ip) = self.satellites.ip_for(room) else {
            return Err(SpeechOutputError::Failed);
        };
        let work = async {
            let synthesis =
                (self.piper.synthesize(text, None).await).map_err(|_| SpeechOutputError::Failed)?;
            let (pcm, format) =
                wav_to_pcm(&synthesis.audio_wav).map_err(|_| SpeechOutputError::Failed)?;
            let (pcm, format) = for_satellite(pcm, format);
            let peer = SocketAddr::new(ip, 0);
            let volume = self.satellites.volume_for(peer);
            let pcm = at_volume(&pcm, format.bits_per_sample, volume).into_owned();
            Ok((pcm, format))
        };
        let (pcm, format) = tokio::select! {
            biased;
            _ = lease.cancelled() => return Err(SpeechOutputError::Cancelled),
            result = work => result,
        }?;
        Ok(PreparedCommentary {
            ip,
            pcm,
            format,
            lease,
        })
    }

    pub(crate) async fn deliver(
        &self,
        mut prepared: PreparedCommentary,
    ) -> Result<(), SpeechOutputError> {
        let work = crate::push::speak_to(prepared.ip, &prepared.pcm, prepared.format);
        tokio::select! {
            biased;
            _ = prepared.lease.cancelled() => Err(SpeechOutputError::Cancelled),
            result = work => result.map_err(|_| SpeechOutputError::Failed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_tts::speech_session::SpeechOutput;

    use tokio::sync::Notify;

    #[tokio::test]
    async fn voice_cancels_pending_commentary_and_owns_the_room() {
        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let mut commentary = arbiter.try_start_commentary(&room).unwrap();
        let voice = arbiter.enter_conversation(room.clone());
        tokio::time::timeout(std::time::Duration::from_secs(1), commentary.cancelled())
            .await
            .unwrap();
        assert!(arbiter.try_start_commentary(&room).is_err());
        drop(commentary);
        drop(voice);
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    #[tokio::test]
    async fn dispatch_task_keeps_room_reserved_until_normal_completion() {
        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let conversation = arbiter.enter_conversation(room.clone());
        let other_room = RoomName::parse("kitchen").unwrap();
        let started = Arc::new(Notify::new());
        let gate = Arc::new(Notify::new());
        let task_started = started.clone();
        let task_gate = gate.clone();
        let task = spawn_conversation_task(Some(conversation), async move {
            task_started.notify_one();
            task_gate.notified().await;
            Ok::<_, &'static str>(())
        });
        started.notified().await;
        assert!(arbiter.try_start_commentary(&room).is_err());
        assert!(arbiter.try_start_commentary(&other_room).is_ok());
        gate.notify_one();
        assert_eq!(task.await.unwrap(), Ok(()));
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    #[tokio::test]
    async fn dispatch_error_releases_room_and_overlapping_turns_hold_until_last_exit() {
        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let first = arbiter.enter_conversation(room.clone());
        let second = arbiter.enter_conversation(room.clone());
        let error = spawn_conversation_task(Some(first), async {
            Err::<(), _>("synthetic dispatch error")
        });

        assert_eq!(error.await.unwrap(), Err("synthetic dispatch error"));
        assert!(arbiter.try_start_commentary(&room).is_err());
        drop(second);
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    #[tokio::test]
    async fn aborted_dispatch_releases_room_reservation() {
        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let conversation = arbiter.enter_conversation(room.clone());
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = spawn_conversation_task(Some(conversation), async move {
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        assert!(arbiter.try_start_commentary(&room).is_err());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    #[tokio::test]
    async fn disconnect_drops_pending_session_reservation() {
        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let peer: SocketAddr = "127.0.0.1:43210".parse().unwrap();
        let mut conversations = HashMap::from([(peer, arbiter.enter_conversation(room.clone()))]);

        assert!(arbiter.try_start_commentary(&room).is_err());
        conversations.remove(&peer);
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    #[tokio::test]
    async fn notification_preempts_commentary_and_blocks_new_commentary() {
        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let mut commentary = arbiter.try_start_commentary(&room).unwrap();
        let notification = arbiter.reserve_notification(&room);
        tokio::time::timeout(std::time::Duration::from_secs(1), commentary.cancelled())
            .await
            .unwrap();
        assert!(arbiter.try_start_commentary(&room).is_err());
        drop(commentary);
        drop(notification);
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    #[tokio::test]
    async fn rejected_audio_stop_releases_but_valid_session_transfers_lease() {
        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let peer: SocketAddr = "127.0.0.1:43210".parse().unwrap();
        let mut conversations = HashMap::from([(peer, arbiter.enter_conversation(room.clone()))]);
        assert!(take_conversation_after_audio(&mut conversations, peer, true, false).is_none());
        assert!(arbiter.try_start_commentary(&room).is_ok());
        conversations.insert(peer, arbiter.enter_conversation(room.clone()));
        let transferred = take_conversation_after_audio(&mut conversations, peer, true, true);
        assert!(arbiter.try_start_commentary(&room).is_err());
        drop(transferred);
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    #[test]
    fn invalid_format_and_oversize_session_stops_release_the_actual_wake_lease() {
        use niles_wyoming::{Event, EventKind, SessionTracker, server::IncomingEvent};
        use serde_json::json;

        fn incoming(
            peer: SocketAddr,
            kind: EventKind,
            data: serde_json::Value,
            payload: Vec<u8>,
        ) -> IncomingEvent {
            IncomingEvent {
                from: peer,
                event: Event {
                    kind,
                    data,
                    payload,
                    version: None,
                },
            }
        }

        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let peer: SocketAddr = "127.0.0.1:43210".parse().unwrap();
        let mut conversations = HashMap::new();
        let mut tracker = SessionTracker::new();

        conversations.insert(peer, arbiter.enter_conversation(room.clone()));
        let accepted = tracker.feed(incoming(peer, EventKind::AudioStart, json!({}), vec![]));
        take_conversation_after_audio(&mut conversations, peer, false, accepted.is_some());
        let accepted = tracker.feed(incoming(peer, EventKind::AudioStop, json!({}), vec![]));
        take_conversation_after_audio(&mut conversations, peer, true, accepted.is_some());
        assert!(arbiter.try_start_commentary(&room).is_ok());

        conversations.insert(peer, arbiter.enter_conversation(room.clone()));
        let accepted = tracker.feed(incoming(
            peer,
            EventKind::AudioStart,
            json!({"rate":16000,"width":2,"channels":1}),
            vec![],
        ));
        take_conversation_after_audio(&mut conversations, peer, false, accepted.is_some());
        let accepted = tracker.feed(incoming(
            peer,
            EventKind::AudioChunk,
            json!({}),
            vec![0; 5 * 1024 * 1024 + 1],
        ));
        take_conversation_after_audio(&mut conversations, peer, false, accepted.is_some());
        let accepted = tracker.feed(incoming(peer, EventKind::AudioStop, json!({}), vec![]));
        take_conversation_after_audio(&mut conversations, peer, true, accepted.is_some());
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    fn office() -> (Arc<RoomArbiter>, Arc<SatelliteRegistry>, RoomName) {
        let mut config = niles_config::SatellitesConfig::default();
        config.satellites.insert(
            "office".into(),
            niles_config::SatelliteConfig {
                ip: "127.0.0.1".into(),
                room: "office".into(),
                volume: 100,
            },
        );
        (
            Arc::new(RoomArbiter::default()),
            Arc::new(SatelliteRegistry::from_config(&config)),
            RoomName::parse("office").unwrap(),
        )
    }

    /// A voice turn as the dispatch task ends it: the reply written, then
    /// the room held for `conversation_hold`.
    async fn after_a_reply(playing: Duration, asked: bool) {
        let (arbiter, satellites, room) = office();
        let policy =
            CommentaryPolicy::new(arbiter.clone(), satellites, QuietHoursConfig::default());
        let owner = PluginOwner::new("synthetic-test").unwrap();
        let conversation = arbiter.enter_conversation(room.clone());
        let task = spawn_conversation_task(Some(conversation), async move {
            tokio::time::sleep(conversation_hold(playing, asked)).await;
        });
        tokio::task::yield_now().await;
        let held = conversation_hold(playing, asked);
        tokio::time::advance(held - Duration::from_millis(1)).await;
        assert_eq!(policy.check(&owner, &room).await, Eligibility::Busy);
        tokio::time::advance(Duration::from_millis(1)).await;
        task.await.unwrap();
        assert_eq!(policy.check(&owner, &room).await, Eligibility::Eligible);
    }

    #[tokio::test(start_paused = true)]
    async fn commentary_waits_out_the_reply_as_it_plays() {
        after_a_reply(Duration::from_secs(6), false).await;
    }

    #[tokio::test(start_paused = true)]
    async fn after_a_question_commentary_also_waits_out_the_answer_window_and_no_more() {
        after_a_reply(Duration::from_secs(6), true).await;
        assert_eq!(
            conversation_hold(Duration::from_secs(6), true),
            Duration::from_secs(6) + crate::follow_up::ANSWER_WINDOW
        );
        assert!(crate::follow_up::ANSWER_WINDOW < Duration::from_secs(10));
    }

    #[test]
    fn a_reply_that_never_played_holds_nothing() {
        assert_eq!(conversation_hold(Duration::ZERO, false), Duration::ZERO);
    }

    fn output(arbiter: Arc<RoomArbiter>, satellites: Arc<SatelliteRegistry>) -> CommentaryOutput {
        let piper = PiperClient::new(niles_tts::PiperConfig {
            base_url: "http://127.0.0.1:1".into(),
            default_voice: "en_GB-alan-medium".into(),
            request_timeout: Duration::from_secs(1),
        })
        .unwrap();
        CommentaryOutput::new(Arc::new(piper), satellites, arbiter)
    }

    #[tokio::test]
    async fn commentary_into_a_room_already_speaking_is_busy_not_failed() {
        let (arbiter, satellites, room) = office();
        let _other = arbiter.try_start_commentary(&room).unwrap();
        let output = output(arbiter, satellites);
        assert!(matches!(
            output.prepare(&room, "twenty").await,
            Err(SpeechOutputError::Busy)
        ));
    }

    #[tokio::test]
    async fn synthesis_refusing_the_text_is_failed() {
        let (arbiter, satellites, room) = office();
        let output = output(arbiter.clone(), satellites);
        assert!(matches!(
            output.prepare(&room, "   ").await,
            Err(SpeechOutputError::Failed)
        ));
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    struct GatedOutput {
        started: Arc<Notify>,
        gate: Arc<Notify>,
    }

    #[tokio::test]
    async fn arbitration_cancels_a_synthesis_or_delivery_wait() {
        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let output = Arc::new(GatedOutput {
            started: Arc::new(Notify::new()),
            gate: Arc::new(Notify::new()),
        });
        let output_prepare = output.clone();
        let output_deliver = output.clone();
        let output = SpeechOutput::new(
            move |_room: RoomName, _text: String| async { Ok(()) },
            move |()| {
                let output = output_deliver.clone();
                async move {
                    output.started.notify_one();
                    output.gate.notified().await;
                    Ok(())
                }
            },
        );
        let lease = arbiter.try_start_commentary(&room).unwrap();
        let started = output_prepare.started.clone();
        let task = tokio::spawn(async move {
            let mut lease = lease;
            let other = RoomName::parse("other").unwrap();
            let work = async {
                output.prepare(&other, "text").await?;
                output.deliver(()).await
            };
            tokio::select! { biased; _ = lease.cancelled() => Err(SpeechOutputError::Cancelled), result = work => result }
        });
        started.notified().await;
        let _voice = arbiter.enter_conversation(room);
        assert_eq!(task.await.unwrap(), Err(SpeechOutputError::Cancelled));
    }
}
