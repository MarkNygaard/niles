//! The notification centre and its delivery to the satellites.

use crate::satellites::SatelliteRegistry;
use crate::{speakers, tv};
use niles_core::RoomName;
use niles_notifications::NotificationCenter;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

/// Build a `NotificationCenter` from the `[notifications]` section.
pub(crate) fn build_notification_center(
    cfg: &niles_config::NotificationsConfig,
    timezone_str: &str,
) -> NotificationCenter {
    let mut center = NotificationCenter::new(cfg.capacity);
    if let Some(quiet) = cfg.to_quiet_hours_config(timezone_str) {
        center = center.with_quiet_hours(quiet);
    }
    center
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
        let Some(room_str) = room else {
            return false;
        };
        let Ok(room_name) = RoomName::parse(room_str) else {
            return false;
        };
        let peer = {
            let index = self.peer_index.lock().unwrap_or_else(|e| e.into_inner());
            index.get(&room_name).copied()
        };
        // No live connection is the *normal* case for anything
        // unprompted: the satellite hangs up after each turn, so a
        // timer firing thirty seconds later finds nothing here. Dial it
        // instead.
        let push_to = peer
            .is_none()
            .then(|| self.satellites.ip_for(&room_name))
            .flatten();
        if peer.is_none() && push_to.is_none() {
            tracing::warn!(
                "nothing to say {text:?} to: no satellite is connected from {room_name}                  and none is configured there"
            );
            return false;
        }

        let piper = self.piper.clone();
        let sender = self.sender.clone();
        let speakers = self.speakers.clone();
        let satellites = self.satellites.clone();
        let text = text.to_string();
        tokio::spawn(async move {
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
