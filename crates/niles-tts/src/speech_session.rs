//! Bounded, host-owned text speech sessions.
//!
//! Acceptance is separate from [`TerminalResult`]. A new accepted request
//! replaces the single not-yet-transporting request, including one in prepare.
//! Expiry is checked immediately before entering `deliver`; transport that has
//! started is never replaced. Callers supply the live eligibility policy.

use niles_core::RoomName;
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio::{
    sync::{Mutex, Notify, oneshot},
    task::JoinHandle,
};

pub const MAX_PENDING_UTTERANCES: usize = 1;
pub const MAX_UTTERANCE_CHARS: usize = 120;
pub const QUEUED_UTTERANCE_TTL: Duration = Duration::from_secs(3);
pub const OWNER_RATE_LIMIT: usize = 120;
pub const OWNER_RATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_RATE_TRACKED_OWNERS: usize = 256;

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
    RateLimited,
    NoSatellite,
}
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalResult {
    DeliveredToOutput,
    Expired,
    Cancelled,
    Rejected(Rejection),
    Failed,
    Replaced,
}
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Eligibility {
    Eligible,
    Busy,
    QuietHours,
    Disabled,
    Unavailable,
    NoSatellite,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("plugin owner must not be blank")]
    InvalidOwner,
    #[error("a session already exists for this plugin owner")]
    OwnerHasSession,

    #[error("session is unavailable: {0:?}")]
    Ineligible(Rejection),
    #[error("session was closed")]
    Closed,
}

/// The output callback could not prepare or deliver an utterance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("speech output failed")]
pub struct SpeechOutputError;

pub type PrepareFuture<Prepared> =
    Pin<Box<dyn Future<Output = Result<Prepared, ()>> + Send + 'static>>;
pub type DeliverFuture = Pin<Box<dyn Future<Output = Result<(), ()>> + Send + 'static>>;
pub type EligibilityFuture = Pin<Box<dyn Future<Output = Eligibility> + Send + 'static>>;
type PrepareCallback<Prepared> =
    dyn Fn(RoomName, String) -> PrepareFuture<Prepared> + Send + Sync + 'static;
type DeliverCallback<Prepared> = dyn Fn(Prepared) -> DeliverFuture + Send + Sync + 'static;
type EligibilityCallback =
    dyn Fn(PluginOwner, RoomName) -> EligibilityFuture + Send + Sync + 'static;

/// Callable policy boundary for live speech eligibility.
pub struct SpeechEligibility {
    check: Arc<EligibilityCallback>,
}
impl SpeechEligibility {
    pub fn new<F, Fut>(check: F) -> Self
    where
        F: Fn(PluginOwner, RoomName) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Eligibility> + Send + 'static,
    {
        Self {
            check: Arc::new(move |owner, room| Box::pin(check(owner, room))),
        }
    }

    pub async fn check(&self, owner: &PluginOwner, room: &RoomName) -> Eligibility {
        (self.check)(owner.clone(), room.clone()).await
    }
}

/// Callable output boundary; successful completion means delivery completed.
pub struct SpeechOutput<Prepared: Send + 'static> {
    prepare: Arc<PrepareCallback<Prepared>>,
    deliver: Arc<DeliverCallback<Prepared>>,
}
impl<Prepared: Send + 'static> SpeechOutput<Prepared> {
    pub fn new<Prepare, PrepareFut, Deliver, DeliverFut>(prepare: Prepare, deliver: Deliver) -> Self
    where
        Prepare: Fn(RoomName, String) -> PrepareFut + Send + Sync + 'static,
        PrepareFut: Future<Output = Result<Prepared, ()>> + Send + 'static,
        Deliver: Fn(Prepared) -> DeliverFut + Send + Sync + 'static,
        DeliverFut: Future<Output = Result<(), ()>> + Send + 'static,
    {
        Self {
            prepare: Arc::new(move |room, text| Box::pin(prepare(room, text))),
            deliver: Arc::new(move |prepared| Box::pin(deliver(prepared))),
        }
    }

    pub async fn prepare(
        &self,
        room: &RoomName,
        text: &str,
    ) -> Result<Prepared, SpeechOutputError> {
        (self.prepare)(room.clone(), text.to_owned())
            .await
            .map_err(|()| SpeechOutputError)
    }

    pub async fn deliver(&self, prepared: Prepared) -> Result<(), SpeechOutputError> {
        (self.deliver)(prepared)
            .await
            .map_err(|()| SpeechOutputError)
    }
}

type Sender = Arc<std::sync::Mutex<Option<oneshot::Sender<TerminalResult>>>>;
struct Request {
    text: String,
    queued_at: tokio::time::Instant,
    result: Sender,
}
impl Drop for Request {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.result.lock()
            && let Some(sender) = slot.take()
        {
            let _ = sender.send(TerminalResult::Cancelled);
        }
    }
}
struct Session {
    owner: PluginOwner,
    room: RoomName,
    pending: Arc<std::sync::Mutex<Option<Request>>>,
    notify: Arc<Notify>,
    task: Option<JoinHandle<()>>,
}
struct State {
    shutdown: bool,
    next_id: u64,
    sessions: HashMap<SessionId, Session>,
    by_owner: HashMap<PluginOwner, SessionId>,
    rate_history: HashMap<PluginOwner, VecDeque<tokio::time::Instant>>,
}

/// Owns session tasks. Explicit async shutdown/close resolves accepted requests.
/// Dropping the manager aborts tasks synchronously; requests dropped with their
/// worker resolve as [`TerminalResult::Cancelled`].
pub struct SpeechSessionManager<Prepared: Send + 'static> {
    output: SpeechOutput<Prepared>,
    policy: SpeechEligibility,
    state: Mutex<State>,
}

fn rejection(reason: Eligibility) -> Option<Rejection> {
    match reason {
        Eligibility::Eligible => None,
        Eligibility::Busy => Some(Rejection::Busy),
        Eligibility::QuietHours => Some(Rejection::QuietHours),
        Eligibility::Disabled => Some(Rejection::Disabled),
        Eligibility::Unavailable => Some(Rejection::Unavailable),
        Eligibility::NoSatellite => Some(Rejection::NoSatellite),
    }
}
fn finish(slot: &Sender, result: TerminalResult) {
    if let Some(tx) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = tx.send(result);
    }
}

impl<Prepared: Send + 'static> SpeechSessionManager<Prepared> {
    pub fn new(output: SpeechOutput<Prepared>, policy: SpeechEligibility) -> Self {
        Self {
            output,
            policy,
            state: Mutex::new(State {
                shutdown: false,
                next_id: 1,
                sessions: HashMap::new(),
                by_owner: HashMap::new(),
                rate_history: HashMap::new(),
            }),
        }
    }

    pub async fn open(
        &self,
        owner: PluginOwner,
        room: RoomName,
    ) -> Result<SessionId, SessionError> {
        {
            let state = self.state.lock().await;
            if state.shutdown {
                return Err(SessionError::Closed);
            }
        }
        if let Some(reason) = rejection(self.policy.check(&owner, &room).await) {
            return Err(SessionError::Ineligible(reason));
        }
        let mut state = self.state.lock().await;
        if state.shutdown {
            return Err(SessionError::Closed);
        }

        if state.by_owner.contains_key(&owner) {
            return Err(SessionError::OwnerHasSession);
        }
        let id = SessionId(state.next_id);
        state.next_id = state.next_id.wrapping_add(1).max(1);
        let pending = Arc::new(std::sync::Mutex::new(None));
        let notify = Arc::new(Notify::new());
        state.by_owner.insert(owner.clone(), id);
        state.sessions.insert(
            id,
            Session {
                owner,
                room,
                pending,
                notify,
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
        let (owner, room) = {
            let state = self.state.lock().await;
            let session = state.sessions.get(&id).ok_or(Rejection::Closed)?;
            (session.owner.clone(), session.room.clone())
        };
        if let Some(reason) = rejection(self.policy.check(&owner, &room).await) {
            return Err(reason);
        }
        let mut state = self.state.lock().await;
        if state.shutdown {
            return Err(Rejection::Closed);
        }

        if !state.sessions.contains_key(&id) {
            return Err(Rejection::Closed);
        }
        let now = tokio::time::Instant::now();
        state.rate_history.retain(|_, history| {
            while history
                .front()
                .is_some_and(|at| now.duration_since(*at) >= OWNER_RATE_WINDOW)
            {
                history.pop_front();
            }
            !history.is_empty()
        });
        let history_len = state.rate_history.get(&owner).map_or(0, VecDeque::len);
        if history_len >= OWNER_RATE_LIMIT
            || (!state.rate_history.contains_key(&owner)
                && state.rate_history.len() >= MAX_RATE_TRACKED_OWNERS)
        {
            return Err(Rejection::RateLimited);
        }
        let request_id = RequestId(state.next_id);
        state.next_id = state.next_id.wrapping_add(1).max(1);
        let session = state.sessions.get_mut(&id).ok_or(Rejection::Closed)?;
        if session.task.is_none() {
            session.task = Some(tokio::spawn(session_worker(
                Arc::clone(&session.pending),
                Arc::clone(&session.notify),
                SpeechOutput {
                    prepare: Arc::clone(&self.output.prepare),
                    deliver: Arc::clone(&self.output.deliver),
                },
                SpeechEligibility {
                    check: Arc::clone(&self.policy.check),
                },
                owner.clone(),
                room,
            )));
        }
        let (tx, rx) = oneshot::channel();
        let request = Request {
            text: text.to_owned(),
            queued_at: tokio::time::Instant::now(),
            result: Arc::new(std::sync::Mutex::new(Some(tx))),
        };
        if let Some(previous) = session
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(request)
        {
            finish(&previous.result, TerminalResult::Replaced);
            drop(previous);
        }
        session.notify.notify_one();
        state.rate_history.entry(owner).or_default().push_back(now);
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
}

async fn session_worker<Prepared: Send + 'static>(
    pending: Arc<std::sync::Mutex<Option<Request>>>,
    notify: Arc<Notify>,
    output: SpeechOutput<Prepared>,
    policy: SpeechEligibility,
    owner: PluginOwner,
    room: RoomName,
) {
    loop {
        let Some(mut request) = take_pending(&pending) else {
            notify.notified().await;
            continue;
        };
        if request.queued_at.elapsed() >= QUEUED_UTTERANCE_TTL {
            finish(&request.result, TerminalResult::Expired);
            continue;
        }
        if let Some(reason) = rejection(policy.check(&owner, &room).await) {
            finish(&request.result, TerminalResult::Rejected(reason));
            continue;
        }
        let prepared = 'preparation: loop {
            tokio::select! {
                biased;
                _ = notify.notified() => {
                    if let Some(newest) = take_pending(&pending) {
                        finish(&request.result, TerminalResult::Replaced);
                        request = newest;
                        if request.queued_at.elapsed() >= QUEUED_UTTERANCE_TTL {
                            finish(&request.result, TerminalResult::Expired);
                            break 'preparation Err(SpeechOutputError);
                        }
                    }
                }
                prepared = output.prepare(&room, &request.text) => break 'preparation prepared,
            }
        };
        let Ok(prepared) = prepared else {
            finish(&request.result, TerminalResult::Failed);
            continue;
        };
        // A prepared request remains replaceable until this final expiry check
        // and synchronous entry into deliver; only then is transport active.
        if pending.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            finish(&request.result, TerminalResult::Replaced);
            drop(prepared);
            // Leave the slot in place: taking and restoring it could overwrite
            // a newer submission accepted between the two lock acquisitions.
            continue;
        }
        if request.queued_at.elapsed() >= QUEUED_UTTERANCE_TTL {
            finish(&request.result, TerminalResult::Expired);
            continue;
        }
        let terminal = match output.deliver(prepared).await {
            Ok(()) => TerminalResult::DeliveredToOutput,
            Err(SpeechOutputError) => TerminalResult::Failed,
        };
        finish(&request.result, terminal);
    }
}

fn take_pending(pending: &std::sync::Mutex<Option<Request>>) -> Option<Request> {
    pending.lock().unwrap_or_else(|e| e.into_inner()).take()
}

impl<Prepared: Send + 'static> Drop for SpeechSessionManager<Prepared> {
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
    #[tokio::test]
    async fn output_callback_failures_have_a_typed_error() {
        let output =
            super::SpeechOutput::<()>::new(|_, _| async { Err(()) }, |_| async { Err(()) });
        let room = niles_core::RoomName::parse("office").unwrap();
        assert_eq!(
            output.prepare(&room, "dart").await,
            Err(super::SpeechOutputError)
        );
        assert_eq!(output.deliver(()).await, Err(super::SpeechOutputError));
    }
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
    fn fake_output(output: Arc<FakeOutput>) -> SpeechOutput<String> {
        let prepare = |_room: RoomName, text: String| async move { Ok(text) };
        let deliver_output = output.clone();
        SpeechOutput::new(prepare, move |text| {
            let output = deliver_output.clone();
            async move {
                if let Some(started) = &output.started {
                    started.notify_one();
                }
                if let Some(gate) = &output.gate {
                    gate.notified().await;
                }
                output.delivered.lock().await.push(text);
                if output.fail { Err(()) } else { Ok(()) }
            }
        })
    }

    struct PreparationOutput {
        started: Arc<Notify>,
        gate: Arc<tokio::sync::Semaphore>,
        delivered: Arc<Mutex<Vec<String>>>,
    }
    fn preparation_output(output: Arc<PreparationOutput>) -> SpeechOutput<String> {
        let prepare_output = output.clone();
        let deliver_output = output;
        SpeechOutput::new(
            move |_room: RoomName, text: String| {
                let output = prepare_output.clone();
                async move {
                    output.started.notify_one();
                    output.gate.acquire().await.unwrap().forget();
                    Ok(text)
                }
            },
            move |text| {
                let output = deliver_output.clone();
                async move {
                    output.delivered.lock().await.push(text);
                    Ok(())
                }
            },
        )
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
    fn callable_policy(policy: Arc<Policy>) -> SpeechEligibility {
        SpeechEligibility::new(move |_: PluginOwner, _: RoomName| {
            let policy = policy.clone();
            async move {
                let check = policy.checks.fetch_add(1, Ordering::SeqCst);

                if let Some(started) = &policy.check_started
                    && check == 2
                {
                    started.notify_one();
                }
                if let Some(gate) = &policy.gate_on_check
                    && check == 2
                {
                    gate.notified().await;
                }
                policy.eligibility.lock().await.clone()
            }
        })
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
    struct TestManager {
        manager: SpeechSessionManager<String>,
        output: Arc<FakeOutput>,
        policy: Arc<Policy>,
    }
    impl std::ops::Deref for TestManager {
        type Target = SpeechSessionManager<String>;
        fn deref(&self) -> &Self::Target {
            &self.manager
        }
    }
    fn manager(output: FakeOutput, policy: Policy) -> TestManager {
        let output = Arc::new(output);
        let policy = Arc::new(policy);
        TestManager {
            manager: SpeechSessionManager::new(
                fake_output(output.clone()),
                callable_policy(policy.clone()),
            ),
            output,
            policy,
        }
    }

    #[tokio::test]
    async fn short_sentence_limit_counts_unicode_characters() {
        let m = manager(FakeOutput::default(), Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        assert_eq!(
            m.submit(session, &"ø".repeat(120))
                .await
                .unwrap()
                .1
                .await
                .unwrap(),
            TerminalResult::DeliveredToOutput
        );
        for text in ["ø".repeat(121), " ".repeat(120), String::new()] {
            assert_eq!(
                m.submit(session, &text).await.unwrap_err(),
                Rejection::InvalidText
            );
        }
        m.close(session).await;
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_survives_reopen_and_recovers_at_window_boundary() {
        let m = manager(FakeOutput::default(), Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        for _ in 0..OWNER_RATE_LIMIT {
            assert_eq!(
                m.submit(session, "dart").await.unwrap().1.await.unwrap(),
                TerminalResult::DeliveredToOutput
            );
        }
        assert_eq!(
            m.submit(session, "limited").await.unwrap_err(),
            Rejection::RateLimited
        );
        m.close(session).await;
        let reopened = m.open(owner(), room()).await.unwrap();
        assert_eq!(
            m.submit(reopened, "still limited").await.unwrap_err(),
            Rejection::RateLimited
        );
        let other = m.open(named_owner("other"), room()).await.unwrap();
        assert_eq!(
            m.submit(other, "independent")
                .await
                .unwrap()
                .1
                .await
                .unwrap(),
            TerminalResult::DeliveredToOutput
        );
        tokio::time::advance(OWNER_RATE_WINDOW - Duration::from_millis(1)).await;
        assert_eq!(
            m.submit(reopened, "too soon").await.unwrap_err(),
            Rejection::RateLimited
        );
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(
            m.submit(reopened, "recovered")
                .await
                .unwrap()
                .1
                .await
                .unwrap(),
            TerminalResult::DeliveredToOutput
        );
        assert_eq!(m.state.lock().await.rate_history[&owner()].len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_history_is_bounded_during_continuous_use() {
        let m = manager(FakeOutput::default(), Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        for _ in 0..180 {
            assert_eq!(
                m.submit(session, "dart").await.unwrap().1.await.unwrap(),
                TerminalResult::DeliveredToOutput
            );
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        assert_eq!(m.state.lock().await.rate_history[&owner()].len(), 60);
    }

    #[tokio::test(start_paused = true)]
    async fn invalid_and_denied_requests_do_not_consume_rate_allowance() {
        let m = manager(FakeOutput::default(), Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        for _ in 0..OWNER_RATE_LIMIT {
            assert_eq!(
                m.submit(session, " ").await.unwrap_err(),
                Rejection::InvalidText
            );
            m.policy.set(Eligibility::Busy).await;
            assert_eq!(
                m.submit(session, "busy").await.unwrap_err(),
                Rejection::Busy
            );
            m.policy.set(Eligibility::Eligible).await;
        }
        for _ in 0..OWNER_RATE_LIMIT {
            assert_eq!(
                m.submit(session, "allowed").await.unwrap().1.await.unwrap(),
                TerminalResult::DeliveredToOutput
            );
        }
        assert_eq!(
            m.submit(session, "limited").await.unwrap_err(),
            Rejection::RateLimited
        );
    }

    #[tokio::test]
    async fn sequential_submissions_each_deliver_after_prior_completion() {
        let m = manager(FakeOutput::default(), Policy::eligible());
        let session = m.open(owner(), room()).await.unwrap();
        let mut results = Vec::new();
        for text in ["one", "two", "three"] {
            let result = m.submit(session, text).await.unwrap().1;
            assert_eq!(result.await.unwrap(), TerminalResult::DeliveredToOutput);
            results.push(text);
        }
        assert_eq!(*m.output.delivered.lock().await, results);
    }

    #[tokio::test]
    async fn active_transport_is_preserved_and_only_latest_pending_is_delivered() {
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
        let (_, replaced) = m.submit(session, "older pending").await.unwrap();
        let (_, latest) = m.submit(session, "latest pending").await.unwrap();
        assert_eq!(replaced.await.unwrap(), TerminalResult::Replaced);
        m.output.gate.as_ref().unwrap().notify_one();
        assert_eq!(active.await.unwrap(), TerminalResult::DeliveredToOutput);
        started.notified().await;
        m.output.gate.as_ref().unwrap().notify_one();
        assert_eq!(latest.await.unwrap(), TerminalResult::DeliveredToOutput);
        m.close(session).await;
        assert_eq!(
            *m.output.delivered.lock().await,
            ["active", "latest pending"]
        );
    }

    #[tokio::test]
    async fn newest_submission_replaces_work_blocked_in_preparation() {
        let started = Arc::new(Notify::new());
        let output = Arc::new(PreparationOutput {
            started: started.clone(),
            gate: Arc::new(tokio::sync::Semaphore::new(0)),
            delivered: Arc::new(Mutex::new(Vec::new())),
        });
        let delivered = output.delivered.clone();
        let m = SpeechSessionManager::new(
            preparation_output(output.clone()),
            callable_policy(Arc::new(Policy::eligible())),
        );
        let session = m.open(owner(), room()).await.unwrap();
        let (_, old) = m.submit(session, "old synthesis").await.unwrap();
        started.notified().await;
        let (_, newest) = m.submit(session, "new synthesis").await.unwrap();
        assert_eq!(old.await.unwrap(), TerminalResult::Replaced);
        output.gate.add_permits(2);
        assert_eq!(newest.await.unwrap(), TerminalResult::DeliveredToOutput);
        assert_eq!(*delivered.lock().await, ["new synthesis"]);
    }

    #[tokio::test(start_paused = true)]
    async fn request_expiring_during_synthesis_never_enters_transport() {
        let started = Arc::new(Notify::new());
        let output = Arc::new(PreparationOutput {
            started: started.clone(),
            gate: Arc::new(tokio::sync::Semaphore::new(0)),
            delivered: Arc::new(Mutex::new(Vec::new())),
        });
        let delivered = output.delivered.clone();
        let m = SpeechSessionManager::new(
            preparation_output(output.clone()),
            callable_policy(Arc::new(Policy::eligible())),
        );
        let session = m.open(owner(), room()).await.unwrap();
        let (_, result) = m.submit(session, "stale synthesis").await.unwrap();
        started.notified().await;
        tokio::time::advance(QUEUED_UTTERANCE_TTL).await;
        output.gate.add_permits(1);
        assert_eq!(result.await.unwrap(), TerminalResult::Expired);
        assert!(delivered.lock().await.is_empty());
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
        m.close(session_a).await;
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
    }

    #[tokio::test]
    async fn manager_drop_cancels_active_and_pending_requests() {
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
        let (_, pending) = m.submit(session, "pending").await.unwrap();
        drop(m);
        assert_eq!(active.await.unwrap(), TerminalResult::Cancelled);
        assert_eq!(pending.await.unwrap(), TerminalResult::Cancelled);
    }

    #[tokio::test(start_paused = true)]
    async fn replacements_consume_quota_for_every_accepted_request() {
        let started = Arc::new(Notify::new());
        let m = manager(
            FakeOutput {
                gate: Some(Arc::new(Notify::new())),
                started: Some(started.clone()),
                ..Default::default()
            },
            Policy::eligible(),
        );
        let session = m.open(owner(), room()).await.unwrap();
        let (_, active) = m.submit(session, "active").await.unwrap();
        started.notified().await;
        let mut previous: Option<oneshot::Receiver<TerminalResult>> = None;
        for index in 1..OWNER_RATE_LIMIT {
            let (_, result) = m
                .submit(session, &format!("pending-{index}"))
                .await
                .unwrap();
            if let Some(previous) = previous.replace(result) {
                assert_eq!(previous.await.unwrap(), TerminalResult::Replaced);
            }
        }
        assert_eq!(
            m.state.lock().await.rate_history[&owner()].len(),
            OWNER_RATE_LIMIT
        );
        assert_eq!(
            m.submit(session, "over quota").await.unwrap_err(),
            Rejection::RateLimited
        );
        m.close(session).await;
        assert_eq!(active.await.unwrap(), TerminalResult::Cancelled);
        assert_eq!(previous.unwrap().await.unwrap(), TerminalResult::Cancelled);
    }

    #[tokio::test]
    async fn shutdown_is_terminal() {
        let m = manager(FakeOutput::default(), Policy::eligible());
        m.shutdown().await;
        assert_eq!(m.open(owner(), room()).await, Err(SessionError::Closed));
    }
}
