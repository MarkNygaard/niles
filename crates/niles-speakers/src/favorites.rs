//! Sonos Favorites: what somebody starred in the Sonos app.
//!
//! The one place Niles can reach a station or a playlist exactly as the
//! household chose it — Spotify's own playlists included, which no
//! public API hands out any more — so they are tried before any search.

use crate::client::is_radio;
use crate::household::unescape;
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Favorite {
    /// As the Sonos app shows it: "DR P4 Østjylland".
    pub title: String,
    pub uri: String,
    /// The DIDL-Lite Sonos stored beside it, handed back when playing.
    pub metadata: String,
    /// A station, loaded directly. Anything else — a playlist, an
    /// album — goes through the queue.
    pub station: bool,
}

static ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<item\b.*?</item>").expect("valid regex"));
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<dc:title>(.*?)</dc:title>").expect("valid regex"));
static RES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<res\b[^>]*>(.*?)</res>").expect("valid regex"));
static RES_MD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<r:resMD>(.*?)</r:resMD>").expect("valid regex"));

/// The favorites in a Browse result, once unescaped. One without a URI
/// — Sonos's own "Discover Sonos Radio" shelves — is a folder for the
/// app to open, not something to play, and is left out.
pub(crate) fn parse(didl: &str) -> Vec<Favorite> {
    ITEM.find_iter(didl)
        .filter_map(|item| {
            let item = item.as_str();
            let title = unescape(TITLE.captures(item)?.get(1)?.as_str());
            let uri = unescape(RES.captures(item)?.get(1)?.as_str().trim());
            if uri.is_empty() {
                return None;
            }
            let metadata = RES_MD
                .captures(item)
                .and_then(|c| c.get(1))
                .map(|m| unescape(m.as_str()))
                .unwrap_or_default();
            let station = is_radio(&uri) || metadata.contains("audioItem.audioBroadcast");
            Some(Favorite {
                title,
                uri,
                metadata,
                station,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three of a real household's favorites, as Browse returns them
    /// once the SOAP layer is unescaped: a TuneIn station, a plain
    /// stream, and one of Sonos's own shelves with nothing to play.
    const DIDL: &str = r#"<DIDL-Lite xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/" xmlns:r="urn:schemas-rinconnetworks-com:metadata-1-0/" xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/"><item id="FV:2/1" parentID="FV:2" restricted="false"><dc:title>DR P3</dc:title><upnp:class>object.itemobject.item.sonos-favorite</upnp:class><res protocolInfo="x-sonosapi-stream:*:*:*">x-sonosapi-stream:s24861?sid=333&amp;flags=8224&amp;sn=14</res><r:resMD>&lt;DIDL-Lite&gt;&lt;item id="10092020s24861"&gt;&lt;dc:title&gt;DR P3&lt;/dc:title&gt;&lt;upnp:class&gt;object.item.audioItem.audioBroadcast&lt;/upnp:class&gt;&lt;desc id="cdudn"&gt;SA_RINCON85255_X_#Svc85255-0-Token&lt;/desc&gt;&lt;/item&gt;&lt;/DIDL-Lite&gt;</r:resMD></item><item id="FV:2/2" parentID="FV:2" restricted="false"><dc:title>GO FM!</dc:title><res protocolInfo="x-rincon-mp3radio:*:*:*">x-rincon-mp3radio://http://89.249.7.68/gofm</res></item><item id="FV:2/3" parentID="FV:2" restricted="false"><dc:title>Discover Sonos Radio</dc:title><r:resMD>&lt;DIDL-Lite&gt;&lt;/DIDL-Lite&gt;</r:resMD></item><item id="FV:2/4" parentID="FV:2" restricted="false"><dc:title>Discover Weekly</dc:title><res protocolInfo="x-rincon-cpcontainer:*:*:*">x-rincon-cpcontainer:1006206cspotify%3aplaylist%3a37i9dQZEVXcB?sid=12&amp;flags=8300&amp;sn=3</res><r:resMD>&lt;DIDL-Lite&gt;&lt;item&gt;&lt;upnp:class&gt;object.container.playlistContainer&lt;/upnp:class&gt;&lt;/item&gt;&lt;/DIDL-Lite&gt;</r:resMD></item></DIDL-Lite>"#;

    #[test]
    fn reads_each_playable_favorite() {
        let titles: Vec<_> = parse(DIDL).into_iter().map(|f| f.title).collect();
        assert_eq!(titles, ["DR P3", "GO FM!", "Discover Weekly"]);
    }

    #[test]
    fn keeps_the_uri_and_metadata_as_sonos_wants_them_back() {
        let p3 = &parse(DIDL)[0];
        assert_eq!(p3.uri, "x-sonosapi-stream:s24861?sid=333&flags=8224&sn=14");
        assert!(p3.metadata.starts_with("<DIDL-Lite>"));
        assert!(p3.metadata.contains("SA_RINCON85255_X_#Svc85255-0-Token"));
    }

    #[test]
    fn tells_a_station_from_a_playlist() {
        let favorites = parse(DIDL);
        assert!(favorites[0].station);
        assert!(favorites[1].station);
        assert!(!favorites[2].station);
    }
}
