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
    speech_session::{Eligibility, PluginOwner, SpeechEligibility, SpeechOutput},
};
use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
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
        if state.conversations != 0 || state.commentary.is_some() {
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
    fn busy(&self, room: &RoomName) -> bool {
        let rooms = self.rooms.lock().unwrap_or_else(|e| e.into_inner());
        rooms
            .get(room)
            .is_some_and(|state| state.conversations != 0 || state.commentary.is_some())
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

pub(crate) fn drop_conversation_for_peer(
    conversations: &mut HashMap<SocketAddr, ConversationLease>,
    peer: SocketAddr,
) {
    conversations.remove(&peer);
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
impl SpeechEligibility for CommentaryPolicy {
    async fn check(&self, _: &PluginOwner, room: &RoomName) -> Eligibility {
        if self.quiet.is_active(Utc::now()) {
            Eligibility::QuietHours
        } else if self.satellites.ip_for(room).is_none() {
            Eligibility::Unavailable
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
impl SpeechOutput for CommentaryOutput {
    async fn render_and_deliver(&self, room: &RoomName, text: &str) -> Result<(), ()> {
        let mut lease = self.arbiter.try_start_commentary(room)?;
        let Some(ip) = self.satellites.ip_for(room) else {
            return Err(());
        };
        let work = async {
            let synthesis = self.piper.synthesize(text, None).await.map_err(|_| ())?;
            let (pcm, format) = wav_to_pcm(&synthesis.audio_wav).map_err(|_| ())?;
            let (pcm, format) = for_satellite(pcm, format);
            let peer = SocketAddr::new(ip, 0);
            let volume = self.satellites.volume_for(peer);
            let pcm = at_volume(&pcm, format.bits_per_sample, volume);
            crate::push::speak_to(ip, &pcm, format)
                .await
                .map_err(|_| ())
        };
        tokio::select! { biased; _ = lease.cancelled() => Err(()), result = work => result }
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
        let peer = "127.0.0.1:43210".parse().unwrap();
        let mut conversations = HashMap::from([(peer, arbiter.enter_conversation(room.clone()))]);

        assert!(arbiter.try_start_commentary(&room).is_err());
        drop_conversation_for_peer(&mut conversations, peer);
        assert!(arbiter.try_start_commentary(&room).is_ok());
    }

    struct GatedOutput {
        started: Arc<Notify>,
        gate: Arc<Notify>,
    }
    impl SpeechOutput for GatedOutput {
        async fn render_and_deliver(&self, _: &RoomName, _: &str) -> Result<(), ()> {
            self.started.notify_one();
            self.gate.notified().await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn arbitration_cancels_a_synthesis_or_delivery_wait() {
        let arbiter = Arc::new(RoomArbiter::default());
        let room = RoomName::parse("office").unwrap();
        let output = GatedOutput {
            started: Arc::new(Notify::new()),
            gate: Arc::new(Notify::new()),
        };
        let lease = arbiter.try_start_commentary(&room).unwrap();
        let started = output.started.clone();
        let task = tokio::spawn(async move {
            let mut lease = lease;
            let other = RoomName::parse("other").unwrap();
            tokio::select! { biased; _ = lease.cancelled() => Err(()), result = output.render_and_deliver(&other, "text") => result }
        });
        started.notified().await;
        let _voice = arbiter.enter_conversation(room);
        assert_eq!(task.await.unwrap(), Err(()));
    }
}
