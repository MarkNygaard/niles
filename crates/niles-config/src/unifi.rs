//! UniFi network presence configuration.

use crate::error::{Error, Result};
use serde::Deserialize;

/// `[presence.unifi]` — the network's own answer to who is home.
///
/// A phone joins the Wi-Fi the moment it is in range, which is before
/// the door and a good deal before a geofence has been reported to
/// anybody's cloud. The controller is on the LAN, so asking it costs
/// nothing and can be done every few seconds.
///
/// It is a second source rather than a replacement. Wi-Fi is excellent
/// at arrival and poor at departure — a phone lingers on the network,
/// and a sleeping one can drop off while its owner is on the sofa —
/// where a geofence is the other way round. The aggregator already
/// merges sources with "anybody says home, and it is home", so keeping
/// both gives the fast arrival and the careful departure.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnifiConfig {
    /// The console, as an address on the LAN: `192.168.1.1`.
    #[serde(default)]
    pub host: String,
    /// Which site, for a console that has more than one.
    #[serde(default = "default_site")]
    pub site: String,
    /// The environment variable holding the API key, when one is used.
    ///
    /// Empty is the normal case now: the key is typed into Settings and
    /// kept in the secret store. The variable stays supported because
    /// every other credential here works that way.
    #[serde(default)]
    pub api_key_env: String,
    /// How often to ask. Seconds.
    ///
    /// Short on purpose: this is a request on the local network with no
    /// quota behind it, and the whole reason for the integration is
    /// that five minutes is too long to stand in a dark hall.
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
}

fn default_site() -> String {
    "default".into()
}

fn default_poll_seconds() -> u64 {
    10
}

/// Written out rather than derived, and the two tests below are why:
/// `#[serde(default = "…")]` applies to *parsing*, so a derived
/// `Default` would hand code an empty site and a poll interval of
/// zero — values no config file could ever produce.
impl Default for UnifiConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            api_key_env: String::new(),
            site: default_site(),
            poll_seconds: default_poll_seconds(),
        }
    }
}

impl UnifiConfig {
    /// The API key, from the environment or the secret store.
    ///
    /// Generated in the console under Settings → Control Plane →
    /// Integrations. A key rather than an account on purpose: an
    /// account would carry the console's two-factor authentication into
    /// a background poll, where there is nobody to answer it.
    pub fn resolve_api_key(&self) -> Result<String> {
        crate::env::require_secret(
            "presence.unifi",
            "presence.unifi.api_key",
            &self.api_key_env,
        )
    }

    /// Whether there is enough here to connect.
    ///
    /// The key is not part of this: it lives in the secret store, and
    /// whether it is present is a question for whoever resolves it —
    /// asking here would mean the config knowing about credentials it
    /// deliberately never sees.
    pub fn is_configured(&self) -> bool {
        !self.host.trim().is_empty()
    }

    pub fn validate(&self) -> Result<()> {
        if !self.is_configured() {
            return Ok(());
        }
        // A host with a scheme is the mistake worth catching: the URL is
        // built around it, and `https://https://…` fails at the far end
        // with nothing pointing back here.
        if self.host.contains("://") {
            return Err(Error::InvalidSection {
                section: "presence.unifi",
                reason: format!("host {:?} should be an address, not a URL", self.host),
            });
        }
        if self.site.trim().is_empty() {
            return Err(Error::InvalidSection {
                section: "presence.unifi",
                reason: "site cannot be empty".into(),
            });
        }
        if !(2..=300).contains(&self.poll_seconds) {
            return Err(Error::InvalidSection {
                section: "presence.unifi",
                reason: "poll_seconds must be in 2..=300".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_configured_is_not_an_error() {
        // The section is optional, and a house without UniFi simply
        // goes on asking tado.
        assert!(UnifiConfig::default().validate().is_ok());
        assert!(!UnifiConfig::default().is_configured());
    }

    #[test]
    fn an_address_is_enough() {
        let cfg = UnifiConfig {
            host: "192.168.1.1".into(),
            ..Default::default()
        };
        assert!(cfg.is_configured());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn a_url_is_refused_where_an_address_belongs() {
        let cfg = UnifiConfig {
            host: "https://192.168.1.1".into(),
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("not a URL"), "{err}");
    }

    #[test]
    fn the_poll_has_to_be_quick_but_not_absurd() {
        let too_fast = UnifiConfig {
            host: "192.168.1.1".into(),
            poll_seconds: 1,
            ..Default::default()
        };
        assert!(too_fast.validate().is_err());
        assert_eq!(UnifiConfig::default().poll_seconds, 10);
    }
}
