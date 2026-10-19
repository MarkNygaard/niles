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
    Disabled,
    Unavailable,
    Overload,
    Expired,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpeechError {
    Disabled,
    Busy,
    QuietHours,

    Unavailable,
    Limit,
    Invalid,
    Closed,
    NotOwned,
}

/// Production adapter; explicitly constructed/injected with the shared
/// `niles-tts` manager. No default service or fallback is created by plugins.
pub struct SessionSpeechHost<
    O: niles_tts::speech_session::SpeechOutput,
    P: niles_tts::speech_session::SpeechEligibility,
> {
    manager: Arc<SpeechSessionManager<O, P>>,
    handle: Handle,
    sessions: Mutex<BTreeMap<(String, u32), Arc<SessionSlot>>>,
    next: Mutex<u32>,
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
impl<O, P> SessionSpeechHost<O, P>
where
    O: niles_tts::speech_session::SpeechOutput,
    P: niles_tts::speech_session::SpeechEligibility,
{
    pub fn new(manager: Arc<SpeechSessionManager<O, P>>, handle: Handle) -> Self {
        Self {
            manager,
            handle,
            sessions: Mutex::new(BTreeMap::new()),
            next: Mutex::new(1),
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
fn map_rejection(reason: Rejection) -> SpeechOutcome {
    match reason {
        Rejection::Busy => SpeechOutcome::Busy,
        Rejection::QuietHours => SpeechOutcome::Quiet,
        Rejection::Disabled => SpeechOutcome::Disabled,
        Rejection::Unavailable => SpeechOutcome::Unavailable,
        Rejection::QueueFull => SpeechOutcome::Overload,
        Rejection::Closed => SpeechOutcome::Cancelled,
        _ => SpeechOutcome::Failed,
    }
}
fn map_terminal(result: TerminalResult) -> SpeechOutcome {
    match result {
        TerminalResult::DeliveredToOutput => SpeechOutcome::Delivered,
        TerminalResult::Rejected(reason) => map_rejection(reason),
        TerminalResult::Cancelled => SpeechOutcome::Cancelled,
        TerminalResult::Expired => SpeechOutcome::Expired,
        _ => SpeechOutcome::Failed,
    }
}
fn map_session_outcome(error: SessionError) -> SpeechOutcome {
    match error {
        SessionError::Ineligible(reason) => map_rejection(reason),
        SessionError::OwnerHasSession => SpeechOutcome::Busy,
        SessionError::Disabled => SpeechOutcome::Disabled,
        _ => SpeechOutcome::Failed,
    }
}
fn map_error(error: SpeechError) -> HostError {
    match error {
        SpeechError::Disabled => HostError::Unavailable("speech stream is disabled".into()),
        SpeechError::Busy => HostError::Unavailable("speech output is busy".into()),
        SpeechError::QuietHours => {
            HostError::Unavailable("speech is suppressed during quiet hours".into())
        }
        SpeechError::Unavailable => HostError::Unavailable("speech output is unavailable".into()),
        SpeechError::Limit => HostError::Limit("speech stream is at capacity".into()),
        SpeechError::Invalid => HostError::Invalid("invalid room or speech text".into()),
        SpeechError::Closed => HostError::Unavailable("speech stream session is closed".into()),
        SpeechError::NotOwned => {
            HostError::NotPermitted("session does not belong to this plugin".into())
        }
    }
}
impl<O, P> SessionSpeechHost<O, P>
where
    O: niles_tts::speech_session::SpeechOutput,
    P: niles_tts::speech_session::SpeechEligibility,
{
    fn map_session(error: SessionError) -> SpeechError {
        match error {
            SessionError::Disabled => SpeechError::Disabled,
            SessionError::OwnerHasSession => SpeechError::Busy,
            SessionError::Closed => SpeechError::Closed,
            SessionError::InvalidOwner => SpeechError::Invalid,
            SessionError::Ineligible(reason) => Self::map_rejection(reason),
        }
    }
    fn map_rejection(reason: Rejection) -> SpeechError {
        match reason {
            Rejection::Busy => SpeechError::Busy,
            Rejection::QuietHours => SpeechError::QuietHours,
            Rejection::Disabled => SpeechError::Disabled,
            Rejection::QueueFull => SpeechError::Limit,
            Rejection::Closed => SpeechError::Closed,
            Rejection::InvalidText => SpeechError::Invalid,
            _ => SpeechError::Unavailable,
        }
    }
    fn owner(&self, owner: &str, session: u32) -> Result<(), SpeechError> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(owner.to_owned(), session))
            .filter(|slot| slot.accepting.load(Ordering::Acquire))
            .map(|_| ())
            .ok_or(SpeechError::NotOwned)
    }
}
impl<O, P> SpeechStreamHost for SessionSpeechHost<O, P>
where
    O: niles_tts::speech_session::SpeechOutput,
    P: niles_tts::speech_session::SpeechEligibility,
{
    fn open(&self, owner: &str, room: RoomName) -> Result<u32, SpeechError> {
        let owner_id =
            PluginOwner::new(format!("{}:{owner}", self.owner_token)).map_err(Self::map_session)?;
        let raw = {
            let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
            let id = *next;
            *next = next.checked_add(1).ok_or(SpeechError::Limit)?;
            id
        };
        let key = (owner.to_owned(), raw);
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
            if sessions.keys().any(|(existing, _)| existing == owner) {
                return Err(SpeechError::Busy);
            }
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
            let work = async {
                while let Some(Command::Submit {
                    request,
                    text,
                    deadline,
                }) = rx.recv().await
                {
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
                    let outcome =
                        match tokio::time::timeout_at(deadline, manager.submit(id.unwrap(), &text))
                            .await
                        {
                            Ok(Ok((_, terminal))) => terminal
                                .await
                                .map(map_terminal)
                                .unwrap_or(SpeechOutcome::Cancelled),
                            Ok(Err(reason)) => map_rejection(reason),
                            Err(_) => SpeechOutcome::Expired,
                        };
                    push_status(&results, (request, outcome));
                }
            };
            tokio::select! {
                biased;
                _ = cancelled.changed() => {},
                () = work => {},
            }
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
        self.owner(owner, session)?;
        let slot = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(owner.to_owned(), session))
            .cloned()
            .ok_or(SpeechError::NotOwned)?;
        let mut queue = slot.results.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= MAX_RETAINED_RESULTS {
            return Err(SpeechError::Limit);
        }
        if !slot.accepting.load(Ordering::Acquire) {
            return Err(SpeechError::Closed);
        }
        let raw = {
            let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
            let raw = u64::from(*next);
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
            ready.push(queue.pop_front().unwrap());
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
        if let Some(service) = self.house.speech_stream.get() {
            service.close_owner(&self.plugin.id);
        }
    }
}

impl HostState {
    fn speech_enabled(&self) -> Result<(), HostError> {
        if !self.plugin.manifest.permissions.speech_stream {
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
        let room = RoomName::parse(room.as_str()).map_err(|e| HostError::Invalid(e.to_string()))?;

        let service = self.house.speech_stream.get().ok_or_else(|| {
            HostError::Unavailable("speech output service is not configured".into())
        })?;
        service.open(&self.plugin.id, room).map_err(map_error)
    }
    fn submit(&mut self, session: u32, text: String) -> Result<u64, HostError> {
        self.speech_enabled()?;
        if text.trim().is_empty()
            || text.chars().count() > niles_tts::speech_session::MAX_UTTERANCE_CHARS
        {
            return Err(HostError::Invalid(
                "speech text must contain 1..=256 characters".into(),
            ));
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
                    SpeechOutcome::Disabled => Outcome::Disabled,
                    SpeechOutcome::Unavailable => Outcome::Unavailable,
                    SpeechOutcome::Overload => Outcome::Overload,
                    SpeechOutcome::Expired => Outcome::Expired,
                    SpeechOutcome::Failed => Outcome::Failed,
                    SpeechOutcome::Cancelled => Outcome::Cancelled,
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
