//! TuneIn radio: finding a station, and handing it to a Sonos.
//!
//! TuneIn's station search is open — no account, no key. Sonos then
//! plays a station by its TuneIn id, through whichever TuneIn service
//! the household has: the old one anybody can use (service 254), or the
//! newer one tied to an account (333 and a serial number). Which one is
//! learned from a station the household already plays, so a URI Niles
//! builds is the same shape as one the Sonos app built.

use crate::error::{Error, Result};
use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

/// One station, as TuneIn lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Station {
    /// TuneIn's id: "s24861".
    pub id: String,
    pub name: String,
    /// TuneIn's line under the name — a slogan, a language, what is on.
    pub about: String,
}

/// Which TuneIn service the household plays through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Account {
    /// Sonos's number for the service.
    pub service: u32,
    /// Which account on it, for a service that has accounts.
    pub serial: u32,
}

impl Account {
    /// The old TuneIn service, which needs no account. Used when the
    /// household has never played a TuneIn station Niles could learn
    /// from.
    pub const OPEN: Account = Account {
        service: 254,
        serial: 0,
    };

    /// The account a TuneIn station's URI was played through.
    pub fn of(uri: &str) -> Option<Account> {
        static TUNEIN: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"^x-sonosapi-stream:s\d+\?sid=(\d+)&flags=\d+&sn=(\d+)")
                .expect("valid regex")
        });
        let caps = TUNEIN.captures(uri)?;
        Some(Account {
            service: caps[1].parse().ok()?,
            serial: caps[2].parse().ok()?,
        })
    }

    /// The URI and metadata that make a Sonos play `station`.
    pub fn play(&self, station: &Station) -> (String, String) {
        let uri = format!(
            "x-sonosapi-stream:{}?sid={}&flags=8224&sn={}",
            station.id, self.service, self.serial
        );
        // How Sonos names the service in metadata: its number times 256
        // plus 7, and for the open TuneIn a name of its own.
        let descriptor = if self.service == Self::OPEN.service {
            "SA_RINCON65031_".to_string()
        } else {
            let kind = self.service * 256 + 7;
            format!("SA_RINCON{kind}_X_#Svc{kind}-0-Token")
        };
        let metadata = format!(
            r#"<DIDL-Lite xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:upnp="urn:schemas-upnp-org:metadata-1-0/upnp/" xmlns:r="urn:schemas-rinconnetworks-com:metadata-1-0/" xmlns="urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/"><item id="10092020{id}" parentID="L" restricted="true"><dc:title>{title}</dc:title><upnp:class>object.item.audioItem.audioBroadcast</upnp:class><desc id="cdudn" nameSpace="urn:schemas-rinconnetworks-com:metadata-1-0/">{descriptor}</desc></item></DIDL-Lite>"#,
            id = station.id,
            title = xml_text(&station.name),
        );
        (uri, metadata)
    }
}

fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    body: Vec<Outline>,
}

#[derive(Deserialize)]
struct Outline {
    #[serde(default)]
    guide_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    subtext: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

/// Stations whose name matches `query`, in TuneIn's order.
pub async fn search(http: &reqwest::Client, query: &str) -> Result<Vec<Station>> {
    let body = http
        .get("https://opml.radiotime.com/Search.ashx")
        .query(&[("query", query), ("types", "station"), ("render", "json")])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse(&body)
}

fn parse(body: &str) -> Result<Vec<Station>> {
    let response: Response = serde_json::from_str(body).map_err(|e| Error::ParseResponse {
        reason: format!("TuneIn search: {e}"),
    })?;
    Ok(response
        .body
        .into_iter()
        // Stations are audio outlines with an "s…" id; the rest are
        // links to artists and shows.
        .filter(|o| o.kind.as_deref() == Some("audio"))
        .filter_map(|o| {
            let id = o.guide_id.filter(|id| id.starts_with('s'))?;
            Some(Station {
                id,
                name: o.text?,
                about: o.subtext.unwrap_or_default(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p3() -> Station {
        Station {
            id: "s24861".into(),
            name: "DR P3".into(),
            about: "Danish".into(),
        }
    }

    #[test]
    fn learns_the_account_from_a_station_the_household_plays() {
        assert_eq!(
            Account::of("x-sonosapi-stream:s10136?sid=333&flags=8232&sn=14"),
            Some(Account {
                service: 333,
                serial: 14
            })
        );
        assert_eq!(
            Account::of("x-rincon-mp3radio://http://89.249.7.68/gofm"),
            None
        );
    }

    #[test]
    fn plays_a_station_the_way_the_sonos_app_does() {
        let account = Account {
            service: 333,
            serial: 14,
        };
        let (uri, metadata) = account.play(&p3());
        assert_eq!(uri, "x-sonosapi-stream:s24861?sid=333&flags=8224&sn=14");
        // The descriptor a favorite of the same station carries.
        assert!(metadata.contains(">SA_RINCON85255_X_#Svc85255-0-Token<"));
        assert!(metadata.contains("<dc:title>DR P3</dc:title>"));
    }

    #[test]
    fn the_open_service_has_a_descriptor_of_its_own() {
        let (uri, metadata) = Account::OPEN.play(&p3());
        assert_eq!(uri, "x-sonosapi-stream:s24861?sid=254&flags=8224&sn=0");
        assert!(metadata.contains(">SA_RINCON65031_<"));
    }

    #[test]
    fn reads_stations_and_skips_the_rest() {
        let body = r#"{"head":{"status":"200"},"body":[
            {"element":"outline","type":"link","text":"Artist: P4 with T","guide_id":"m1779836"},
            {"element":"outline","type":"audio","text":"DR P4 Fyn","subtext":"DR (Danish Broadcasting...","guide_id":"s8330","item":"station"},
            {"element":"outline","type":"audio","text":"DR P1","guide_id":"s24860","item":"station"}
        ]}"#;
        let stations = parse(body).unwrap();
        assert_eq!(stations.len(), 2);
        assert_eq!(stations[0].id, "s8330");
        assert_eq!(stations[1].about, "");
    }
}
