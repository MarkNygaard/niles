//! A timer that rings until somebody stops it.
//!
//! Saying "timer finished" once is a notification, and a notification
//! is easy to miss from the next room. A timer is an alarm: it keeps
//! going until you deal with it.
//!
//! The satellite cannot listen while it plays — playback holds the I2S
//! bus the microphone reads from — so the chime comes in bursts, and
//! the quiet between them is when "Niles, stop" can be heard. A
//! continuous tone would be an alarm nobody could stop by voice.

use crate::satellites::SatelliteRegistry;
use niles_core::{Event, EventBus, RoomName};
use niles_notifications::NotificationCenter;
use niles_scheduler::{TimerId, TimerStore};
use niles_wyoming::AudioFormat;
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One chime, then quiet until the next. Long enough that the gap holds
/// a wake word and the start of "stop"; short enough that it still
/// sounds like something ringing rather than something that rang.
pub const EVERY: Duration = Duration::from_secs(5);

/// A timer nobody is home to hear should not ring into an empty house
/// all afternoon.
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(10 * 60);

const RATE: u32 = 22_050;

/// The chime, as 16-bit mono PCM.
///
/// Synthesised rather than shipped as a file: two bell notes are a
/// dozen lines, and a sound file is one more thing the image has to
/// carry and the build has to find.
pub fn chime() -> (Vec<u8>, AudioFormat) {
    // C6 then E6, twice. Bright enough to cut through a kitchen, and a
    // rising third reads as "done" rather than "wrong".
    const NOTES: [(f32, f32); 4] = [
        (1046.5, 0.0),
        (1318.5, 0.22),
        (1046.5, 0.62),
        (1318.5, 0.84),
    ];
    const RING: f32 = 0.45;
    const LENGTH: f32 = 1.4;

    let total = (LENGTH * RATE as f32) as usize;
    let mut samples = vec![0f32; total];
    for (freq, start) in NOTES {
        let from = (start * RATE as f32) as usize;
        for (i, s) in samples[from..].iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            if t > RING {
                break;
            }
            // A 2 ms attack, or every note starts with a click.
            let attack = (t / 0.002).min(1.0);
            let decay = (-t * 9.0).exp();
            // The octave is what makes it a bell and not a beep.
            let tone = (TAU * freq * t).sin() + 0.3 * (TAU * 2.0 * freq * t).sin();
            *s += 0.35 * attack * decay * tone;
        }
    }

    let pcm = samples
        .into_iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
        .collect();
    (pcm, AudioFormat::new(RATE, 16, 1))
}

/// Why a ringing timer went quiet.
#[derive(Debug, PartialEq, Eq)]
pub enum Ended {
    Stopped,
    GaveUp,
}

/// Ring until the timer is no longer ringing, or until [`GIVE_UP_AFTER`].
///
/// `busy` is whether the satellite is in a conversation right now —
/// somebody saying "Niles, stop", most likely. A chime dialled in then
/// would queue behind the conversation and play after the reply, so the
/// timer would ring once more after being told to stop.
pub async fn ring<Fut>(
    still_ringing: impl Fn() -> bool,
    busy: impl Fn() -> bool,
    mut push: impl FnMut() -> Fut,
) -> Ended
where
    Fut: Future<Output = anyhow::Result<()>>,
{
    let started = tokio::time::Instant::now();
    let mut ticks = tokio::time::interval(EVERY);
    // A chime takes as long to send as it does to play, so the ticks
    // are measured from each other, not from a burst that ran long.
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut failing = false;
    loop {
        ticks.tick().await;
        if !still_ringing() {
            return Ended::Stopped;
        }
        if started.elapsed() >= GIVE_UP_AFTER {
            return Ended::GaveUp;
        }
        if busy() {
            continue;
        }
        match push().await {
            Ok(()) => failing = false,
            // Once, not every five seconds for ten minutes.
            Err(e) if !failing => {
                tracing::warn!("[timer] could not ring the satellite: {e:#}");
                failing = true;
            }
            Err(_) => {}
        }
    }
}

/// Ring every timer that fires, in the room it was set from.
///
/// A timer from a room with no satellite Niles can dial — or set from
/// somewhere that is not a room — is still announced the old way, as a
/// notification, because a timer that fires silently is the one thing
/// worse than one that fires once.
pub fn spawn_timer_alarms(
    bus: &EventBus,
    timers: Arc<TimerStore>,
    satellites: Arc<SatelliteRegistry>,
    peer_index: Arc<Mutex<HashMap<RoomName, SocketAddr>>>,
    center: Arc<NotificationCenter>,
) -> tokio::task::JoinHandle<()> {
    let mut bus_rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(Event::TimerFired { id, name, origin }) => {
                    let room = satellites.room_for(origin).cloned();
                    let ip = room.as_ref().and_then(|r| satellites.ip_for(r));
                    let (Some(room), Some(ip)) = (room.clone(), ip) else {
                        let text = match name {
                            Some(n) => format!("'{n}' timer finished"),
                            None => "Timer finished".to_string(),
                        };
                        center.deliver(
                            text,
                            room.map(|r| r.as_str().to_string()),
                            niles_notifications::Priority::Important,
                        );
                        continue;
                    };
                    let (pcm, format) = chime();
                    let pcm = crate::speak::at_volume(
                        &pcm,
                        format.bits_per_sample,
                        satellites.volume_for(SocketAddr::new(ip, 0)),
                    )
                    .into_owned();
                    let timers = timers.clone();
                    let peer_index = peer_index.clone();
                    tokio::spawn(async move {
                        let id = TimerId(id);
                        let ended = ring(
                            || timers.list().iter().any(|t| t.id == id && t.is_ringing()),
                            || {
                                peer_index
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .contains_key(&room)
                            },
                            || crate::push::speak_to(ip, &pcm, format),
                        )
                        .await;
                        if ended == Ended::GaveUp {
                            // Or "stop", said tomorrow, would answer for
                            // an alarm that went quiet hours ago.
                            timers.cancel(id);
                            tracing::info!("[timer] {room} rang for {GIVE_UP_AFTER:?} unanswered");
                        }
                    });
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("timer alarms lagged by {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn the_chime_is_audio_the_satellite_can_play() {
        let (pcm, format) = chime();
        assert_eq!(format.bits_per_sample, 16);
        assert_eq!(format.channels, 1);
        assert_eq!(pcm.len() % 2, 0, "whole samples only");
        let seconds = pcm.len() as f32 / 2.0 / format.sample_rate_hz as f32;
        assert!(
            seconds < EVERY.as_secs_f32() / 2.0,
            "most of each cycle stays quiet"
        );
    }

    #[test]
    fn the_chime_starts_and_ends_without_a_click() {
        let (pcm, _) = chime();
        let samples: Vec<i16> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| i16::from_le_bytes(*s))
            .collect();
        assert!(samples[0].unsigned_abs() < 100);
        assert!(samples.last().unwrap().unsigned_abs() < 100);
        let loudest = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(loudest > 8_000, "audible");
        assert!(loudest < i16::MAX as u16, "not clipped");
    }

    #[tokio::test(start_paused = true)]
    async fn it_keeps_ringing_until_stopped() {
        let rings = AtomicUsize::new(0);
        let ended = ring(
            || rings.load(Ordering::SeqCst) < 3,
            || false,
            || {
                rings.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            },
        )
        .await;
        assert_eq!(ended, Ended::Stopped);
        assert_eq!(rings.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn it_holds_its_chime_while_somebody_is_talking() {
        let rings = AtomicUsize::new(0);
        let checks = AtomicUsize::new(0);
        let talking = AtomicBool::new(true);
        ring(
            || {
                // Somebody talks through two cycles, then stops it.
                let n = checks.fetch_add(1, Ordering::SeqCst);
                if n == 2 {
                    talking.store(false, Ordering::SeqCst);
                }
                n < 2
            },
            || talking.load(Ordering::SeqCst),
            || {
                rings.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            },
        )
        .await;
        assert_eq!(rings.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn it_gives_up_on_an_empty_house() {
        let rings = AtomicUsize::new(0);
        let ended = ring(
            || true,
            || false,
            || {
                rings.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            },
        )
        .await;
        assert_eq!(ended, Ended::GaveUp);
        let expected = (GIVE_UP_AFTER.as_secs() / EVERY.as_secs()) as usize;
        assert_eq!(rings.load(Ordering::SeqCst), expected);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreachable_satellite_does_not_stop_the_timer() {
        // The next burst may get through: a satellite rebooting is not a
        // timer somebody stopped.
        let tries = AtomicUsize::new(0);
        let ended = ring(
            || tries.load(Ordering::SeqCst) < 4,
            || false,
            || {
                tries.fetch_add(1, Ordering::SeqCst);
                async { Err(anyhow::anyhow!("no route to host")) }
            },
        )
        .await;
        assert_eq!(ended, Ended::Stopped);
        assert_eq!(tries.load(Ordering::SeqCst), 4);
    }
}
