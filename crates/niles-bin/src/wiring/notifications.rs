//! The notification centre and its delivery to the satellites.

use crate::satellites::SatelliteRegistry;
use crate::wiring::stores::database;
use crate::{speakers, tv};
use async_trait::async_trait;
use niles_config::Config;
use niles_core::RoomName;
use niles_notifications::{Notification, NotificationCenter, NotificationHistory};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

/// Build a `NotificationCenter` from the `[notifications]` section, with
/// what it said before the last restart when there is a database.
pub(crate) async fn build_notification_center(cfg: &Config) -> NotificationCenter {
    let mut center = NotificationCenter::new(cfg.notifications.capacity);
    if let Some(quiet) = cfg.notifications.to_quiet_hours_config(&cfg.home.timezone) {
        center = center.with_quiet_hours(quiet);
    }
    let Some(db) = database(cfg) else {
        tracing::info!("[notifications] no database; what Niles says is forgotten at a restart");
        return center;
    };
    let table = Arc::new(niles_db::PostgresNotifications::new(
        db.pool(),
        db.describe_target(),
    ));
    let earlier = match table.recent(cfg.notifications.capacity as i64).await {
        Ok(documents) => readable(&documents),
        Err(e) => {
            tracing::warn!("[notifications] what was said before could not be read: {e}");
            Vec::new()
        }
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Notification>();
    tokio::spawn(async move {
        while let Some(n) = rx.recv().await {
            let kept = match serde_json::to_string(&n) {
                Ok(document) => table.keep(&n.id, n.created_at, &document).await,
                Err(e) => Err(e.to_string()),
            };
            if let Err(e) = kept {
                tracing::warn!("[notifications] {:?} is not kept: {e}", n.text);
            }
        }
    });
    center.with_history(earlier, Arc::new(QueuedNotifications { tx }))
}

/// The stored documents that still read as notifications. One that does
/// not is skipped, rather than costing the rest.
fn readable(documents: &[String]) -> Vec<Notification> {
    documents
        .iter()
        .filter_map(|d| {
            serde_json::from_str(d)
                .inspect_err(|e| tracing::warn!("[notifications] a stored one would not read: {e}"))
                .ok()
        })
        .collect()
}

/// Hands each notification to the task that writes it: delivering one
/// must not wait on the database.
struct QueuedNotifications {
    tx: tokio::sync::mpsc::UnboundedSender<Notification>,
}

impl NotificationHistory for QueuedNotifications {
    fn keep(&self, notification: &Notification) {
        if self.tx.send(notification.clone()).is_err() {
            tracing::warn!(
                "[notifications] nothing is writing them any more; this one is not kept"
            );
        }
    }
}

/// Wyoming-backed delivery for the notification center.
pub(crate) struct WyomingDelivery {
    pub(crate) piper: Arc<niles_tts::PiperClient>,
    pub(crate) sender: niles_wyoming::WyomingSender,
    pub(crate) speakers: Arc<speakers::SpeakerRegistry>,
    pub(crate) satellites: Arc<SatelliteRegistry>,
    pub(crate) peer_index: Arc<Mutex<HashMap<RoomName, SocketAddr>>>,
    pub(crate) tv: Arc<tv::TvControl>,
}

#[async_trait]
impl niles_notifications::NotificationDelivery for WyomingDelivery {
    async fn deliver(
        &self,
        text: &str,
        room: Option<&str>,
        _priority: niles_notifications::Priority,
    ) -> niles_notifications::TransportOutcome {
        // On the screen as well, for whoever is watching rather than
        // listening. Extra to the spoken one, so a TV that is off or
        // does not answer changes nothing.
        // TV announcements are household-wide; there is no room-scoped TV
        // target, so showing one would widen an explicit room audience.
        if room.is_none() && self.tv.shows_announcements() {
            let tv = self.tv.clone();
            let message = text.to_string();
            tokio::spawn(async move {
                if let Err(e) = tv.show(&message).await {
                    tracing::debug!("[tv] announcement not shown: {e}");
                }
            });
        }
        let rooms = announcement_rooms(&self.satellites, room);
        if rooms.is_empty() {
            tracing::warn!("nothing to say {text:?} through: no satellite is set up for {room:?}");
            return niles_notifications::TransportOutcome::Failed;
        }
        aggregate_rooms(&rooms, |room| self.speak_in(room, text)).await
    }
}

/// Where an announcement is spoken: the room it names, and for one that
/// names none — an announcement for the house, which is what a grocery
/// delivery, a rule's "wherever Niles would" and a timer from a
/// satellite Niles does not know are — every room with a satellite.
pub(crate) fn announcement_rooms(
    satellites: &SatelliteRegistry,
    room: Option<&str>,
) -> Vec<RoomName> {
    match room {
        Some(room) => RoomName::parse(room)
            .ok()
            .filter(|room| satellites.ip_for(room).is_some())
            .into_iter()
            .collect(),
        None => satellites.rooms(),
    }
}

impl WyomingDelivery {
    /// Say `text` through the satellite in `room`: on its connection if
    /// it has one open, dialled otherwise. False when there is neither.
    async fn speak_in(&self, room_name: RoomName, text: &str) -> bool {
        let peer = {
            let index = self.peer_index.lock().unwrap_or_else(|e| e.into_inner());
            index.get(&room_name).copied()
        };
        let (peer, push_to) = room_route(&self.satellites, peer, &room_name);
        // No live connection is the *normal* case for anything
        // unprompted: the satellite hangs up after each turn, so a
        // timer firing thirty seconds later finds nothing here. Dial it
        // instead.
        if peer.is_none() && push_to.is_none() {
            tracing::warn!(
                "nothing to say {text:?} to: no satellite is connected from {room_name} and none is configured there"
            );
            return false;
        }

        let piper = self.piper.clone();
        let sender = self.sender.clone();
        let speakers = self.speakers.clone();
        let satellites = self.satellites.clone();
        let text = text.to_string();
        bounded_operation(async move {
            if let Some(peer) = peer {
                if let Err(e) = crate::speak::speak_back(
                    &piper,
                    &sender,
                    peer,
                    &text,
                    &speakers,
                    &satellites,
                    false,
                )
                .await
                {
                    tracing::warn!("[{peer}] notification speak-back failed: {e:#}");
                    return false;
                }
                return true;
            }
            let ip = push_to.expect("checked above");
            match piper.synthesize(&text, None).await {
                Ok(synthesis) => {
                    let (pcm, format) = match crate::speak::wav_to_pcm(&synthesis.audio_wav) {
                        Ok((pcm, format)) => crate::speak::for_satellite(pcm, format),
                        Err(e) => {
                            tracing::warn!("could not decode synthesized notification: {e:#}");
                            return false;
                        }
                    };
                    if let Err(e) = crate::push::speak_to(ip, &pcm, format).await {
                        tracing::warn!(
                            "could not reach the satellite at {ip} to say {text:?}: {e:#}"
                        );
                    } else {
                        tracing::info!("pushed {text:?} to the satellite at {ip}");
                        return true;
                    }
                }
                Err(e) => tracing::warn!("synthesizing a notification failed: {e:#}"),
            }
            false
        })
        .await
        .unwrap_or_else(|| {
            tracing::warn!("notification delivery to {room_name} timed out");
            false
        })
    }
}

const DELIVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

async fn bounded_operation<F: std::future::Future>(future: F) -> Option<F::Output> {
    tokio::time::timeout(DELIVERY_TIMEOUT, future).await.ok()
}

async fn aggregate_rooms<T, F, Fut>(
    rooms: &[T],
    operation: F,
) -> niles_notifications::TransportOutcome
where
    T: Clone,
    F: Fn(T) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let results = futures_util::future::join_all(rooms.iter().cloned().map(operation)).await;
    let succeeded = results.iter().filter(|success| **success).count();
    if succeeded == 0 || results.is_empty() {
        niles_notifications::TransportOutcome::Failed
    } else if succeeded == results.len() {
        niles_notifications::TransportOutcome::Completed
    } else {
        niles_notifications::TransportOutcome::Partial
    }
}

fn room_route(
    satellites: &SatelliteRegistry,
    peer: Option<SocketAddr>,
    room: &RoomName,
) -> (Option<SocketAddr>, Option<std::net::IpAddr>) {
    let peer = peer.filter(|peer| {
        satellites
            .room_for(*peer)
            .is_some_and(|assigned| assigned == room)
    });
    let dial = peer.is_none().then(|| satellites.ip_for(room)).flatten();
    (peer, dial)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn satellites(rooms: &[(&str, &str)]) -> SatelliteRegistry {
        let mut cfg = niles_config::SatellitesConfig::default();
        for (ip, room) in rooms {
            cfg.satellites.insert(
                format!("{room}-{ip}"),
                niles_config::SatelliteConfig {
                    ip: (*ip).into(),
                    room: (*room).into(),
                    volume: 100,
                },
            );
        }
        SatelliteRegistry::from_config(&cfg)
    }

    fn names(rooms: Vec<RoomName>) -> Vec<String> {
        rooms.iter().map(|r| r.as_str().to_string()).collect()
    }

    #[test]
    fn an_announcement_for_the_house_is_said_in_every_room_with_a_satellite() {
        let sats = satellites(&[
            ("192.168.1.10", "kitchen"),
            ("192.168.1.11", "office"),
            ("192.168.1.12", "kitchen"),
        ]);
        assert_eq!(
            names(announcement_rooms(&sats, None)),
            ["kitchen", "office"]
        );
    }

    #[test]
    fn an_announcement_for_a_room_is_said_only_there() {
        let sats = satellites(&[("192.168.1.10", "kitchen"), ("192.168.1.11", "office")]);
        assert_eq!(names(announcement_rooms(&sats, Some("office"))), ["office"]);
    }

    #[tokio::test]
    async fn a_stored_notification_reads_back_and_a_broken_one_is_skipped() {
        let center = NotificationCenter::new(5);
        let said = center
            .deliver(
                "The groceries are here",
                None,
                niles_notifications::Priority::Routine,
            )
            .await;
        let documents = vec![
            serde_json::to_string(&said).unwrap(),
            "not a notification".to_string(),
        ];
        let read = readable(&documents);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].text, "The groceries are here");
        assert_eq!(read[0].id, said.id);
    }

    #[test]
    fn with_no_satellites_an_announcement_for_the_house_has_nowhere_to_go() {
        assert!(announcement_rooms(&satellites(&[]), None).is_empty());
    }

    #[test]
    fn explicit_unknown_or_unrouted_rooms_have_no_announcement_route() {
        let sats = satellites(&[("192.168.1.10", "kitchen")]);
        assert!(announcement_rooms(&sats, Some("study")).is_empty());
        assert!(announcement_rooms(&sats, Some("office")).is_empty());
    }

    #[test]
    fn stale_peer_only_survives_when_registry_assigns_it_to_the_room() {
        let sats = satellites(&[("192.168.1.10", "kitchen"), ("192.168.1.11", "office")]);
        let kitchen = RoomName::parse("kitchen").unwrap();
        let reassigned: SocketAddr = "192.168.1.11:5000".parse().unwrap();
        let removed: SocketAddr = "192.168.1.12:5000".parse().unwrap();
        let current: SocketAddr = "192.168.1.10:5000".parse().unwrap();
        assert_eq!(room_route(&sats, Some(reassigned), &kitchen).0, None);
        assert_eq!(room_route(&sats, Some(removed), &kitchen).0, None);
        assert_eq!(
            room_route(&sats, Some(current), &kitchen),
            (Some(current), None)
        );
    }

    #[test]
    fn missing_peer_dials_only_the_configured_satellite_in_that_room() {
        let sats = satellites(&[("192.168.1.10", "kitchen"), ("192.168.1.11", "office")]);
        let kitchen = RoomName::parse("kitchen").unwrap();
        assert_eq!(
            room_route(&sats, None, &kitchen),
            (None, Some("192.168.1.10".parse().unwrap()))
        );
    }

    #[tokio::test]
    async fn production_aggregation_maps_all_partial_failed_and_empty_routes() {
        use niles_notifications::TransportOutcome::{Completed, Failed, Partial};
        let rooms = [1, 2];
        assert_eq!(aggregate_rooms(&rooms, |_| async { true }).await, Completed);
        assert_eq!(
            aggregate_rooms(&rooms, |room| async move { room == 1 }).await,
            Partial
        );
        assert_eq!(aggregate_rooms(&rooms, |_| async { false }).await, Failed);
        assert_eq!(
            aggregate_rooms::<i32, _, _>(&[], |_| async { true }).await,
            Failed
        );
    }

    #[tokio::test(start_paused = true)]
    async fn bounded_operation_times_out_pending_transport_and_returns_completed_result() {
        let pending = tokio::spawn(bounded_operation(std::future::pending::<bool>()));
        tokio::time::advance(DELIVERY_TIMEOUT - std::time::Duration::from_millis(1)).await;
        assert!(!pending.is_finished());
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        assert_eq!(pending.await.unwrap(), None);
        assert_eq!(bounded_operation(async { true }).await, Some(true));
        assert_eq!(bounded_operation(async { false }).await, Some(false));
    }
}
