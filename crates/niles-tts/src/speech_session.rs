//! Bounded, host-owned text speech sessions.
//!
//! Acceptance is separate from [`TerminalResult`]. Callers supply the live
//! eligibility policy; this module never invents a production policy.

use niles_core::RoomName;
use std::{collections::HashMap, future::Future, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::{
    sync::{Mutex, mpsc, oneshot},
    task::JoinHandle,
};

pub const MAX_PENDING_UTTERANCES: usize = 4;
pub const MAX_UTTERANCE_CHARS: usize = 256;
pub const QUEUED_UTTERANCE_TTL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PluginOwner(String);
impl PluginOwner {
    pub fn new(value: impl Into<String>) -> Result<Self, SessionError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(SessionError::InvalidOwner);
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestId(u64);

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    Disabled,
    Busy,
    QuietHours,
    Unavailable,
    QueueFull,
    Closed,
    InvalidText,
}
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalResult {
    DeliveredToOutput,
    Expired,
    Cancelled,
    Rejected(Rejection),
    Failed,
}
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Eligibility {
    Eligible,
    Busy,
    QuietHours,
    Disabled,
    Unavailable,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("plugin owner must not be blank")]
    InvalidOwner,
    #[error("a session already exists for this plugin owner")]
    OwnerHasSession,
    #[error("speech is disabled")]
    Disabled,
    #[error("session is unavailable: {0:?}")]
    Ineligible(Rejection),
    #[error("session was closed")]
    Closed,
}

/// Supplies live policy when opening, submitting, and immediately before delivery.
pub trait SpeechEligibility: Send + Sync + 'static {
    fn check(
        &self,
        owner: &PluginOwner,
        room: &RoomName,
    ) -> impl Future<Output = Eligibility> + Send;
}
/// Injectable output boundary; successful completion means delivery operation completed.
pub trait SpeechOutput: Send + Sync + 'static {
    fn render_and_deliver(
        &self,
        room: &RoomName,
        text: &str,
    ) -> impl Future<Output = Result<(), ()>> + Send;
}

type Sender = Arc<Mutex<Option<oneshot::Sender<TerminalResult>>>>;
struct Request {
    text: String,
    queued_at: tokio::time::Instant,
    result: Sender,
}
impl Drop for Request {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.result.try_lock()
            && let Some(sender) = slot.take()
        {
            let _ = sender.send(TerminalResult::Cancelled);
        }
    }
}
struct Session {
    owner: PluginOwner,
    room: RoomName,
    sender: mpsc::Sender<Request>,
    receiver: Option<mpsc::Receiver<Request>>,
    task: Option<JoinHandle<()>>,
}
struct State {
    enabled: bool,
    shutdown: bool,
    next_id: u64,
    sessions: HashMap<SessionId, Session>,
    by_owner: HashMap<PluginOwner, SessionId>,
}

/// Owns session tasks. Explicit async shutdown/close resolves accepted requests.
/// Dropping the manager aborts tasks synchronously; requests dropped with their
/// worker resolve as [`TerminalResult::Cancelled`].
pub struct SpeechSessionManager<O: SpeechOutput, P: SpeechEligibility> {
    output: Arc<O>,
    policy: Arc<P>,
    state: Mutex<State>,
}

fn rejection(reason: Eligibility) -> Option<Rejection> {
    match reason {
        Eligibility::Eligible => None,
        Eligibility::Busy => Some(Rejection::Busy),
        Eligibility::QuietHours => Some(Rejection::QuietHours),
        Eligibility::Disabled => Some(Rejection::Disabled),
        Eligibility::Unavailable => Some(Rejection::Unavailable),
    }
}
async fn finish(slot: &Sender, result: TerminalResult) {
    if let Some(tx) = slot.lock().await.take() {
        let _ = tx.send(result);
    }
}

impl<O: SpeechOutput, P: SpeechEligibility> SpeechSessionManager<O, P> {
    pub fn new(enabled: bool, output: O, policy: P) -> Self {
        Self {
            output: Arc::new(output),
            policy: Arc::new(policy),
            state: Mutex::new(State {
                enabled,
                shutdown: false,
                next_id: 1,
                sessions: HashMap::new(),
                by_owner: HashMap::new(),
            }),
        }
    }

    pub async fn set_enabled(&self, enabled: bool) {
        let ids = {
            let mut state = self.state.lock().await;
            state.enabled = enabled;
            if enabled {
                Vec::new()
            } else {
                state.sessions.keys().copied().collect()
            }
        };
        for id in ids {
            self.close(id).await;
        }
    }

    pub async fn open(
        &self,
        owner: PluginOwner,
        room: RoomName,
    ) -> Result<SessionId, SessionError> {
        let enabled = {
            let state = self.state.lock().await;
            if state.shutdown {
                return Err(SessionError::Closed);
            }
            state.enabled
        };
        if !enabled {
            return Err(SessionError::Disabled);
        }
        if let Some(reason) = rejection(self.policy.check(&owner, &room).await) {
            return Err(SessionError::Ineligible(reason));
        }
        let mut state = self.state.lock().await;
        if state.shutdown {
            return Err(SessionError::Closed);
        }
        if !state.enabled {
            return Err(SessionError::Disabled);
        }
        if state.by_owner.contains_key(&owner) {
            return Err(SessionError::OwnerHasSession);
        }
        let id = SessionId(state.next_id);
        state.next_id = state.next_id.wrapping_add(1).max(1);
        let (sender, receiver) = mpsc::channel(MAX_PENDING_UTTERANCES);
        state.by_owner.insert(owner.clone(), id);
        state.sessions.insert(
            id,
            Session {
                owner,
                room,
                sender,
                receiver: Some(receiver),
                task: None,
            },
        );
        Ok(id)
    }

    pub async fn submit(
        &self,
        id: SessionId,
        text: &str,
    ) -> Result<(RequestId, oneshot::Receiver<TerminalResult>), Rejection> {
        if text.trim().is_empty() || text.chars().count() > MAX_UTTERANCE_CHARS {
            return Err(Rejection::InvalidText);
        }
        let (owner, room, enabled) = {
            let state = self.state.lock().await;
            let session = state.sessions.get(&id).ok_or(Rejection::Closed)?;
            (
                session.owner.clone(),
                session.room.clone(),
                state.enabled && !state.shutdown,
            )
        };
        if !enabled {
            return Err(Rejection::Disabled);
        }
        if let Some(reason) = rejection(self.policy.check(&owner, &room).await) {
            return Err(reason);
        }
        let mut state = self.state.lock().await;
        if state.shutdown {
            return Err(Rejection::Closed);
        }
        if !state.enabled {
            return Err(Rejection::Disabled);
        }
        let request_id = RequestId(state.next_id);
        state.next_id = state.next_id.wrapping_add(1).max(1);
        let session = state.sessions.get_mut(&id).ok_or(Rejection::Closed)?;
        if session.task.is_none() {
            let receiver = session.receiver.take().ok_or(Rejection::Closed)?;
            session.task = Some(tokio::spawn(session_worker(
                receiver,
                Arc::clone(&self.output),
                Arc::clone(&self.policy),
                owner,
                room,
            )));
        }
        let (tx, rx) = oneshot::channel();
        let request = Request {
            text: text.to_owned(),
            queued_at: tokio::time::Instant::now(),
            result: Arc::new(Mutex::new(Some(tx))),
        };
        session
            .sender
            .try_send(request)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => Rejection::QueueFull,
                mpsc::error::TrySendError::Closed(_) => Rejection::Closed,
            })?;
        Ok((request_id, rx))
    }

    pub async fn close(&self, id: SessionId) {
        let mut state = self.state.lock().await;
        if let Some(mut session) = state.sessions.remove(&id) {
            if let Some(task) = session.task.take() {
                task.abort();
                let _ = task.await;
            }
            state.by_owner.remove(&session.owner);
        }
    }

    pub async fn shutdown_owner(&self, owner: &PluginOwner) {
        let id = self.state.lock().await.by_owner.get(owner).copied();
        if let Some(id) = id {
            self.close(id).await;
        }
    }

    pub async fn shutdown(&self) {
        let mut state = self.state.lock().await;
        state.shutdown = true;
        let sessions = std::mem::take(&mut state.sessions);
        for (_, mut session) in sessions {
            if let Some(task) = session.task.take() {
                task.abort();
                let _ = task.await;
            }
        }
        state.by_owner.clear();
    }

    pub async fn room(&self, id: SessionId) -> Option<RoomName> {
        self.state
            .lock()
            .await
            .sessions
            .get(&id)
            .map(|session| session.room.clone())
    }
}

async fn session_worker<O: SpeechOutput, P: SpeechEligibility>(
    mut receiver: mpsc::Receiver<Request>,
    output: Arc<O>,
    policy: Arc<P>,
    owner: PluginOwner,
    room: RoomName,
) {
    while let Some(request) = receiver.recv().await {
        if request.queued_at.elapsed() >= QUEUED_UTTERANCE_TTL {
            finish(&request.result, TerminalResult::Expired).await;
            continue;
        }
        if let Some(reason) = rejection(policy.check(&owner, &room).await) {
            finish(&request.result, TerminalResult::Rejected(reason)).await;
            continue;
        }
        if request.queued_at.elapsed() >= QUEUED_UTTERANCE_TTL {
            finish(&request.result, TerminalResult::Expired).await;
            continue;
        }
        let terminal = match output.render_and_deliver(&room, &request.text).await {
            Ok(()) => TerminalResult::DeliveredToOutput,
            Err(()) => TerminalResult::Failed,
        };
        finish(&request.result, terminal).await;
    }
}

impl<O: SpeechOutput, P: SpeechEligibility> Drop for SpeechSessionManager<O, P> {
    fn drop(&mut self) {
        if let Ok(state) = self.state.try_lock() {
            for session in state.sessions.values() {
                if let Some(task) = &session.task {
                    task.abort();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    #[derive(Default)]
    struct FakeOutput {
        delivered: Mutex<Vec<String>>,
        gate: Option<Arc<Notify>>,
        started: Option<Arc<Notify>>,
        fail: bool,
    }
    impl SpeechOutput for FakeOutput {
        async fn render_and_deliver(&self, _: &RoomName, text: &str) -> Result<(), ()> {
            if let Some(started) = &self.started {
                started.notify_one();
            }
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            self.delivered.lock().await.push(text.into());
            if self.fail { Err(()) } else { Ok(()) }
        }
    }

    struct Policy {
        eligibility: Mutex<Eligibility>,
        checks: AtomicUsize,
        gate_on_check: Option<Arc<Notify>>,
        check_started: Option<Arc<Notify>>,
    }
    impl Policy {
        fn eligible() -> Self {
            Self {
                eligibility: Mutex::new(Eligibility::Eligible),
                checks: AtomicUsize::new(0),
                gate_on_check: None,
                check_started: None,
            }
        }
        async fn set(&self, eligibility: Eligibility) {
            *self.eligibility.lock().await = eligibility;
        }
    }
    impl SpeechEligibility for Policy {
        async fn check(&self, _: &PluginOwner, _: &RoomName) -> Eligibility {
            let check = self.checks.fetch_add(1, Ordering::SeqCst);

            if let Some(started) = &self.check_started
                && check == 2
            {
                started.notify_one();
            }
            if let Some(gate) = &self.gate_on_check
                && check == 2
            {
                gate.notified().await;
            }
            self.eligibility.lock().await.clone()
        }
    }

    fn owner() -> PluginOwner {
        PluginOwner::new("plugin").unwrap()
    }
    fn named_owner(name: &str) -> PluginOwner {
        PluginOwner::new(name).unwrap()
    }
    fn room() -> RoomName {
        RoomName::parse("office").unwrap()
    }
    fn manager(output: FakeOutput, policy: Policy) -> SpeechSessionManager<FakeOutput, Policy> {
        SpeechSessionManager::new(true, output, policy)
    }

    #[tokio::test]
    async fn accepted_requests_are_delivered_in_order_through_output_boundary() {
        let m = manager(FakeOutput::default(), Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        let mut results = Vec::new();
        for text in ["one", "two", "three"] {
            results.push(m.submit(session, text).await.unwrap().1);
        }
        for result in results {
            assert_eq!(result.await.unwrap(), TerminalResult::DeliveredToOutput);
        }
        assert_eq!(*m.output.delivered.lock().await, ["one", "two", "three"]);
    }

    #[tokio::test]
    async fn active_request_plus_four_queued_are_allowed_and_fifth_queued_is_rejected() {
        let started = Arc::new(Notify::new());
        let output = FakeOutput {
            gate: Some(Arc::new(Notify::new())),
            started: Some(started.clone()),
            ..Default::default()
        };
        let m = manager(output, Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        let (_, active) = m.submit(session, "active").await.unwrap();
        started.notified().await;
        let mut queued = Vec::new();
        for index in 0..MAX_PENDING_UTTERANCES {
            queued.push(
                m.submit(session, &format!("queued-{index}"))
                    .await
                    .unwrap()
                    .1,
            );
        }
        assert_eq!(
            m.submit(session, "fifth").await.unwrap_err(),
            Rejection::QueueFull
        );
        m.close(session).await;
        assert_eq!(active.await.unwrap(), TerminalResult::Cancelled);
        for result in queued {
            assert_eq!(result.await.unwrap(), TerminalResult::Cancelled);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn queued_request_expires_before_it_can_be_delivered() {
        let started = Arc::new(Notify::new());
        let output = FakeOutput {
            gate: Some(Arc::new(Notify::new())),
            started: Some(started.clone()),
            ..Default::default()
        };
        let m = manager(output, Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        let (_, active) = m.submit(session, "active").await.unwrap();
        started.notified().await;
        let (_, queued) = m.submit(session, "stale").await.unwrap();
        tokio::time::advance(QUEUED_UTTERANCE_TTL).await;
        m.output.gate.as_ref().unwrap().notify_one();
        assert_eq!(active.await.unwrap(), TerminalResult::DeliveredToOutput);
        assert_eq!(queued.await.unwrap(), TerminalResult::Expired);
        assert_eq!(*m.output.delivered.lock().await, ["active"]);
    }

    #[tokio::test]
    async fn policy_await_cannot_make_expired_utterance_reach_output() {
        let policy_gate = Arc::new(Notify::new());
        let policy = Policy {
            gate_on_check: Some(policy_gate.clone()),
            ..Policy::eligible()
        };
        let m = manager(FakeOutput::default(), policy);
        let session = m.open(owner(), room()).await.unwrap();
        let (_, result) = m.submit(session, "late").await.unwrap();
        while m.policy.checks.load(Ordering::SeqCst) < 3 {
            tokio::task::yield_now().await;
        }
        tokio::time::pause();
        tokio::time::advance(QUEUED_UTTERANCE_TTL).await;
        policy_gate.notify_one();
        assert_eq!(result.await.unwrap(), TerminalResult::Expired);
        assert!(m.output.delivered.lock().await.is_empty());
    }

    #[tokio::test]
    async fn policy_denial_at_open_maps_every_live_eligibility_reason() {
        for (eligibility, rejection) in [
            (Eligibility::Busy, Rejection::Busy),
            (Eligibility::QuietHours, Rejection::QuietHours),
            (Eligibility::Disabled, Rejection::Disabled),
            (Eligibility::Unavailable, Rejection::Unavailable),
        ] {
            let policy = Policy::eligible();
            policy.set(eligibility).await;
            let m = manager(FakeOutput::default(), policy);
            assert_eq!(
                m.open(owner(), room()).await,
                Err(SessionError::Ineligible(rejection))
            );
        }
    }

    #[tokio::test]
    async fn policy_denial_at_submit_returns_each_live_rejection() {
        for (eligibility, rejection) in [
            (Eligibility::Busy, Rejection::Busy),
            (Eligibility::QuietHours, Rejection::QuietHours),
            (Eligibility::Disabled, Rejection::Disabled),
            (Eligibility::Unavailable, Rejection::Unavailable),
        ] {
            let m = manager(FakeOutput::default(), Policy::eligible());
            let session = m.open(owner(), room()).await.unwrap();
            m.policy.set(eligibility).await;
            assert_eq!(m.submit(session, "text").await.unwrap_err(), rejection);
        }
    }

    #[tokio::test]
    async fn policy_denial_before_delivery_returns_each_live_terminal_rejection() {
        for (eligibility, rejection) in [
            (Eligibility::Busy, Rejection::Busy),
            (Eligibility::QuietHours, Rejection::QuietHours),
            (Eligibility::Disabled, Rejection::Disabled),
            (Eligibility::Unavailable, Rejection::Unavailable),
        ] {
            let policy_started = Arc::new(Notify::new());
            let policy_gate = Arc::new(Notify::new());
            let policy = Policy {
                gate_on_check: Some(policy_gate.clone()),
                check_started: Some(policy_started.clone()),
                ..Policy::eligible()
            };
            let output = FakeOutput::default();
            let m = manager(output, policy);
            let session = m.open(owner(), room()).await.unwrap();
            let (_, result) = m.submit(session, "text").await.unwrap();
            policy_started.notified().await;
            m.policy.set(eligibility).await;
            policy_gate.notify_one();
            assert_eq!(result.await.unwrap(), TerminalResult::Rejected(rejection));
            assert!(m.output.delivered.lock().await.is_empty());
        }
    }

    #[tokio::test]
    async fn output_failure_is_observable() {
        let m = manager(
            FakeOutput {
                fail: true,
                ..Default::default()
            },
            Policy::eligible(),
        );
        let session = m.open(owner(), room()).await.unwrap();
        let (_, result) = m.submit(session, "fails").await.unwrap();
        assert_eq!(result.await.unwrap(), TerminalResult::Failed);
        assert_eq!(*m.output.delivered.lock().await, ["fails"]);
    }

    #[tokio::test]
    async fn disabling_closes_sessions_and_rejects_new_open() {
        let started = Arc::new(Notify::new());
        let output = FakeOutput {
            gate: Some(Arc::new(Notify::new())),
            started: Some(started.clone()),
            ..Default::default()
        };
        let m = manager(output, Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        let (_, result) = m.submit(session, "active").await.unwrap();
        started.notified().await;
        m.set_enabled(false).await;
        assert_eq!(result.await.unwrap(), TerminalResult::Cancelled);
        assert_eq!(m.open(owner(), room()).await, Err(SessionError::Disabled));
        assert_eq!(
            m.submit(session, "later").await.unwrap_err(),
            Rejection::Closed
        );
    }

    #[tokio::test]
    async fn owner_shutdown_cancels_requests_without_affecting_another_owner() {
        let started = Arc::new(Notify::new());
        let output = FakeOutput {
            gate: Some(Arc::new(Notify::new())),
            started: Some(started.clone()),
            ..Default::default()
        };
        let m = manager(output, Policy::eligible());
        let owner_a = named_owner("a");
        let owner_b = named_owner("b");
        let session_a = m.open(owner_a.clone(), room()).await.unwrap();
        let session_b = m.open(owner_b.clone(), room()).await.unwrap();
        let (_, cancelled) = m.submit(session_a, "a").await.unwrap();
        started.notified().await;
        let (_, kept) = m.submit(session_b, "b").await.unwrap();
        m.shutdown_owner(&owner_a).await;
        assert_eq!(cancelled.await.unwrap(), TerminalResult::Cancelled);
        m.output.gate.as_ref().unwrap().notify_one();
        assert_eq!(kept.await.unwrap(), TerminalResult::DeliveredToOutput);
        assert_eq!(*m.output.delivered.lock().await, ["b"]);
        assert!(m.open(owner_a, room()).await.is_ok());
    }

    #[tokio::test]
    async fn close_finishes_before_same_owner_can_reopen_and_closed_session_never_delivers() {
        let started = Arc::new(Notify::new());
        let output = FakeOutput {
            gate: Some(Arc::new(Notify::new())),
            started: Some(started.clone()),
            ..Default::default()
        };
        let m = manager(output, Policy::eligible());
        let plugin = owner();
        let session = m.open(plugin.clone(), room()).await.unwrap();
        let (_, active) = m.submit(session, "do-not-deliver").await.unwrap();
        started.notified().await;
        assert_eq!(
            m.open(plugin.clone(), room()).await,
            Err(SessionError::OwnerHasSession)
        );
        m.close(session).await;
        assert!(m.open(plugin.clone(), room()).await.is_ok());
        assert_eq!(active.await.unwrap(), TerminalResult::Cancelled);
        assert!(m.output.delivered.lock().await.is_empty());
    }

    #[tokio::test]
    async fn independent_owners_can_each_open_a_session() {
        let m = manager(FakeOutput::default(), Policy::eligible());
        let a = m.open(named_owner("a"), room()).await.unwrap();
        let b = m.open(named_owner("b"), room()).await.unwrap();
        assert_ne!(a, b);
        assert_eq!(m.room(a).await, Some(room()));
        assert_eq!(m.room(b).await, Some(room()));
    }

    #[tokio::test]
    async fn manager_drop_cancels_active_and_queued_requests() {
        let started = Arc::new(Notify::new());
        let output = FakeOutput {
            gate: Some(Arc::new(Notify::new())),
            started: Some(started.clone()),
            ..Default::default()
        };
        let m = manager(output, Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        let (_, active) = m.submit(session, "active").await.unwrap();
        started.notified().await;
        let (_, queued) = m.submit(session, "queued").await.unwrap();
        drop(m);
        assert_eq!(active.await.unwrap(), TerminalResult::Cancelled);
        assert_eq!(queued.await.unwrap(), TerminalResult::Cancelled);
    }

    #[tokio::test]
    async fn shutdown_is_terminal() {
        let m = manager(FakeOutput::default(), Policy::eligible());
        m.shutdown().await;
        assert_eq!(m.open(owner(), room()).await, Err(SessionError::Closed));
    }
}
