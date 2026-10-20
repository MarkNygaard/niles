use crate::{
    bindings::{niles::integration::types::HostError, notify_0_6},
    host::{HostState, NOTIFICATIONS_PER_HOUR},
    manifest::Notify,
    notification_requests::{RequestId, RequestStatus, TrackerError},
};
use niles_core::RoomName;
use niles_notifications::{NotificationCenter, Priority};
use std::sync::Arc;

pub(crate) struct ValidatedNotification {
    /// `None` where Niles has not connected notifications, as under
    /// `niles plugin dev`: a `say` is then logged rather than spoken.
    pub center: Option<Arc<NotificationCenter>>,
    pub message: String,
    pub room: Option<String>,
    pub priority: Priority,
}

pub(crate) fn validate(
    state: &HostState,
    message: String,
    priority: notify_0_6::Priority,
    room: Option<String>,
) -> Result<ValidatedNotification, HostError> {
    let granted = state.plugin.manifest.permissions.notify;
    match (granted, priority) {
        (Notify::No, _) => {
            return Err(HostError::NotPermitted(
                "the manifest does not ask for notify".into(),
            ));
        }
        (Notify::Yes, notify_0_6::Priority::Urgent) => {
            return Err(HostError::NotPermitted(
                "urgent needs notify = \"urgent\" in the manifest".into(),
            ));
        }
        _ => {}
    }
    let message = message.trim().to_owned();
    if message.is_empty() || message.chars().count() > super::host::MAX_MESSAGE {
        return Err(HostError::Invalid(format!(
            "a message is 1 to {} characters",
            super::host::MAX_MESSAGE
        )));
    }
    if let Some(room) = &room {
        if !crate::manifest::is_identifier(room) {
            return Err(HostError::Invalid(format!(
                "{room:?} is not a room id like living_room"
            )));
        }
        let parsed = RoomName::parse(room)
            .map_err(|_| HostError::Invalid(format!("{room:?} is not a valid room")))?;
        // Checked once Niles has said which rooms have a satellite, which
        // `niles plugin dev` never does.
        if let Some(routes) = state.house.notification_rooms.get() {
            let known_from_devices = state
                .house
                .registry
                .list_all()
                .iter()
                .any(|device| device.id.room() == &parsed);
            let configured_route = routes.contains(&parsed);
            if !known_from_devices && !configured_route {
                return Err(HostError::Invalid(format!("unknown room {room:?}")));
            }
            if !configured_route {
                return Err(HostError::Unavailable(format!(
                    "room {room:?} has no configured satellite"
                )));
            }
        }
    }
    let center = state.house.notifications.get().cloned();
    let priority = match priority {
        notify_0_6::Priority::Low => Priority::Routine,
        notify_0_6::Priority::Normal => Priority::Important,
        notify_0_6::Priority::Urgent => Priority::Urgent,
    };
    Ok(ValidatedNotification {
        center,
        message,
        room,
        priority,
    })
}

pub(crate) fn charge_quota(state: &HostState) -> Result<(), HostError> {
    if state.plugin.may_notify(NOTIFICATIONS_PER_HOUR) {
        Ok(())
    } else {
        Err(HostError::Limit(format!(
            "at most {NOTIFICATIONS_PER_HOUR} notifications an hour"
        )))
    }
}

pub(crate) fn map_status(status: RequestStatus) -> notify_0_6::RequestStatus {
    match status {
        RequestStatus::Pending => notify_0_6::RequestStatus::Pending,
        RequestStatus::Completed => notify_0_6::RequestStatus::Completed,
        RequestStatus::Partial => notify_0_6::RequestStatus::Partial,
        RequestStatus::Suppressed => notify_0_6::RequestStatus::Suppressed,
        RequestStatus::Failed => notify_0_6::RequestStatus::Failed,
    }
}

pub(crate) fn tracker_error(error: TrackerError) -> HostError {
    match error {
        TrackerError::Capacity => {
            HostError::Limit("at most 32 notification requests per plugin instance".into())
        }
        TrackerError::Closed | TrackerError::Unknown => {
            HostError::Invalid("unknown notification request".into())
        }
    }
}

pub(crate) fn status(state: &HostState, id: u64) -> Result<notify_0_6::RequestStatus, HostError> {
    let tracker = state
        .house
        .notification_requests
        .get()
        .ok_or_else(|| HostError::Unavailable("notifications are not connected".into()))?;
    tracker
        .status(
            &state.plugin.id,
            state.notification_generation,
            RequestId(id),
        )
        .map(map_status)
        .map_err(tracker_error)
}
