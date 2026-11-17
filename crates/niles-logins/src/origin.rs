//! Where a saved login may ever be used.

use crate::error::{Error, Result};
use serde::Serialize;
use url::{Host, Url};

/// Where a login may ever be used: the scheme, host and port of a site,
/// nothing more. It is always ASCII — an international look-alike name
/// reads as its `xn--…` punycode on every screen, so what the person
/// checks is what the browser reaches.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Origin(String);

impl Origin {
    /// The origin as it reads on screen, such as `https://shop.example`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A typed site address as an origin. `https://` is assumed when no
    /// scheme is given, `http` is lifted to `https`, and everything that
    /// Niles's browser couldn't reach anyway — a non-https scheme, an
    /// unusual port, credentials in the address, a name or IP that is
    /// never the public internet — is refused with a reason that never
    /// quotes what was typed.
    pub fn parse(given: &str) -> Result<Origin> {
        let given = given.trim();
        if given.is_empty() {
            return Err(site("a site is needed, such as https://shop.example"));
        }
        if given.chars().count() > 2000 {
            return Err(site("that site address is too long"));
        }
        let with_scheme = if given.contains("://") {
            given.to_string()
        } else {
            format!("https://{given}")
        };
        let mut url = Url::parse(&with_scheme).map_err(|_| {
            site("that doesn't read as a site address, such as https://shop.example")
        })?;
        match url.scheme() {
            "http" => url
                .set_scheme("https")
                .expect("https is a valid scheme for any http URL"),
            "https" => {}
            _ => {
                return Err(site(
                    "only https sites: Niles's browser reaches nothing else",
                ));
            }
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(site(
                "a site address can't carry a username or password; they go in their own fields",
            ));
        }
        if !matches!(url.port(), None | Some(443)) {
            return Err(site(
                "only https sites on their usual port, 443: Niles's browser reaches nothing else",
            ));
        }
        let public = match url.host() {
            Some(Host::Domain(d)) => niles_websearch::public_name(d),
            Some(Host::Ipv4(ip)) => niles_websearch::public_ip(ip.into()),
            Some(Host::Ipv6(ip)) => niles_websearch::public_ip(ip.into()),
            None => false,
        };
        if !public {
            return Err(site(
                "that site isn't on the public internet, and Niles's browser reaches nothing else",
            ));
        }
        let ascii = url.origin().ascii_serialization();
        Ok(Origin(ascii.trim_end_matches('.').to_string()))
    }
}

fn site(reason: &str) -> Error {
    Error::Invalid {
        kind: "site",
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_site_reads_as_one_origin() {
        for given in [
            "shop.example",
            "http://shop.example",
            "HTTPS://Shop.Example:443/login?next=/x#f",
            "https://shop.example.",
        ] {
            assert_eq!(
                Origin::parse(given).unwrap().as_str(),
                "https://shop.example"
            );
        }
    }

    #[test]
    fn an_international_name_reads_as_punycode() {
        assert_eq!(
            Origin::parse("https://bücher.example").unwrap().as_str(),
            "https://xn--bcher-kva.example"
        );
    }

    #[test]
    fn what_the_browser_cant_reach_is_refused() {
        for given in [
            "ftp://shop.example",
            "https://shop.example:8443",
            "https://user:pw@shop.example",
            "https://192.168.1.10",
            "https://localhost",
            "https://router.lan",
            "https://[::1]",
            "https://shop",
            "",
            "not a url at all ::",
        ] {
            assert!(Origin::parse(given).is_err(), "{given} should be refused");
        }
    }

    #[test]
    fn a_refusal_never_quotes_what_was_typed() {
        let err = Origin::parse("https://hunter2:pw@shop.example").unwrap_err();
        assert!(!err.to_string().contains("hunter2"));
    }
}
