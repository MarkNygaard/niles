//! The Sonos household, for the page that places each room.
//!
//! Asked of a speaker as the page opens, so what it lists is what is on
//! the network now — a speaker bought yesterday included — beside what
//! `[speakers]` has placed. A placed speaker that does not answer stays
//! listed, by the name it had, so a room is not silently lost to an
//! unplugged Move.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use niles_config::SpeakersConfig;
use niles_speakers::{HttpTransport, SonosRoom};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SpeakersDto {
    /// Whether an address has been given to ask.
    pub configured: bool,
    /// Why the household could not be read, when it could not.
    pub error: Option<String>,
    pub sonos: Vec<SonosDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SonosDto {
    pub id: String,
    pub name: String,
    pub ip: Option<String>,
    /// A soundbar with speakers around it: the one a TV plays through.
    pub home_theater: bool,
    /// The Niles room it is placed in, if any.
    pub room: Option<String>,
    pub answering: bool,
}

/// `GET /speakers`
pub async fn list(State(state): State<AppState>) -> Json<SpeakersDto> {
    let Some(cfg) = state.config.as_ref().map(|c| c.current()) else {
        return Json(rows(&SpeakersConfig::default(), Ok(Vec::new())));
    };
    let speakers = &cfg.speakers;
    let host = speakers.host.trim();
    if host.is_empty() {
        return Json(rows(speakers, Ok(Vec::new())));
    }
    let household = niles_speakers::household(&HttpTransport::new(), host)
        .await
        .map_err(|e| format!("No Sonos answered at {host}: {e}"));
    Json(rows(speakers, household))
}

fn rows(cfg: &SpeakersConfig, household: Result<Vec<SonosRoom>, String>) -> SpeakersDto {
    let configured = !cfg.host.trim().is_empty();
    let (found, error) = match household {
        Ok(found) => (found, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    let placed_in = |id: &str| cfg.sonos.get(id).map(|s| s.room.clone());
    let mut sonos: Vec<SonosDto> = found
        .iter()
        .map(|r| SonosDto {
            id: r.id.clone(),
            name: r.name.clone(),
            ip: Some(r.ip.clone()),
            home_theater: r.home_theater,
            room: placed_in(&r.id),
            answering: true,
        })
        .collect();
    let mut missing: Vec<SonosDto> = cfg
        .sonos
        .iter()
        .filter(|(id, _)| !found.iter().any(|r| &r.id == *id))
        .map(|(id, placed)| SonosDto {
            id: id.clone(),
            name: if placed.name.is_empty() {
                id.clone()
            } else {
                placed.name.clone()
            },
            ip: None,
            home_theater: false,
            room: Some(placed.room.clone()),
            answering: false,
        })
        .collect();
    missing.sort_by(|a, b| a.name.cmp(&b.name));
    sonos.append(&mut missing);
    SpeakersDto {
        configured,
        error,
        sonos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(toml: &str) -> SpeakersConfig {
        toml::from_str(toml).unwrap()
    }

    fn found(id: &str, name: &str) -> SonosRoom {
        SonosRoom {
            id: id.into(),
            name: name.into(),
            ip: "10.0.0.2".into(),
            home_theater: false,
            coordinator: id.into(),
        }
    }

    #[test]
    fn lists_what_answers_with_the_room_it_is_placed_in() {
        let dto = rows(
            &cfg("host = \"10.0.0.2\"\n[sonos.RINCON_A]\nroom = \"kitchen\""),
            Ok(vec![
                found("RINCON_A", "Sonos Move"),
                found("RINCON_B", "Bedroom"),
            ]),
        );
        assert!(dto.configured);
        assert_eq!(dto.sonos[0].room.as_deref(), Some("kitchen"));
        assert_eq!(dto.sonos[1].room, None);
        assert!(dto.sonos.iter().all(|s| s.answering));
    }

    #[test]
    fn a_placed_speaker_that_does_not_answer_stays_listed_by_name() {
        let dto = rows(
            &cfg(
                "host = \"10.0.0.2\"\n[sonos.RINCON_A]\nroom = \"kitchen\"\nname = \"Sonos Move\"",
            ),
            Err("No Sonos answered at 10.0.0.2".into()),
        );
        assert_eq!(dto.error.as_deref(), Some("No Sonos answered at 10.0.0.2"));
        assert_eq!(dto.sonos.len(), 1);
        assert_eq!(dto.sonos[0].name, "Sonos Move");
        assert!(!dto.sonos[0].answering);
    }

    #[test]
    fn without_an_address_there_is_nothing_to_ask() {
        let dto = rows(&cfg(""), Ok(Vec::new()));
        assert!(!dto.configured);
        assert!(dto.sonos.is_empty());
    }
}
