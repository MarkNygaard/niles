//! The notification centre and its delivery to the satellites.

use crate::satellites::SatelliteRegistry;
use crate::speech_stream::{NotificationLease, RoomArbiter};
use crate::wiring::stores::database;
use crate::{speakers, tv};
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
    pub(crate) speech_arbiter: Arc<RoomArbiter>,
}

impl niles_notifications::NotificationDelivery for WyomingDelivery {
    fn deliver(
        &self,
        text: &str,
        room: Option<&str>,
        _priority: niles_notifications::Priority,
    ) -> bool {
        // On the screen as well, for whoever is watching rather than
        // listening. Extra to the spoken one, so a TV that is off or
        // does not answer changes nothing.
        if self.tv.shows_announcements() {
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
            return false;
        }
        let mut said = false;
        for room in &rooms {
            said |= self.speak_in(room, text);
        }
        said
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
        Some(room) => RoomName::parse(room).into_iter().collect(),
        None => satellites.rooms(),
    }
}

impl WyomingDelivery {
    /// Say `text` through the satellite in `room`: on its connection if
    /// it has one open, dialled otherwise. False when there is neither.
    fn speak_in(&self, room_name: &RoomName, text: &str) -> bool {
        let peer = {
            let index = self.peer_index.lock().unwrap_or_else(|e| e.into_inner());
            index.get(room_name).copied()
        };
        // No live connection is the *normal* case for anything
        // unprompted: the satellite hangs up after each turn, so a
        // timer firing thirty seconds later finds nothing here. Dial it
        // instead.
        let push_to = peer
            .is_none()
            .then(|| self.satellites.ip_for(room_name))
            .flatten();
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
        let notification_lease: NotificationLease =
            self.speech_arbiter.reserve_notification(room_name);
        tokio::spawn(async move {
            let _notification_lease = notification_lease;
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
                }
                return;
            }
            let ip = push_to.expect("checked above");
            match piper.synthesize(&text, None).await {
                Ok(synthesis) => {
                    let (pcm, format) = match crate::speak::wav_to_pcm(&synthesis.audio_wav) {
                        Ok((pcm, format)) => crate::speak::for_satellite(pcm, format),
                        Err(e) => {
                            tracing::warn!("could not decode synthesized notification: {e:#}");
                            return;
                        }
                    };
                    if let Err(e) = crate::push::speak_to(ip, &pcm, format).await {
                        tracing::warn!(
                            "could not reach the satellite at {ip} to say {text:?}: {e:#}"
                        );
                    } else {
                        tracing::info!("pushed {text:?} to the satellite at {ip}");
                    }
                }
                Err(e) => tracing::warn!("synthesizing a notification failed: {e:#}"),
            }
        });
        true
    }
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

    #[test]
    fn a_stored_notification_reads_back_and_a_broken_one_is_skipped() {
        let center = NotificationCenter::new(5);
        let said = center.deliver(
            "The groceries are here",
            None,
            niles_notifications::Priority::Routine,
        );
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
}
