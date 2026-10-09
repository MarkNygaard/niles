//! The words this house is spoken to in, for speech recognition.
//!
//! ElevenLabs Scribe takes "keyterms": words to expect, which it then
//! prefers when the audio is unclear. A satellite in a room with music
//! hears "stop the wishing"; told the house says "music", it is likelier
//! to hear that. The list is built from the house itself — its rooms,
//! lights, scenes, speakers and favourite stations — so a lamp renamed
//! in Zigbee2MQTT is a word Niles listens for by the next refresh.
//!
//! Kept to a hundred terms: past that, ElevenLabs bills every request
//! as twenty seconds long, which a three-second command is not.

use crate::music::Music;
use niles_config::ConfigStore;
use niles_core::DeviceRegistry;
use niles_scheduler::SceneStore;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Over a hundred and each request is billed as twenty seconds.
const MOST: usize = 100;

/// What the house says to Niles that no device name carries.
const SPOKEN: &[&str] = &[
    "music",
    "radio",
    "Spotify",
    "Sonos",
    "TV",
    "Netflix",
    "YouTube",
    "DR TV",
    "pause",
    "stop",
    "louder",
    "quieter",
    "lights",
    "everywhere",
    "timer",
];

/// The house's words, refreshed in the background and read for every
/// utterance.
#[derive(Default)]
pub(crate) struct Vocabulary {
    words: RwLock<Vec<String>>,
}

impl Vocabulary {
    /// `configured` first — what somebody wrote in `[stt] keyterms` is
    /// never crowded out — then the house's own words.
    pub(crate) fn with(&self, configured: &[String]) -> Vec<String> {
        let house = self.words.read().unwrap_or_else(|e| e.into_inner());
        let mut words = Vec::new();
        for word in configured.iter().chain(house.iter()) {
            push_term(&mut words, word);
        }
        words
    }

    fn set(&self, words: Vec<String>) {
        *self.words.write().unwrap_or_else(|e| e.into_inner()) = words;
    }
}

/// Add `word` when Scribe would take it and it is not already there.
fn push_term(words: &mut Vec<String>, word: &str) {
    let word = word.replace('_', " ");
    let word = word.trim();
    let usable = !word.is_empty()
        && word.chars().count() < 50
        && word.split_whitespace().count() <= 5
        && !word.contains(['<', '>', '{', '}', '[', ']', '\\']);
    if usable && words.len() < MOST && !words.iter().any(|w| w.eq_ignore_ascii_case(word)) {
        words.push(word.to_string());
    }
}

/// The house's words, most useful first.
fn house_words(
    rooms: &[String],
    devices: &[String],
    scenes: &[String],
    speakers: &[String],
    favorites: &[String],
) -> Vec<String> {
    let mut words = Vec::new();
    for word in SPOKEN
        .iter()
        .map(|w| w.to_string())
        .chain(rooms.iter().cloned())
        .chain(scenes.iter().cloned())
        .chain(favorites.iter().cloned())
        .chain(speakers.iter().cloned())
        .chain(devices.iter().cloned())
    {
        push_term(&mut words, &word);
    }
    words
}

/// Gather the house's words now, and again every five minutes.
pub(crate) fn spawn_refresh(
    vocabulary: Arc<Vocabulary>,
    registry: Arc<DeviceRegistry>,
    config: Arc<ConfigStore>,
    scenes: Arc<SceneStore>,
    music: Arc<Music>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // The devices arrive from MQTT in the first seconds; a list made
        // before them would be a list of rooms with nothing in them.
        tokio::time::sleep(Duration::from_secs(20)).await;
        let mut every = tokio::time::interval(Duration::from_secs(5 * 60));
        loop {
            every.tick().await;
            let devices = registry.list_all();
            let mut rooms: Vec<String> = devices
                .iter()
                .map(|d| d.id.room().as_str().to_string())
                .collect();
            rooms.sort();
            rooms.dedup();
            let names: Vec<String> = devices
                .iter()
                .map(|d| d.id.name().as_str().to_string())
                .collect();
            let cfg = config.current();
            let speakers: Vec<String> = cfg
                .speakers
                .sonos
                .values()
                .map(|s| s.name.clone())
                .collect();
            let favorites = music.favorite_titles().await;
            let words = house_words(&rooms, &names, &scenes.names(), &speakers, &favorites);
            tracing::debug!("[vocabulary] {} words for speech recognition", words.len());
            vocabulary.set(words);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn the_house_says_its_rooms_and_lights() {
        let words = house_words(
            &strings(&["living_room", "walk_in_closet"]),
            &strings(&["floor_lamp", "tv_light"]),
            &strings(&["cosy"]),
            &strings(&["Living Room Back"]),
            &strings(&["DR P4 Østjylland 95.9 (Local Music)"]),
        );
        for expected in [
            "music",
            "living room",
            "walk in closet",
            "floor lamp",
            "cosy",
            "Living Room Back",
        ] {
            assert!(
                words.iter().any(|w| w == expected),
                "{expected} in {words:?}"
            );
        }
    }

    #[test]
    fn what_scribe_refuses_is_left_out() {
        let words = house_words(
            &[],
            &strings(&["a lamp with far too many words in its name", "[weird]"]),
            &[],
            &[],
            &strings(&["x".repeat(60).as_str()]),
        );
        assert!(!words.iter().any(|w| w.contains("far too many")));
        assert!(!words.iter().any(|w| w.contains('[')));
        assert!(!words.iter().any(|w| w.len() >= 50));
    }

    #[test]
    fn never_more_than_a_hundred_and_never_twice() {
        let many: Vec<String> = (0..300).map(|n| format!("lamp {n}")).collect();
        let words = house_words(&strings(&["kitchen", "Kitchen"]), &many, &[], &[], &[]);
        assert_eq!(words.len(), MOST);
        assert_eq!(
            words
                .iter()
                .filter(|w| w.eq_ignore_ascii_case("kitchen"))
                .count(),
            1
        );
    }

    #[test]
    fn what_was_configured_comes_first() {
        let vocabulary = Vocabulary::default();
        vocabulary.set((0..150).map(|n| format!("word {n}")).collect());
        let words = vocabulary.with(&strings(&["Niles"]));
        assert_eq!(words[0], "Niles");
        assert_eq!(words.len(), MOST);
    }
}
