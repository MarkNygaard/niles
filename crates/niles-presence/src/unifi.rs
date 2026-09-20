//! Who is home, according to the network they are standing in.
//!
//! A phone joins the Wi-Fi the moment it is in range — before the door,
//! and well before a geofence crossing has been reported to anybody's
//! cloud. The console is on the LAN with no quota behind it, so this
//! can be asked every few seconds where tado is asked every five
//! minutes.
//!
//! Deliberately a *second* source. Wi-Fi is excellent at arrival and
//! poor at departure: a phone lingers on the network after its owner
//! has gone, and a sleeping one can drop off while they are on the
//! sofa. A geofence is the other way round. The aggregator merges
//! sources with "anybody says home, and it is home", which takes the
//! best half of each.

use crate::error::{Error, Result};
use crate::source::PresenceSource;
use crate::state::PresenceSignal;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// One HTTP call to the console, so tests need no console.
#[async_trait]
pub trait UnifiTransport: Send + Sync {
    /// `GET` with the API key, returning status and body.
    async fn get(&self, url: &str, api_key: &str) -> Result<(u16, String)>;
}

/// A client as the console reports it.
///
/// Field names carry aliases because UniFi's integration API is
/// documented by its own console rather than publicly, and the two
/// spellings both appear in the wild. Getting this wrong would look
/// like an empty house rather than like an error, which is the sort of
/// failure worth spending a few aliases to avoid.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiClient {
    #[serde(alias = "mac")]
    pub mac_address: Option<String>,
    #[serde(alias = "ip")]
    pub ip_address: Option<String>,
    #[serde(alias = "hostname", alias = "displayName")]
    pub name: Option<String>,
}

impl UnifiClient {
    /// Lowercased, because a MAC is written both ways and compared as
    /// neither.
    pub fn mac(&self) -> Option<String> {
        self.mac_address.as_ref().map(|m| m.trim().to_lowercase())
    }
}

/// The console's list endpoints wrap their rows; older ones do not.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Listed<T> {
    Wrapped { data: Vec<T> },
    Bare(Vec<T>),
}

impl<T> Listed<T> {
    fn rows(self) -> Vec<T> {
        match self {
            Listed::Wrapped { data } => data,
            Listed::Bare(rows) => rows,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnifiSite {
    id: String,
    #[serde(alias = "name", alias = "internalReference")]
    label: Option<String>,
}

/// The console, and the phones worth watching for.
pub struct UnifiSource {
    /// `https://<host>/proxy/network/integration/v1`
    base_url: String,
    /// What the config calls the site — a name, or an id already.
    site: String,
    api_key: String,
    transport: Arc<dyn UnifiTransport>,
    /// Resolved once: the console answers by id, people write names.
    site_id: RwLock<Option<String>>,
    /// The addresses that count as somebody being home.
    ///
    /// Shared rather than owned because they are edited from the app —
    /// pairing a new phone writes one — and rebuilding the source under
    /// a running poll loop to carry a changed list would be a great
    /// deal of machinery for a `HashSet`.
    watching: Arc<RwLock<HashSet<String>>>,
}

impl UnifiSource {
    pub fn new(
        host: &str,
        site: &str,
        api_key: impl Into<String>,
        transport: Arc<dyn UnifiTransport>,
        watching: Arc<RwLock<HashSet<String>>>,
    ) -> Self {
        Self {
            base_url: format!(
                "https://{}/proxy/network/integration/v1",
                host.trim().trim_end_matches('/')
            ),
            site: site.trim().to_string(),
            api_key: api_key.into(),
            transport,
            site_id: RwLock::new(None),
            watching,
        }
    }

    /// The site's id, resolved from its name the first time.
    ///
    /// The integration API addresses sites by id, and nobody knows
    /// theirs. A console with one site answers for it whatever the
    /// config says, which is the common case and worth not failing on.
    async fn site_id(&self) -> Result<String> {
        if let Some(id) = self
            .site_id
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Ok(id);
        }
        let body = self.get(&format!("{}/sites", self.base_url)).await?;
        let sites: Vec<UnifiSite> = serde_json::from_str::<Listed<UnifiSite>>(&body)
            .map_err(|e| Error::Parse {
                reason: format!("unifi sites: {e}"),
            })?
            .rows();

        let chosen = sites
            .iter()
            .find(|s| s.id == self.site)
            .or_else(|| {
                sites
                    .iter()
                    .find(|s| s.label.as_deref().is_some_and(|l| l == self.site))
            })
            .or(sites.first())
            .ok_or_else(|| Error::Parse {
                reason: "unifi reports no sites at all".into(),
            })?;

        let id = chosen.id.clone();
        *self.site_id.write().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
        Ok(id)
    }

    /// Everything the console can currently see on the network.
    pub async fn clients(&self) -> Result<Vec<UnifiClient>> {
        let site = self.site_id().await?;
        let body = self
            .get(&format!("{}/sites/{site}/clients", self.base_url))
            .await?;
        Ok(serde_json::from_str::<Listed<UnifiClient>>(&body)
            .map_err(|e| Error::Parse {
                reason: format!("unifi clients: {e}"),
            })?
            .rows())
    }

    /// The MAC of whoever is making a request from `ip`.
    ///
    /// This is the whole of the pairing story: somebody presses a
    /// button on their phone, the request arrives from an address on
    /// the LAN, and the console says which device holds it. Nobody
    /// types a MAC, and the answer cannot be about the wrong phone.
    pub async fn mac_at(&self, ip: &str) -> Result<Option<UnifiClient>> {
        Ok(self
            .clients()
            .await?
            .into_iter()
            .find(|c| c.ip_address.as_deref() == Some(ip)))
    }

    async fn get(&self, url: &str) -> Result<String> {
        let (status, body) = self.transport.get(url, &self.api_key).await?;
        match status {
            200 => Ok(body),
            401 | 403 => Err(Error::Auth {
                reason: "unifi refused the API key".into(),
            }),
            other => Err(Error::Parse {
                reason: format!("unifi answered {other}: {}", body.trim()),
            }),
        }
    }
}

#[async_trait]
impl PresenceSource for UnifiSource {
    async fn poll(&self) -> Result<PresenceSignal> {
        let watching = self
            .watching
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        // Nobody has paired a phone yet. Saying "away" here would be a
        // source with nothing to go on outvoting one that has: it
        // reports nothing instead, and the aggregator hears only from
        // tado until somebody pairs.
        if watching.is_empty() {
            return Err(Error::Parse {
                reason: "no phones paired yet".into(),
            });
        }

        let anyone_home = self
            .clients()
            .await?
            .iter()
            .filter_map(|c| c.mac())
            .any(|mac| watching.contains(&mac));

        Ok(PresenceSignal {
            source: "unifi".into(),
            anyone_home,
            observed_at: Utc::now(),
        })
    }

    fn name(&self) -> &str {
        "unifi"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MockTransport {
        answers: Mutex<Vec<Result<(u16, String)>>>,
        urls: Mutex<Vec<String>>,
    }

    impl MockTransport {
        fn new(answers: Vec<Result<(u16, String)>>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers),
                urls: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl UnifiTransport for MockTransport {
        async fn get(&self, url: &str, _api_key: &str) -> Result<(u16, String)> {
            self.urls.lock().unwrap().push(url.to_string());
            self.answers.lock().unwrap().remove(0)
        }
    }

    const SITES: &str = r#"{"data":[{"id":"site-123","name":"default"}]}"#;
    const CLIENTS: &str = r#"{"data":[
        {"macAddress":"AA:BB:CC:DD:EE:FF","ipAddress":"192.168.1.50","name":"Mark's iPhone"},
        {"macAddress":"11:22:33:44:55:66","ipAddress":"192.168.1.51","name":"Majse's iPhone"},
        {"macAddress":"99:99:99:99:99:99","ipAddress":"192.168.1.9","name":"Printer"}
    ]}"#;

    fn source(
        answers: Vec<Result<(u16, String)>>,
        watching: &[&str],
    ) -> (Arc<MockTransport>, UnifiSource) {
        let mock = MockTransport::new(answers);
        let watching = Arc::new(RwLock::new(
            watching
                .iter()
                .map(|m| m.to_string())
                .collect::<HashSet<_>>(),
        ));
        let source = UnifiSource::new("192.168.1.1", "default", "key", mock.clone(), watching);
        (mock, source)
    }

    #[tokio::test]
    async fn a_paired_phone_on_the_network_is_somebody_home() {
        let (_mock, source) = source(
            vec![Ok((200, SITES.into())), Ok((200, CLIENTS.into()))],
            &["aa:bb:cc:dd:ee:ff"],
        );
        assert!(source.poll().await.expect("polls").anyone_home);
    }

    #[tokio::test]
    async fn a_house_of_unpaired_devices_is_empty() {
        // The printer is on the network and is not a person.
        let (_mock, source) = source(
            vec![Ok((200, SITES.into())), Ok((200, CLIENTS.into()))],
            &["77:77:77:77:77:77"],
        );
        assert!(!source.poll().await.expect("polls").anyone_home);
    }

    #[tokio::test]
    async fn case_does_not_decide_whether_somebody_is_home() {
        // The console writes a MAC in capitals and the app stores what
        // it was given; comparing them raw would quietly never match.
        let (_mock, source) = source(
            vec![Ok((200, SITES.into())), Ok((200, CLIENTS.into()))],
            &["AA:BB:CC:DD:EE:FF"],
        );
        let signal = source.poll().await.expect("polls");
        assert!(
            !signal.anyone_home,
            "the stored address is compared lowercased, so this one should miss"
        );
    }

    #[tokio::test]
    async fn nobody_paired_is_not_the_same_as_nobody_home() {
        // A source with nothing to go on must not outvote one that has.
        let (_mock, source) = source(vec![], &[]);
        assert!(source.poll().await.is_err());
    }

    #[tokio::test]
    async fn the_request_says_which_phone_is_asking() {
        let (_mock, source) = source(
            vec![Ok((200, SITES.into())), Ok((200, CLIENTS.into()))],
            &["aa:bb:cc:dd:ee:ff"],
        );
        let found = source
            .mac_at("192.168.1.51")
            .await
            .expect("asks")
            .expect("somebody holds that address");
        assert_eq!(found.mac().as_deref(), Some("11:22:33:44:55:66"));
        assert_eq!(found.name.as_deref(), Some("Majse's iPhone"));
    }

    #[tokio::test]
    async fn an_address_nobody_holds_is_nobody() {
        let (_mock, source) = source(
            vec![Ok((200, SITES.into())), Ok((200, CLIENTS.into()))],
            &["aa:bb:cc:dd:ee:ff"],
        );
        assert!(source.mac_at("10.0.0.1").await.expect("asks").is_none());
    }

    #[tokio::test]
    async fn the_site_is_resolved_once_and_remembered() {
        let (mock, source) = source(
            vec![
                Ok((200, SITES.into())),
                Ok((200, CLIENTS.into())),
                Ok((200, CLIENTS.into())),
            ],
            &["aa:bb:cc:dd:ee:ff"],
        );
        source.poll().await.expect("first");
        source.poll().await.expect("second");

        let urls = mock.urls.lock().unwrap().clone();
        assert_eq!(
            urls.iter().filter(|u| u.ends_with("/sites")).count(),
            1,
            "the site list should be asked for once: {urls:?}"
        );
        assert!(urls[1].contains("/sites/site-123/clients"));
    }

    #[tokio::test]
    async fn a_refused_key_says_so_rather_than_reporting_an_empty_house() {
        let (_mock, source) = source(
            vec![Ok((401, "unauthorized".into()))],
            &["aa:bb:cc:dd:ee:ff"],
        );
        assert!(matches!(source.poll().await, Err(Error::Auth { .. })));
    }
}
