//! Spotify: finding what to play, and handing it to a Sonos.
//!
//! Two halves, kept apart on purpose. Finding uses Spotify's Web API
//! with an app's own keys (the client credentials flow), which can
//! search the catalogue without anybody's login. Playing is Sonos's:
//! the household's Spotify account is linked in the Sonos app, and a
//! track, album or playlist is put in a speaker's queue in the shape the
//! Sonos app uses, so Sonos fetches it through that account.
//!
//! Spotify took "an artist's top tracks" away from new apps in February
//! 2026, so an artist plays as the tracks a search for them returns —
//! ten at most, which is what search allows.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// The Spotify developer app's keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Artist,
    Track,
    Album,
    Playlist,
}

/// Something in Spotify's catalogue.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Item {
    pub kind: Kind,
    /// `spotify:track:…`
    pub uri: String,
    pub name: String,
    /// The artist, for a track or an album.
    pub by: Option<String>,
}

/// What a search found, by kind, in Spotify's order of relevance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub artists: Vec<Item>,
    pub tracks: Vec<Item>,
    pub albums: Vec<Item>,
    pub playlists: Vec<Item>,
}

pub struct SpotifyClient {
    http: reqwest::Client,
    /// The app's access token, and when it stops working.
    token: Mutex<Option<(Credentials, String, Instant)>>,
}

impl Default for SpotifyClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SpotifyClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .expect("reqwest TLS init"),
            token: Mutex::new(None),
        }
    }

    /// Artists, tracks, albums and playlists matching `query`, as sold
    /// in `market` (a country code: "DK") when there is one.
    pub async fn search(
        &self,
        credentials: &Credentials,
        query: &str,
        market: Option<&str>,
    ) -> Result<Found> {
        self.get(credentials, query, "artist,track,album,playlist", market)
            .await
    }

    /// Tracks by `artist`, most relevant first — what "play John Mayer"
    /// plays, now that Spotify keeps an artist's top tracks to itself.
    ///
    /// A plain search for the name, kept to their own songs: the
    /// `artist:` filter answers with half as many.
    pub async fn tracks_by(
        &self,
        credentials: &Credentials,
        artist: &str,
        market: Option<&str>,
    ) -> Result<Vec<Item>> {
        let found = self.get(credentials, artist, "track", market).await?;
        Ok(own_songs(found.tracks, artist))
    }

    /// One song by one artist: "Gravity by John Mayer".
    pub async fn track(
        &self,
        credentials: &Credentials,
        title: &str,
        artist: &str,
        market: Option<&str>,
    ) -> Result<Option<Item>> {
        let found = self
            .get(
                credentials,
                &format!("track:\"{title}\" artist:\"{artist}\""),
                "track",
                market,
            )
            .await?;
        Ok(found.tracks.into_iter().next())
    }

    async fn get(
        &self,
        credentials: &Credentials,
        query: &str,
        types: &str,
        market: Option<&str>,
    ) -> Result<Found> {
        let token = self.token(credentials).await?;
        let mut params = vec![
            ("q", query),
            ("type", types),
            // The most Spotify allows a development app since 2026.
            ("limit", "10"),
        ];
        // Only what can be played where the house is.
        if let Some(market) = market {
            params.push(("market", market));
        }
        let body = self
            .http
            .get("https://api.spotify.com/v1/search")
            .bearer_auth(token)
            .query(&params)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        parse(&body)
    }

    /// The app's token, fetched again shortly before it runs out or when
    /// the keys change.
    async fn token(&self, credentials: &Credentials) -> Result<String> {
        let mut held = self.token.lock().await;
        if let Some((for_keys, token, until)) = held.as_ref()
            && for_keys == credentials
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }
        #[derive(Deserialize)]
        struct Token {
            access_token: String,
            expires_in: u64,
        }
        let token: Token = self
            .http
            .post("https://accounts.spotify.com/api/token")
            .basic_auth(&credentials.client_id, Some(&credentials.client_secret))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await?
            .error_for_status()
            .map_err(|e| Error::ParseResponse {
                reason: format!("Spotify refused the app's keys: {e}"),
            })?
            .json()
            .await?;
        let until = Instant::now() + Duration::from_secs(token.expires_in.saturating_sub(60));
        *held = Some((credentials.clone(), token.access_token.clone(), until));
        Ok(token.access_token)
    }
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    artists: Option<Page>,
    #[serde(default)]
    tracks: Option<Page>,
    #[serde(default)]
    albums: Option<Page>,
    #[serde(default)]
    playlists: Option<Page>,
}

#[derive(Deserialize)]
struct Page {
    // Spotify answers `null` for an entry it will not show.
    #[serde(default)]
    items: Vec<Option<Raw>>,
}

#[derive(Deserialize)]
struct Raw {
    uri: String,
    name: String,
    #[serde(default)]
    artists: Vec<RawArtist>,
}

#[derive(Deserialize)]
struct RawArtist {
    name: String,
}

fn parse(body: &str) -> Result<Found> {
    let response: Response = serde_json::from_str(body).map_err(|e| Error::ParseResponse {
        reason: format!("Spotify search: {e}"),
    })?;
    let items = |page: Option<Page>, kind: Kind| -> Vec<Item> {
        page.map(|p| p.items)
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .map(|raw| Item {
                kind,
                uri: raw.uri,
                name: raw.name,
                by: raw.artists.into_iter().next().map(|a| a.name),
            })
            .collect()
    };
    Ok(Found {
        artists: items(response.artists, Kind::Artist),
        tracks: items(response.tracks, Kind::Track),
        albums: items(response.albums, Kind::Album),
        playlists: items(response.playlists, Kind::Playlist),
    })
}

/// The songs by `artist`, each once: the same song is often there twice,
/// from an album and from a single.
fn own_songs(tracks: Vec<Item>, artist: &str) -> Vec<Item> {
    let mut seen: Vec<String> = Vec::new();
    tracks
        .into_iter()
        .filter(|t| {
            t.by.as_deref()
                .is_some_and(|by| by.eq_ignore_ascii_case(artist))
        })
        .filter(|t| {
            let name = t.name.to_lowercase();
            let first = !seen.contains(&name);
            seen.push(name);
            first
        })
        .collect()
}

/// What Sonos calls Spotify. It differs by region (2311 in most of the
/// world, 3079 for accounts made in the US), so it is read from the
/// services the household has linked: see [`crate::SonosClient::linked_services`].
pub const SPOTIFY_SERVICES: [u32; 2] = [3079, 2311];

/// The URI and metadata that put `item` in a Sonos queue through
/// Spotify service `service`. An artist is not one of them: it plays as
/// its tracks.
pub fn enqueue(item: &Item, service: u32) -> Option<(String, String)> {
    let encoded = item.uri.replace(':', "%3a");
    let (prefix, key, class) = match item.kind {
        Kind::Track => ("", "00032020", "object.item.audioItem.musicTrack"),
        Kind::Album => (
            "x-rincon-cpcontainer:1004206c",
            "00040000",
            "object.container.album.musicAlbum",
        ),
        Kind::Playlist => (
            "x-rincon-cpcontainer:1006206c",
            "1006206c",
            "object.container.playlistContainer",
        ),
        Kind::Artist => return None,
    };
    let title = item
        .name
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let metadata = format!(
        r#"<DIDL-Lite xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/" xmlns:r="urn:schemas-rinconnetworks-com:metadata-1-0/" xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/"><item id="{key}{encoded}" parentID="-1" restricted="true"><dc:title>{title}</dc:title><upnp:class>{class}</upnp:class><desc id="cdudn" nameSpace="urn:schemas-rinconnetworks-com:metadata-1-0/">SA_RINCON{service}_X_#Svc{service}-0-Token</desc></item></DIDL-Lite>"#
    );
    Some((format!("{prefix}{encoded}"), metadata))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: Kind, uri: &str) -> Item {
        Item {
            kind,
            uri: uri.into(),
            name: "Continuum".into(),
            by: Some("John Mayer".into()),
        }
    }

    #[test]
    fn a_track_is_queued_by_its_encoded_uri() {
        let (uri, metadata) = enqueue(&item(Kind::Track, "spotify:track:abc"), 3079).unwrap();
        assert_eq!(uri, "spotify%3atrack%3aabc");
        assert!(metadata.contains(r#"<item id="00032020spotify%3atrack%3aabc""#));
        assert!(metadata.contains(">SA_RINCON3079_X_#Svc3079-0-Token<"));
    }

    #[test]
    fn an_album_and_a_playlist_are_containers() {
        let (album, metadata) = enqueue(&item(Kind::Album, "spotify:album:xyz"), 2311).unwrap();
        assert_eq!(album, "x-rincon-cpcontainer:1004206cspotify%3aalbum%3axyz");
        assert!(metadata.contains("object.container.album.musicAlbum"));
        assert!(metadata.contains("SA_RINCON2311_X_#Svc2311-0-Token"));
        let (playlist, _) = enqueue(&item(Kind::Playlist, "spotify:playlist:p1"), 3079).unwrap();
        assert_eq!(
            playlist,
            "x-rincon-cpcontainer:1006206cspotify%3aplaylist%3ap1"
        );
    }

    #[test]
    fn an_artist_plays_as_its_tracks_not_as_itself() {
        assert!(enqueue(&item(Kind::Artist, "spotify:artist:a"), 3079).is_none());
    }

    fn song(name: &str, by: &str) -> Item {
        Item {
            kind: Kind::Track,
            uri: format!("spotify:track:{name}"),
            name: name.into(),
            by: Some(by.into()),
        }
    }

    #[test]
    fn an_artist_plays_their_own_songs_once_each() {
        let tracks = vec![
            song("New Light", "John Mayer"),
            song("Gravity", "John Mayer"),
            song("All of Me", "John Legend"),
            song("Gravity", "John Mayer"),
        ];
        let names: Vec<_> = own_songs(tracks, "john mayer")
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["New Light", "Gravity"]);
    }

    #[test]
    fn reads_a_search_and_skips_what_spotify_withholds() {
        let body = r#"{
          "artists": {"items": [{"uri": "spotify:artist:a1", "name": "John Mayer", "artists": []}]},
          "tracks": {"items": [{"uri": "spotify:track:t1", "name": "Gravity", "artists": [{"name": "John Mayer"}]}]},
          "albums": {"items": []},
          "playlists": {"items": [null, {"uri": "spotify:playlist:p1", "name": "Mayer mix"}]}
        }"#;
        let found = parse(body).unwrap();
        assert_eq!(found.artists[0].name, "John Mayer");
        assert_eq!(found.tracks[0].by.as_deref(), Some("John Mayer"));
        assert_eq!(found.playlists.len(), 1);
    }
}
