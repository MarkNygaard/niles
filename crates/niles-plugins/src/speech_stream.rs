//! Adapter from plugin-owned speech-stream sessions to the injectable TTS session manager.
use crate::host::HostState;
use crate::{
    bindings::speech_stream,
    manifest::{SettingKind, SettingValue},
};
use niles_core::RoomName;
use niles_tts::speech_session::{
    PluginOwner, Rejection, SessionError, SpeechSessionManager, TerminalResult,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::runtime::Handle;

use speech_stream::{HostError, Outcome, RequestStatus};

#[cfg(test)]
#[path = "speech_stream_tests.rs"]
mod tests;

pub const ENABLE_SETTING: &str = "speech_stream_enabled";
pub const MAX_RETAINED_RESULTS: usize = 32;
pub const MAX_LIVE_SESSIONS: usize = 64;

/// Host-owned service boundary. Implementations must enqueue and return; they
/// may not synthesize or wait for speaker/transport operations on this call.
pub trait SpeechStreamHost: Send + Sync + 'static {
    fn open(&self, owner: &str, room: RoomName) -> Result<u32, SpeechError>;
    fn submit(&self, owner: &str, session: u32, text: String) -> Result<u64, SpeechError>;
    fn poll(&self, owner: &str, session: u32) -> Result<Vec<(u64, SpeechOutcome)>, SpeechError>;
    fn close(&self, owner: &str, session: u32);
    fn close_owner(&self, owner: &str);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpeechOutcome {
    Pending,
    Delivered,
    Busy,
    Quiet,
    Expired,
    Failed,
    Cancelled,
    Replaced,
    RateLimited,
    NoSatellite,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpeechError {
    Busy,
    NoSatellite(RoomName),
    Limit,
    Invalid,
    Closed,
    NotOwned,
}

/// Production adapter; explicitly constructed/injected with the shared
/// `niles-tts` manager. No default service or fallback is created by plugins.
pub struct SessionSpeechHost<Prepared: Send + 'static> {
    manager: Arc<SpeechSessionManager<Prepared>>,
    handle: Handle,
    route_is_configured: Arc<dyn Fn(&RoomName) -> bool + Send + Sync>,
    sessions: Mutex<BTreeMap<(String, u32), Arc<SessionSlot>>>,
    next_session: Mutex<u32>,
    next_request: Mutex<u64>,
    owner_token: String,
}
struct SessionSlot {
    accepting: AtomicBool,
    commands: tokio::sync::mpsc::Sender<Command>,
    results: Arc<Mutex<VecDeque<(u64, SpeechOutcome)>>>,
    cancel: tokio::sync::watch::Sender<bool>,
    finished: Arc<AtomicBool>,
}
enum Command {
    Submit {
        request: u64,
        text: String,
        deadline: tokio::time::Instant,
    },
}
impl<Prepared: Send + 'static> SessionSpeechHost<Prepared> {
    #[cfg(test)]
    fn new(manager: Arc<SpeechSessionManager<Prepared>>, handle: Handle) -> Self {
        Self::with_route_validator(manager, handle, |_| true)
    }

    pub fn with_route_validator(
        manager: Arc<SpeechSessionManager<Prepared>>,
        handle: Handle,
        route_is_configured: impl Fn(&RoomName) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            manager,
            handle,
            route_is_configured: Arc::new(route_is_configured),
            sessions: Mutex::new(BTreeMap::new()),
            next_session: Mutex::new(1),
            next_request: Mutex::new(1),
            owner_token: format!("plugin-instance-{}", rand::random::<u128>()),
        }
    }
}
fn push_status(results: &Mutex<VecDeque<(u64, SpeechOutcome)>>, item: (u64, SpeechOutcome)) {
    let mut results = results.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = results.iter_mut().find(|entry| entry.0 == item.0) {
        existing.1 = item.1;
    }
}
/// What already finished keeps its outcome when a session closes: an
/// utterance spoken just before the close is `delivered`, not `cancelled`.
fn keep_finished(
    completions: &mut tokio::task::JoinSet<(u64, SpeechOutcome)>,
    results: &Mutex<VecDeque<(u64, SpeechOutcome)>>,
) {
    while let Some(finished) = completions.try_join_next() {
        if let Ok(item) = finished {
            push_status(results, item);
        }
    }
}
fn map_rejection(reason: Rejection) -> SpeechOutcome {
    match reason {
        Rejection::Busy => SpeechOutcome::Busy,
        Rejection::QuietHours => SpeechOutcome::Quiet,
        Rejection::NoSatellite => SpeechOutcome::NoSatellite,
        Rejection::RateLimited => SpeechOutcome::RateLimited,
        Rejection::Closed => SpeechOutcome::Cancelled,
        _ => SpeechOutcome::Failed,
    }
}
fn map_terminal(result: TerminalResult) -> SpeechOutcome {
    match result {
        TerminalResult::DeliveredToOutput => SpeechOutcome::Delivered,
        TerminalResult::Rejected(reason) => map_rejection(reason),
        TerminalResult::Cancelled => SpeechOutcome::Cancelled,
        TerminalResult::Replaced => SpeechOutcome::Replaced,
        TerminalResult::Expired => SpeechOutcome::Expired,
        _ => SpeechOutcome::Failed,
    }
}
fn map_session_outcome(error: SessionError) -> SpeechOutcome {
    match error {
        SessionError::Ineligible(reason) => map_rejection(reason),
        SessionError::OwnerHasSession => SpeechOutcome::Busy,
        _ => SpeechOutcome::Failed,
    }
}
fn map_error(error: SpeechError) -> HostError {
    match error {
        SpeechError::Busy => HostError::Unavailable("speech output is busy".into()),
        SpeechError::NoSatellite(room) => {
            HostError::Unavailable(format!("no satellite is configured for room {room}"))
        }
        SpeechError::Limit => HostError::Limit("speech stream is at capacity".into()),
        SpeechError::Invalid => HostError::Invalid("invalid room or speech text".into()),
        SpeechError::Closed => HostError::Unavailable("speech stream session is closed".into()),
        SpeechError::NotOwned => {
            HostError::NotPermitted("session does not belong to this plugin".into())
        }
    }
}
impl<Prepared: Send + 'static> SessionSpeechHost<Prepared> {
    fn owned_slot(&self, owner: &str, session: u32) -> Result<Arc<SessionSlot>, SpeechError> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(owner.to_owned(), session))
            .cloned()
            .ok_or(SpeechError::NotOwned)
    }

    fn allocate_session(&self, used: impl Fn(u32) -> bool) -> Result<u32, SpeechError> {
        let mut next = self.next_session.lock().unwrap_or_else(|e| e.into_inner());
        let start = *next;
        loop {
            let candidate = *next;
            *next = next.wrapping_add(1).max(1);
            if !used(candidate) {
                return Ok(candidate);
            }
            if *next == start {
                return Err(SpeechError::Limit);
            }
        }
    }
}
impl<Prepared: Send + 'static> SpeechStreamHost for SessionSpeechHost<Prepared> {
    fn open(&self, owner: &str, room: RoomName) -> Result<u32, SpeechError> {
        if !(self.route_is_configured)(&room) {
            return Err(SpeechError::NoSatellite(room));
        }
        let owner_id = PluginOwner::new(format!("{}:{owner}", self.owner_token))
            .map_err(|_| SpeechError::Invalid)?;
        let raw;
        let (tx, mut rx) = tokio::sync::mpsc::channel(MAX_RETAINED_RESULTS);
        let results = Arc::new(Mutex::new(VecDeque::new()));
        let (cancel, mut cancelled) = tokio::sync::watch::channel(false);
        let finished = Arc::new(AtomicBool::new(false));
        {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions.retain(|_, slot| {
                !slot.finished.load(Ordering::Acquire)
                    || !slot
                        .results
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_empty()
            });
            if sessions.len() >= MAX_LIVE_SESSIONS {
                return Err(SpeechError::Limit);
            }
            if sessions.iter().any(|((existing, _), slot)| {
                existing == owner && !slot.finished.load(Ordering::Acquire)
            }) {
                return Err(SpeechError::Busy);
            }
            raw =
                self.allocate_session(|candidate| sessions.keys().any(|(_, id)| *id == candidate))?;
            let key = (owner.to_owned(), raw);
            let slot = Arc::new(SessionSlot {
                accepting: AtomicBool::new(true),
                commands: tx,
                results: results.clone(),
                cancel,
                finished: finished.clone(),
            });
            sessions.insert(key, slot);
        }
        let manager = Arc::clone(&self.manager);
        self.handle.spawn(async move {
            let mut id = None;
            let mut completions = tokio::task::JoinSet::new();
            let work = async {
                loop {
                    if rx.is_closed() && completions.is_empty() {
                        break;
                    }
                    tokio::select! {
                        command = rx.recv(), if !rx.is_closed() => {
                            let Some(Command::Submit { request, text, deadline }) = command else { continue; };
                            if tokio::time::Instant::now() >= deadline {
                                push_status(&results, (request, SpeechOutcome::Expired));
                                continue;
                            }
                            if id.is_none() {
                                match manager.open(owner_id.clone(), room.clone()).await {
                                    Ok(opened) => id = Some(opened),
                                    Err(error) => {
                                        push_status(&results, (request, map_session_outcome(error)));
                                        continue;
                                    }
                                }
                            }
                            match tokio::time::timeout_at(deadline, manager.submit(id.expect("opened just above"), &text)).await {
                                Ok(Ok((_, terminal))) => {
                                    completions.spawn(async move {
                                        let outcome = terminal.await.map(map_terminal).unwrap_or(SpeechOutcome::Cancelled);
                                        (request, outcome)
                                    });
                                }
                                Ok(Err(reason)) => push_status(&results, (request, map_rejection(reason))),
                                Err(_) => push_status(&results, (request, SpeechOutcome::Expired)),
                            }
                        }
                        Some(Ok((request, outcome))) = completions.join_next(), if !completions.is_empty() => {
                            push_status(&results, (request, outcome));
                        }
                    }
                }
            };
            tokio::select! {
                biased;
                _ = cancelled.changed() => {},
                () = work => {},
            }
            keep_finished(&mut completions, &results);
            completions.abort_all();
            if let Some(id) = id {
                manager.close(id).await;
            }
            for (_, outcome) in results.lock().unwrap_or_else(|e| e.into_inner()).iter_mut() {
                if *outcome == SpeechOutcome::Pending {
                    *outcome = SpeechOutcome::Cancelled;
                }
            }
            finished.store(true, Ordering::Release);
        });
        Ok(raw)
    }
    fn submit(&self, owner: &str, session: u32, text: String) -> Result<u64, SpeechError> {
        let slot = self.owned_slot(owner, session)?;
        let mut queue = slot.results.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= MAX_RETAINED_RESULTS {
            return Err(SpeechError::Limit);
        }
        if !slot.accepting.load(Ordering::Acquire) {
            return Err(SpeechError::Closed);
        }
        let raw = {
            let mut next = self.next_request.lock().unwrap_or_else(|e| e.into_inner());
            let raw = *next;
            *next = next.checked_add(1).ok_or(SpeechError::Limit)?;
            raw
        };
        slot.commands
            .try_send(Command::Submit {
                request: raw,
                text,
                deadline: tokio::time::Instant::now()
                    + niles_tts::speech_session::QUEUED_UTTERANCE_TTL,
            })
            .map_err(|_| SpeechError::Limit)?;
        queue.push_back((raw, SpeechOutcome::Pending));
        Ok(raw)
    }
    fn poll(&self, owner: &str, session: u32) -> Result<Vec<(u64, SpeechOutcome)>, SpeechError> {
        let slot = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(owner.to_owned(), session))
            .cloned()
            .ok_or(SpeechError::NotOwned)?;
        let mut queue = slot.results.lock().unwrap_or_else(|e| e.into_inner());
        let mut ready = Vec::new();
        while queue
            .front()
            .is_some_and(|(_, status)| *status != SpeechOutcome::Pending)
        {
            ready.extend(queue.pop_front());
        }
        Ok(ready)
    }
    fn close(&self, owner: &str, session: u32) {
        if let Some(slot) = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(owner.to_owned(), session))
        {
            slot.accepting.store(false, Ordering::Release);
            let _queue = slot.results.lock().unwrap_or_else(|e| e.into_inner());
            let _ = slot.cancel.send(true);
        }
    }
    fn close_owner(&self, owner: &str) {
        let keys: Vec<_> = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .filter(|(id, _)| id == owner)
            .cloned()
            .collect();
        for (_, session) in keys {
            self.close(owner, session);
        }
        // No guest remains to poll these results. Keep the closing reservation
        // until its actor finishes, so a replacement cannot overlap output.
        for ((id, _), slot) in self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if id == owner {
                slot.results
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
            }
        }
    }
}

impl Drop for HostState {
    fn drop(&mut self) {
        if let Some(requests) = self.house.notification_requests.get() {
            requests.close_owner(&self.plugin.id, self.notification_generation);
        }
        if let Some(service) = self.house.speech_stream.get() {
            service.close_owner(&self.plugin.id);
        }
    }
}

impl HostState {
    fn speech_enabled(&self) -> Result<(), HostError> {
        if self.plugin.manifest.permissions.speech_stream.is_none() {
            return Err(HostError::NotPermitted(
                "the manifest does not ask for speech_stream".into(),
            ));
        }
        let setting = self
            .plugin
            .manifest
            .setting(ENABLE_SETTING)
            .ok_or_else(|| {
                HostError::Invalid(format!(
                    "speech_stream requires a boolean {ENABLE_SETTING} setting"
                ))
            })?;
        if setting.kind != SettingKind::Boolean
            || setting
                .default
                .as_ref()
                .is_some_and(|v| !matches!(v, toml::Value::Boolean(false)))
        {
            return Err(HostError::Invalid(format!(
                "{ENABLE_SETTING} must be a boolean setting with no default or default = false"
            )));
        }
        if !matches!(
            setting.value(&self.settings),
            Some(SettingValue::Boolean(true))
        ) {
            return Err(HostError::Unavailable(
                "speech stream is disabled by the household".into(),
            ));
        }
        Ok(())
    }
}
impl speech_stream::Host for HostState {
    fn open(&mut self, room: String) -> Result<u32, HostError> {
        self.speech_enabled()?;
        let grant = self
            .plugin
            .manifest
            .permissions
            .speech_stream
            .as_ref()
            .ok_or_else(|| {
                HostError::NotPermitted("the manifest does not ask for speech_stream".into())
            })?;
        let setting = self.plugin.manifest.setting(&grant.room).ok_or_else(|| {
            HostError::Invalid("speech_stream room setting is not declared".into())
        })?;
        let configured = setting
            .value(&self.settings)
            .and_then(|value| match value {
                SettingValue::Text(room) => Some(room),
                _ => None,
            })
            .ok_or_else(|| {
                HostError::Unavailable("the household has not selected a speech room".into())
            })?;
        if room != configured {
            return Err(HostError::NotPermitted(
                "the requested room does not match the household-selected speech room".into(),
            ));
        }
        let room = RoomName::parse(room.as_str()).map_err(|e| HostError::Invalid(e.to_string()))?;

        let service = self.house.speech_stream.get().ok_or_else(|| {
            HostError::Unavailable("speech output service is not configured".into())
        })?;
        service.open(&self.plugin.id, room).map_err(map_error)
    }
    fn submit(&mut self, session: u32, text: String) -> Result<u64, HostError> {
        self.speech_enabled()?;
        let most = niles_tts::speech_session::MAX_UTTERANCE_CHARS;
        if text.trim().is_empty() || text.chars().count() > most {
            return Err(HostError::Invalid(format!(
                "speech text must contain 1..={most} characters"
            )));
        }
        self.house
            .speech_stream
            .get()
            .ok_or_else(|| {
                HostError::Unavailable("speech output service is not configured".into())
            })?
            .submit(&self.plugin.id, session, text)
            .map_err(map_error)
    }
    fn poll(&mut self, session: u32) -> Result<Vec<RequestStatus>, HostError> {
        let results = self
            .house
            .speech_stream
            .get()
            .ok_or_else(|| {
                HostError::Unavailable("speech output service is not configured".into())
            })?
            .poll(&self.plugin.id, session)
            .map_err(map_error)?;
        Ok(results
            .into_iter()
            .map(|(request, outcome)| RequestStatus {
                request,
                outcome: match outcome {
                    SpeechOutcome::Pending => Outcome::Pending,
                    SpeechOutcome::Delivered => Outcome::Delivered,
                    SpeechOutcome::Busy => Outcome::Busy,
                    SpeechOutcome::Quiet => Outcome::Quiet,
                    SpeechOutcome::Expired => Outcome::Expired,
                    SpeechOutcome::Failed => Outcome::Failed,
                    SpeechOutcome::Cancelled => Outcome::Cancelled,
                    SpeechOutcome::Replaced => Outcome::Replaced,
                    SpeechOutcome::RateLimited => Outcome::RateLimited,
                    SpeechOutcome::NoSatellite => Outcome::NoSatellite,
                },
            })
            .collect())
    }
    fn close(&mut self, session: u32) {
        if let Some(service) = self.house.speech_stream.get() {
            service.close(&self.plugin.id, session);
        }
    }
}
