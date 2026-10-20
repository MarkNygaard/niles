//! Bounded, owner-scoped asynchronous notification request tracking.
use niles_notifications::{DeliveryOutcome, NotificationCenter, Priority};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{runtime::Handle, time::Instant};

pub const MAX_REQUESTS_PER_INSTANCE: usize = 32;
pub const TERMINAL_RETENTION: Duration = Duration::from_secs(5 * 60);

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

/// A generation is unique to one wasmtime instance, not just a plugin id.
pub fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestStatus {
    Pending,
    Completed,
    Partial,
    Suppressed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerError {
    Capacity,
    Unknown,
    Closed,
}

#[derive(Clone)]
pub struct NotificationRequests {
    inner: Arc<Inner>,
}
struct Inner {
    center: Arc<NotificationCenter>,
    handle: Handle,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    records: HashMap<RequestId, Record>,
    reservations: HashMap<RequestId, Owner>,
    closed: HashMap<Owner, Instant>,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Owner {
    plugin: String,
    generation: u64,
}
struct Record {
    owner: Owner,
    status: RequestStatus,
    terminal_at: Option<Instant>,
}

/// A capacity reservation does not create a visible request or start work.
/// Dropping it releases capacity, allowing callers to validate and apply quota
/// between reservation and acceptance.
pub struct RequestReservation {
    id: RequestId,
    owner: Owner,
    inner: Weak<Inner>,
    accepted: bool,
}
impl Drop for RequestReservation {
    fn drop(&mut self) {
        if !self.accepted
            && let Some(inner) = self.inner.upgrade()
        {
            inner
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .reservations
                .remove(&self.id);
        }
    }
}

impl NotificationRequests {
    pub fn new(center: Arc<NotificationCenter>, handle: Handle) -> Self {
        Self {
            inner: Arc::new(Inner {
                center,
                handle,
                state: Mutex::new(State::default()),
            }),
        }
    }

    pub fn reserve(
        &self,
        plugin: &str,
        generation: u64,
    ) -> Result<RequestReservation, TrackerError> {
        let owner = Owner {
            plugin: plugin.to_owned(),
            generation,
        };
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        prune_expired(&mut state);
        if state.closed.contains_key(&owner) {
            return Err(TrackerError::Closed);
        }
        let used = state.records.values().filter(|r| r.owner == owner).count()
            + state.reservations.values().filter(|o| **o == owner).count();
        if used >= MAX_REQUESTS_PER_INSTANCE {
            return Err(TrackerError::Capacity);
        }
        let id = RequestId(
            NEXT_REQUEST
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| TrackerError::Capacity)?,
        );
        state.reservations.insert(id, owner.clone());
        Ok(RequestReservation {
            id,
            owner,
            inner: Arc::downgrade(&self.inner),
            accepted: false,
        })
    }

    pub fn accept(
        &self,
        mut reservation: RequestReservation,
        text: String,
        room: Option<String>,
        priority: Priority,
    ) -> Result<RequestId, TrackerError> {
        let id = reservation.id;
        let owner = reservation.owner.clone();
        let weak = Arc::downgrade(&self.inner);
        let center = Arc::clone(&self.inner.center);
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        prune_expired(&mut state);
        if state.closed.contains_key(&owner) {
            return Err(TrackerError::Closed);
        }
        if state.reservations.get(&id) != Some(&owner) {
            return Err(TrackerError::Unknown);
        }
        state.reservations.remove(&id);
        state.records.insert(
            id,
            Record {
                owner: owner.clone(),
                status: RequestStatus::Pending,
                terminal_at: None,
            },
        );
        // Not bounded here: delivery bounds its own steps (synthesis, the
        // write, a push given the audio's length), and a long announcement
        // is spoken to its end.
        self.inner.handle.spawn(async move {
            let notification = center.deliver(text, room, priority).await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let mut state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed.contains_key(&owner) {
                return;
            }
            let Some(record) = state.records.get_mut(&id) else {
                return;
            };
            if record.owner != owner {
                return;
            }
            record.status = map_outcome(notification.outcome);
            record.terminal_at = Some(Instant::now());
        });
        reservation.accepted = true;
        drop(state);
        Ok(id)
    }

    pub fn status(
        &self,
        plugin: &str,
        generation: u64,
        id: RequestId,
    ) -> Result<RequestStatus, TrackerError> {
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        prune_expired(&mut state);
        let owner = Owner {
            plugin: plugin.to_owned(),
            generation,
        };
        state
            .records
            .get(&id)
            .filter(|record| record.owner == owner)
            .map(|record| record.status)
            .ok_or(TrackerError::Unknown)
    }

    /// The instance is gone: nobody will ask about its requests any more,
    /// so they are forgotten. What is being spoken is not cut off — the
    /// household hears it, and it is kept in the notification history.
    pub fn close_owner(&self, plugin: &str, generation: u64) {
        let owner = Owner {
            plugin: plugin.to_owned(),
            generation,
        };
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed.insert(owner.clone(), Instant::now());
        state.reservations.retain(|_, existing| existing != &owner);
        state.records.retain(|_, record| record.owner != owner);
    }
}

fn prune_expired(state: &mut State) {
    let now = Instant::now();
    state.records.retain(|_, record| {
        record
            .terminal_at
            .is_none_or(|at| now.duration_since(at) < TERMINAL_RETENTION)
    });
    state
        .closed
        .retain(|_, at| now.duration_since(*at) < TERMINAL_RETENTION);
}

fn map_outcome(outcome: DeliveryOutcome) -> RequestStatus {
    match outcome {
        DeliveryOutcome::Delivered => RequestStatus::Completed,
        DeliveryOutcome::Partial => RequestStatus::Partial,
        DeliveryOutcome::Suppressed => RequestStatus::Suppressed,
        DeliveryOutcome::Failed => RequestStatus::Failed,
        _ => RequestStatus::Failed,
    }
}

#[cfg(test)]
#[path = "notification_requests_tests.rs"]
mod tests;
