//! Reaching only the public internet: what a person's own server, and
//! its sign-in, are held to.
//!
//! A person types the address, so it may name the house, the cluster or
//! Niles itself. The address must be `https` on a public name; every
//! address the name resolves to must be public, checked by the resolver
//! each time a connection is made, so DNS can't answer one thing to the
//! check and another to the connection; and redirects aren't followed,
//! so a public server can't send Niles somewhere private.

use crate::error::{Error, Result};
use niles_websearch::{public_ip, public_name};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use url::{Host, Url};

/// Where a client may connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Anywhere the household configured: its own servers.
    Anywhere,
    /// Only `https` on the public internet: a person's own servers.
    PublicInternet,
}

fn refused(reason: impl Into<String>) -> Error {
    Error::NotPublic(reason.into())
}

/// `address` as a URL a person's server may have: `https`, no user or
/// password in it, and a host on the public internet.
pub fn public_url(address: &str) -> Result<Url> {
    let url = Url::parse(address.trim()).map_err(|_| refused("it isn't a web address"))?;
    if url.scheme() != "https" {
        return Err(refused("it must start with https://"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refused("it can't carry a username or password"));
    }
    match url.host() {
        Some(Host::Domain(name)) if public_name(name) => Ok(url),
        Some(Host::Ipv4(ip)) if public_ip(IpAddr::V4(ip)) => Ok(url),
        Some(Host::Ipv6(ip)) if public_ip(IpAddr::V6(ip)) => Ok(url),
        _ => Err(refused(
            "it must be on the public internet, not the house's network, the cluster or Niles itself",
        )),
    }
}

/// Resolves a name only when every address it has is public.
struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            // Absolute, so the pod's search domains can't make a short
            // name a cluster Service.
            let found: Vec<SocketAddr> = tokio::net::lookup_host((format!("{host}."), 0))
                .await?
                .collect();
            if found.is_empty() || found.iter().any(|a| !public_ip(a.ip())) {
                return Err(format!("{host} isn't only on the public internet").into());
            }
            let addrs: Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}

/// An HTTP client for `reach`, with `timeout` per request.
pub fn client(reach: Reach, timeout: Duration) -> Result<reqwest::Client> {
    let builder = reqwest::Client::builder().timeout(timeout);
    let builder = match reach {
        Reach::Anywhere => builder,
        Reach::PublicInternet => builder
            .dns_resolver(Arc::new(PublicResolver))
            .redirect(reqwest::redirect::Policy::none()),
    };
    builder
        .build()
        .map_err(|e| Error::Unreachable(e.to_string()))
}

/// Checks `url` for `reach` before a request is sent.
pub fn check(url: &str, reach: Reach) -> Result<()> {
    match reach {
        Reach::Anywhere => Ok(()),
        Reach::PublicInternet => public_url(url).map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_on_the_public_internet_is_accepted() {
        assert!(public_url("https://mcp.notion.com/mcp").is_ok());
        for refused in [
            "http://mcp.notion.com/mcp",
            "https://localhost/mcp",
            "https://niles.default.svc.cluster.local/mcp",
            "https://niles-browser:9222/",
            "https://192.168.1.10/mcp",
            "https://10.0.0.5/mcp",
            "https://169.254.169.254/latest",
            "https://[::1]/mcp",
            "https://[fd00::1]/mcp",
            "https://user:pass@mcp.notion.com/mcp",
            "not an address",
        ] {
            assert!(public_url(refused).is_err(), "{refused}");
        }
    }

    #[tokio::test]
    async fn the_resolver_refuses_a_name_that_answers_inside() {
        // `localhost` resolves to loopback everywhere.
        let answered = PublicResolver
            .resolve("localhost".parse().expect("a name"))
            .await;
        assert!(answered.is_err());
    }
}
