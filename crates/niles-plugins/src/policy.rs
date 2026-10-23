//! Where a plugin may send HTTP and WebSocket requests.
//!
//! Pure functions over the manifest and IP addresses, so each rule has a
//! test of its own; [`crate::http`] does the I/O around them. Two checks,
//! because a host name proves nothing about where it points:
//!
//! 1. [`HttpPolicy::admit`], before any DNS: is the URL's host declared,
//!    under `http` (exact, or `*.example.com`) or `local_network`, and is
//!    the scheme one it may use?
//! 2. [`HttpPolicy::allows`], for each address the host resolved to.
//!    Loopback and the cloud metadata address are never reachable.
//!    Private, link-local and unique-local addresses only when
//!    `local_network` covers them, whatever the host name says, and
//!    public addresses only for an `http` host over https.
//!
//! The request is then made to the address that passed, never resolved
//! again, so DNS cannot be rebound between the check and the connection.
//!
//! MQTT brokers ([`MqttPolicy`]) get the same two checks, with the
//! broker's own entry in place of the two lists: see [`MqttTarget::allows`].

use crate::manifest::{MqttBroker, Permissions, TlsMode};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub(crate) fn ws_address_allowed(ip: IpAddr, local: bool, loopback: bool) -> Result<(), String> {
    let ip = canonical(ip);
    if loopback && ip.is_loopback() {
        return Ok(());
    }
    if let Some(why) = never(ip) {
        return Err(format!("policy denied {why} address"));
    }
    if is_local(ip) && !local {
        return Err("policy denied undeclared local address".into());
    }
    if !is_local(ip) && local {
        return Err("policy denied public address for local grant".into());
    }
    Ok(())
}

pub(crate) fn is_local_network_address(ip: IpAddr) -> bool {
    is_local(canonical(ip))
}

/// What a plugin's manifest lets it reach, with `setting:` entries
/// already replaced by what the household entered.
#[derive(Debug, Clone, Default)]
pub struct HttpPolicy {
    hosts: Vec<String>,
    local: Vec<Local>,
    /// See [`HttpPolicy::allowing_loopback`].
    loopback: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Local {
    Addr(IpAddr),
    Net(IpAddr, u8),
    Name(String),
    /// A `setting:` address that names this machine. Reached only under
    /// `niles plugin dev --allow-loopback`, and otherwise nothing.
    Loopback,
}

/// A URL that passed the first check, and what it passed as.
#[derive(Debug, Clone)]
pub struct Admitted {
    /// Lower case, without brackets for IPv6.
    pub host: String,
    pub https: bool,
    /// Declared under `http`.
    internet: bool,
    /// Declared by name under `local_network`.
    local_name: bool,
}

impl HttpPolicy {
    /// `setting` gives what the household entered for a setting, for the
    /// `setting:<key>` entries. One left empty reaches nothing.
    pub fn new(permissions: &Permissions, setting: impl Fn(&str) -> Option<String>) -> Self {
        let local = permissions
            .local_network
            .iter()
            .filter_map(|entry| match entry.strip_prefix("setting:") {
                Some(key) => setting(key).and_then(|value| parse_setting_address(&value)),
                None => parse_local(entry).ok(),
            })
            .collect();
        Self {
            hosts: permissions
                .http
                .iter()
                .map(|h| h.trim().to_ascii_lowercase())
                .collect(),
            local,
            loopback: false,
        }
    }

    /// Let a `setting:` address that names this machine (`127.0.0.1`,
    /// `::1`, `localhost`) through, for a fake device under `niles plugin
    /// dev --allow-loopback`. Only the dev command sets it.
    pub(crate) fn allowing_loopback(mut self) -> Self {
        self.loopback = true;
        self
    }

    fn reaches_loopback(&self) -> bool {
        self.loopback && self.local.contains(&Local::Loopback)
    }

    /// Whether the manifest lets the plugin reach anything at all.
    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty() && self.local.is_empty()
    }

    /// Whether `host` is one of the `http` hosts, which is what a page's
    /// image may be fetched from.
    pub fn declares_host(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.hosts.iter().any(|p| host_matches(p, &host))
    }

    /// The first check, before DNS.
    pub fn admit(&self, url: &reqwest::Url) -> Result<Admitted, String> {
        let https = match url.scheme() {
            "https" => true,
            "http" => false,
            other => {
                return Err(format!(
                    "{other}: URLs are not allowed, only http and https"
                ));
            }
        };
        let (host, literal) = match url.host() {
            Some(url::Host::Domain(d)) => (d.trim_end_matches('.').to_ascii_lowercase(), None),
            Some(url::Host::Ipv4(ip)) => (ip.to_string(), Some(IpAddr::V4(ip))),
            Some(url::Host::Ipv6(ip)) => (ip.to_string(), Some(IpAddr::V6(ip))),
            None => return Err("the URL has no host".into()),
        };
        let this_machine =
            host == "localhost" || literal.is_some_and(|ip| canonical(ip).is_loopback());
        if this_machine && self.reaches_loopback() {
            return Ok(Admitted {
                host,
                https,
                internet: false,
                local_name: true,
            });
        }
        if let Some(why) = literal.and_then(|ip| never(canonical(ip))) {
            return Err(format!("{host} is {why}, which no plugin may reach"));
        }
        let internet = literal.is_none() && self.hosts.iter().any(|p| host_matches(p, &host));
        let local_name = literal.is_none()
            && self
                .local
                .iter()
                .any(|l| matches!(l, Local::Name(n) if *n == host));
        let local_literal = literal.is_some_and(|ip| self.covers(canonical(ip)));
        if !internet && !local_name && !local_literal {
            return Err(format!(
                "{host} is not in the manifest's http or local_network permission"
            ));
        }
        if !https && internet && !local_name {
            return Err(format!(
                "{host} is on the internet, which needs https; plain http is for the local network"
            ));
        }
        Ok(Admitted {
            host,
            https,
            internet,
            local_name,
        })
    }

    /// The second check, for one address `admitted.host` resolved to.
    pub fn allows(&self, admitted: &Admitted, ip: IpAddr) -> Result<(), String> {
        let ip = canonical(ip);
        let host = &admitted.host;
        if ip.is_loopback() && !admitted.internet && self.reaches_loopback() {
            return Ok(());
        }
        if let Some(why) = never(ip) {
            return Err(format!("{host} is {ip}, {why}, which no plugin may reach"));
        }
        if is_local(ip) {
            if admitted.local_name || self.covers(ip) {
                return Ok(());
            }
            return Err(format!(
                "{host} is {ip}, on the local network, which local_network does not list"
            ));
        }
        if !admitted.internet {
            return Err(format!(
                "{host} is {ip}, which is not on the local network and not an http host"
            ));
        }
        if !admitted.https {
            return Err(format!(
                "{host} is {ip}, on the internet, which needs https"
            ));
        }
        Ok(())
    }

    fn covers(&self, ip: IpAddr) -> bool {
        self.local.iter().any(|l| match l {
            Local::Addr(a) => canonical(*a) == ip,
            Local::Net(base, prefix) => in_net(ip, *base, *prefix),
            Local::Name(_) | Local::Loopback => false,
        })
    }
}

/// An `http` permission entry, as a sentence when it is wrong.
pub fn check_host_pattern(pattern: &str) -> Result<(), String> {
    let name = pattern.strip_prefix("*.").unwrap_or(pattern);
    if pattern.contains("://") || pattern.contains('/') {
        return Err(format!("{pattern:?}: host names only, no scheme or path"));
    }
    if name.contains('*') {
        return Err(format!(
            "{pattern:?}: a wildcard is only allowed as a leading *."
        ));
    }
    if name.parse::<IpAddr>().is_ok() || name.trim_matches(['[', ']']).parse::<IpAddr>().is_ok() {
        return Err(format!(
            "{pattern:?}: an address belongs under local_network, not http"
        ));
    }
    if name.contains(':') {
        return Err(format!("{pattern:?}: no ports, only host names"));
    }
    if pattern.starts_with("*.") && !name.contains('.') {
        return Err(format!("{pattern:?} is too broad"));
    }
    check_host_name(name).map_err(|why| format!("{pattern:?}: {why}"))
}

/// A `local_network` entry other than `setting:`, as a sentence when it
/// is wrong.
pub fn check_local_entry(entry: &str) -> Result<(), String> {
    parse_local(entry).map(|_| ())
}

fn parse_local(entry: &str) -> Result<Local, String> {
    if let Some((base, prefix)) = entry.split_once('/') {
        let base: IpAddr = base
            .parse()
            .map_err(|_| format!("{entry:?} is not a range like 192.168.20.0/24"))?;
        let prefix: u8 = prefix
            .parse()
            .map_err(|_| format!("{entry:?} is not a range like 192.168.20.0/24"))?;
        let (narrowest, widest) = match base {
            IpAddr::V4(_) => (32, 24),
            IpAddr::V6(_) => (128, 120),
        };
        if prefix > narrowest {
            return Err(format!("{entry:?} is not a range"));
        }
        if prefix < widest {
            return Err(format!(
                "{entry:?} is wider than a /24; list the addresses, or use a setting"
            ));
        }
        if !is_local(canonical(base)) {
            return Err(format!("{entry:?} is not on the home network"));
        }
        return Ok(Local::Net(base, prefix));
    }
    if let Ok(ip) = entry.parse::<IpAddr>() {
        let ip = canonical(ip);
        if let Some(why) = never(ip) {
            return Err(format!("{entry:?} is {why}, which no plugin may reach"));
        }
        if !is_local(ip) {
            return Err(format!(
                "{entry:?} is not on the home network; internet hosts go under http"
            ));
        }
        return Ok(Local::Addr(ip));
    }
    check_host_name(entry).map_err(|why| format!("{entry:?}: {why}"))?;
    Ok(Local::Name(entry.to_ascii_lowercase()))
}

/// The host in what the household typed into a `setting:` address: an
/// IP or a host name, perhaps with a scheme, a port or a path they pasted
/// along with it.
pub(crate) fn setting_host(value: &str) -> &str {
    let value = value.trim();
    let rest = value.split_once("://").map_or(value, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    match authority.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
        None if authority.matches(':').count() == 1 => {
            authority.split(':').next().unwrap_or_default()
        }
        None => authority,
    }
}

/// What the household typed into a `setting:` address, as a local entry.
fn parse_setting_address(value: &str) -> Option<Local> {
    let host = setting_host(value);
    if host.eq_ignore_ascii_case("localhost") {
        return Some(Local::Loopback);
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        let ip = canonical(ip);
        if ip.is_loopback() {
            return Some(Local::Loopback);
        }
        return (never(ip).is_none() && is_local(ip)).then_some(Local::Addr(ip));
    }
    check_host_name(host)
        .ok()
        .map(|()| Local::Name(host.to_ascii_lowercase()))
}

/// A `[[permissions.mqtt]]` host other than `setting:`, as a sentence
/// when it is wrong.
pub fn check_mqtt_host(host: &str, tls: TlsMode) -> Result<(), String> {
    if host.contains("://") || host.contains('/') {
        return Err(format!(
            "{host:?}: a host name or an address only, no scheme or path"
        ));
    }
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        let ip = canonical(ip);
        if let Some(why) = never(ip) {
            return Err(format!("{host:?} is {why}, which no plugin may reach"));
        }
        if tls != TlsMode::Verify && !is_local(ip) {
            return Err(format!(
                "{host:?} is not on the home network, and tls = \"{}\" is only for the home network",
                tls.as_str()
            ));
        }
        return Ok(());
    }
    if host.contains(':') {
        return Err(format!("{host:?}: the port goes in port, not in host"));
    }
    if host.contains('*') {
        return Err(format!("{host:?}: a broker is one host, without wildcards"));
    }
    check_host_name(host).map_err(|why| format!("{host:?}: {why}"))
}

/// The brokers a plugin's manifest names, with `setting:` hosts replaced
/// by what the household entered.
#[derive(Debug, Clone, Default)]
pub struct MqttPolicy {
    brokers: Vec<MqttBroker>,
    /// The `setting:` values, by key.
    entered: HashMap<String, String>,
    loopback: bool,
}

impl MqttPolicy {
    pub fn new(permissions: &Permissions, setting: impl Fn(&str) -> Option<String>) -> Self {
        let entered = permissions
            .mqtt
            .iter()
            .filter_map(MqttBroker::setting)
            .filter_map(|key| Some((key.to_string(), setting(key)?)))
            .collect();
        Self {
            brokers: permissions.mqtt.clone(),
            entered,
            loopback: false,
        }
    }

    /// Let brokers on this machine through: the tests' broker, and a fake
    /// device under `niles plugin dev --allow-loopback`. Only those set it.
    pub(crate) fn allowing_loopback(mut self) -> Self {
        self.loopback = true;
        self
    }

    /// Where `name` is, if the manifest names it: the target, or why it
    /// cannot be reached as it stands.
    pub fn target(&self, name: &str) -> Option<Result<MqttTarget, String>> {
        let broker = self.brokers.iter().find(|b| b.name == name)?;
        Some(self.resolve(broker))
    }

    fn resolve(&self, broker: &MqttBroker) -> Result<MqttTarget, String> {
        let (host, entered) = match broker.setting() {
            Some(key) => {
                let value = self
                    .entered
                    .get(key)
                    .map(|v| setting_host(v))
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| format!("the address in the setting {key} is empty"))?;
                (value.to_ascii_lowercase(), true)
            }
            None => (
                broker.host.trim_matches(['[', ']']).to_ascii_lowercase(),
                false,
            ),
        };
        let literal = host.parse::<IpAddr>().ok();
        if literal.is_none() && !(self.loopback && host == "localhost") {
            check_host_name(&host).map_err(|why| format!("{host:?}: {why}"))?;
        }
        let reach = match broker.tls {
            TlsMode::TrustDevice | TlsMode::Plain => Reach::Home,
            TlsMode::Verify if entered || literal.is_some() => Reach::Either,
            TlsMode::Verify => Reach::Internet,
        };
        let target = MqttTarget {
            broker: broker.name.clone(),
            host,
            port: broker.port(),
            tls: broker.tls,
            reach,
            loopback: self.loopback,
        };
        if let Some(ip) = literal {
            target.allows(ip)?;
        }
        Ok(target)
    }
}

/// Where a broker's addresses may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// `trust-device` and `none`: the home network only.
    Home,
    /// A host name the manifest declares for `verify`: the internet, as
    /// an `http` host.
    Internet,
    /// An address, or one the household entered, for `verify`: either.
    /// The certificate has to be valid for it.
    Either,
}

/// One broker, ready to connect to.
#[derive(Debug, Clone)]
pub struct MqttTarget {
    /// The manifest's name for it.
    pub broker: String,
    /// A host name or an IP address, lower case, without brackets: what
    /// the certificate is checked for and the pin is kept under.
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    reach: Reach,
    loopback: bool,
}

impl MqttTarget {
    /// The host, if it is an address rather than a name.
    pub fn literal(&self) -> Option<IpAddr> {
        self.host.parse().ok()
    }

    /// The check for each address the host resolves to, or for the
    /// address it is: never loopback or the metadata address; only the
    /// home network for `trust-device` and `none`; and for a host name
    /// declared under `verify`, only the internet, as for `http`.
    pub fn allows(&self, ip: IpAddr) -> Result<(), String> {
        let ip = canonical(ip);
        let host = &self.host;
        if self.loopback && ip.is_loopback() {
            return Ok(());
        }
        if let Some(why) = never(ip) {
            return Err(format!("{host} is {ip}, {why}, which no plugin may reach"));
        }
        match (self.reach, is_local(ip)) {
            (Reach::Home, false) => Err(format!(
                "{host} is {ip}, which is not on the home network, and tls = \"{}\" is only for the home network",
                self.tls.as_str()
            )),
            (Reach::Internet, true) => Err(format!(
                "{host} is {ip}, on the home network, and the manifest names it as a broker on the internet"
            )),
            _ => Ok(()),
        }
    }
}

fn check_host_name(name: &str) -> Result<(), String> {
    let name = name.trim_end_matches('.');
    if name.is_empty() || name.len() > 253 {
        return Err("not a host name".into());
    }
    if name.eq_ignore_ascii_case("localhost") || name.to_ascii_lowercase().ends_with(".localhost") {
        return Err("localhost is never reachable".into());
    }
    for label in name.split('.') {
        let ok = (1..=63).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !ok {
            return Err("not a host name".into());
        }
    }
    Ok(())
}

fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => host
            .strip_suffix(domain)
            .is_some_and(|sub| sub.len() > 1 && sub.ends_with('.')),
        None => pattern == host,
    }
}

/// An IPv4 address in IPv6 clothing is checked as what it is, or
/// `::ffff:127.0.0.1` would walk past every rule about 127.0.0.1.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

/// Why `ip` is never reachable, whatever the manifest says.
fn never(ip: IpAddr) -> Option<&'static str> {
    const METADATA_V4: Ipv4Addr = Ipv4Addr::new(169, 254, 169, 254);
    const METADATA_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254);
    match ip {
        IpAddr::V4(v4) if v4 == METADATA_V4 => Some("the cloud metadata address"),
        IpAddr::V6(v6) if v6 == METADATA_V6 => Some("the cloud metadata address"),
        IpAddr::V4(v4) if v4.is_loopback() => Some("loopback"),
        IpAddr::V6(v6) if v6.is_loopback() => Some("loopback"),
        IpAddr::V4(v4) if v4.octets()[0] == 0 || v4.is_unspecified() => Some("unspecified"),
        IpAddr::V6(v6) if v6.is_unspecified() => Some("unspecified"),
        IpAddr::V4(v4) if v4.is_multicast() || v4.is_broadcast() => Some("a broadcast address"),
        IpAddr::V6(v6) if v6.is_multicast() => Some("a broadcast address"),
        IpAddr::V4(v4) if v4.octets()[0] >= 240 => Some("reserved"),
        _ => None,
    }
}

/// Private, link-local or unique-local: the home network's.
fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

fn in_net(ip: IpAddr, base: IpAddr, prefix: u8) -> bool {
    match (ip, canonical(base)) {
        (IpAddr::V4(ip), IpAddr::V4(base)) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            u32::from(ip) & mask == u32::from(base) & mask
        }
        (IpAddr::V6(ip), IpAddr::V6(base)) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            u128::from(ip) & mask == u128::from(base) & mask
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(http: &[&str], local: &[&str], address: Option<&str>) -> HttpPolicy {
        let permissions = Permissions {
            http: http.iter().map(|s| s.to_string()).collect(),
            local_network: local.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        HttpPolicy::new(&permissions, |key| {
            (key == "address")
                .then(|| address.map(str::to_string))
                .flatten()
        })
    }

    fn url(s: &str) -> reqwest::Url {
        s.parse().unwrap()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn an_exact_host_is_admitted_and_others_are_not() {
        let p = policy(&["api.example.com"], &[], None);
        assert!(p.admit(&url("https://api.example.com/v1")).is_ok());
        assert!(p.admit(&url("https://API.Example.com./v1")).is_ok());
        assert!(p.admit(&url("https://example.com/")).is_err());
        assert!(p.admit(&url("https://evil.com/?api.example.com")).is_err());
        assert!(p.admit(&url("https://api.example.com.evil.com/")).is_err());
    }

    #[test]
    fn a_wildcard_covers_subdomains_but_not_the_domain() {
        let p = policy(&["*.tibber.com"], &[], None);
        assert!(p.admit(&url("https://api.tibber.com/")).is_ok());
        assert!(p.admit(&url("https://a.b.tibber.com/")).is_ok());
        assert!(p.admit(&url("https://tibber.com/")).is_err());
        assert!(p.admit(&url("https://eviltibber.com/")).is_err());
    }

    #[test]
    fn the_internet_needs_https() {
        let p = policy(&["api.example.com"], &[], None);
        assert!(p.admit(&url("http://api.example.com/")).is_err());
        assert!(p.admit(&url("ftp://api.example.com/")).is_err());
    }

    #[test]
    fn a_declared_host_that_resolves_to_the_house_is_refused() {
        // DNS rebinding, or just a hostile record: the name is fine, the
        // address is not.
        let p = policy(&["api.example.com"], &[], None);
        let ok = p.admit(&url("https://api.example.com/")).unwrap();
        assert!(p.allows(&ok, ip("93.184.216.34")).is_ok());
        assert!(p.allows(&ok, ip("192.168.1.10")).is_err());
        assert!(p.allows(&ok, ip("10.0.0.1")).is_err());
        assert!(p.allows(&ok, ip("fd12::1")).is_err());
        assert!(p.allows(&ok, ip("fe80::1")).is_err());
    }

    #[test]
    fn loopback_and_metadata_are_never_reachable() {
        let p = policy(
            &["api.example.com"],
            &["169.254.169.0/24"],
            Some("127.0.0.1"),
        );
        let ok = p.admit(&url("https://api.example.com/")).unwrap();
        for never in [
            "127.0.0.1",
            "127.1.2.3",
            "::1",
            "::ffff:127.0.0.1",
            "169.254.169.254",
            "0.0.0.0",
            "::",
            "224.0.0.1",
        ] {
            assert!(p.allows(&ok, ip(never)).is_err(), "{never}");
        }
        // Not even as a literal, nor through a setting that names it.
        assert!(p.admit(&url("http://127.0.0.1/")).is_err());
        assert!(
            p.admit(&url("http://169.254.169.254/latest/meta-data"))
                .is_err()
        );
    }

    #[test]
    fn a_mapped_ipv4_address_is_checked_as_ipv4() {
        let p = policy(&[], &["192.168.1.50"], None);
        let ok = p.admit(&url("http://[::ffff:192.168.1.50]/")).unwrap();
        assert!(p.allows(&ok, ip("::ffff:192.168.1.50")).is_ok());
        assert!(p.allows(&ok, ip("::ffff:192.168.1.51")).is_err());
    }

    #[test]
    fn websocket_targets_follow_public_and_local_network_boundaries() {
        for denied in [
            "127.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "169.254.169.254",
            "fd00:ec2::254",
        ] {
            assert!(
                ws_address_allowed(ip(denied), false, false).is_err(),
                "{denied}"
            );
            assert!(
                ws_address_allowed(ip(denied), true, false).is_err(),
                "{denied}"
            );
        }
        assert!(ws_address_allowed(ip("93.184.216.34"), false, false).is_ok());
        assert!(ws_address_allowed(ip("192.168.1.50"), false, false).is_err());
        assert!(ws_address_allowed(ip("192.168.1.50"), true, false).is_ok());
        assert!(ws_address_allowed(ip("93.184.216.34"), true, false).is_err());
        assert!(ws_address_allowed(ip("127.0.0.1"), true, true).is_ok());
        assert!(ws_address_allowed(ip("169.254.169.254"), true, true).is_err());
    }

    #[test]
    fn a_setting_names_the_device_and_nothing_else() {
        let p = policy(&[], &["setting:address"], Some("192.168.1.50"));
        let ok = p.admit(&url("http://192.168.1.50/status")).unwrap();
        assert!(p.allows(&ok, ip("192.168.1.50")).is_ok());
        assert!(p.admit(&url("http://192.168.1.51/")).is_err());
    }

    #[test]
    fn a_setting_may_carry_a_scheme_or_a_port() {
        for typed in [
            "http://192.168.1.50:8080/",
            "192.168.1.50:80",
            "192.168.1.50",
        ] {
            let p = policy(&[], &["setting:address"], Some(typed));
            assert!(
                p.admit(&url("http://192.168.1.50:8080/")).is_ok(),
                "{typed}"
            );
        }
        let p = policy(&[], &["setting:address"], Some("inverter.lan"));
        let ok = p.admit(&url("http://inverter.lan/")).unwrap();
        assert!(p.allows(&ok, ip("192.168.1.77")).is_ok());
        assert!(
            p.allows(&ok, ip("93.184.216.34")).is_err(),
            "a local name that points at the internet is not local"
        );
    }

    #[test]
    fn an_empty_setting_reaches_nothing() {
        let p = policy(&[], &["setting:address"], None);
        assert!(p.is_empty());
        assert!(p.admit(&url("http://192.168.1.50/")).is_err());
    }

    #[test]
    fn a_range_covers_its_addresses() {
        let p = policy(&[], &["192.168.20.0/24"], None);
        let ok = p.admit(&url("http://192.168.20.7/")).unwrap();
        assert!(p.allows(&ok, ip("192.168.20.7")).is_ok());
        assert!(p.admit(&url("http://192.168.21.7/")).is_err());
    }

    #[test]
    fn manifest_entries_are_checked() {
        assert!(check_host_pattern("api.example.com").is_ok());
        assert!(check_host_pattern("*.example.com").is_ok());
        assert!(check_host_pattern("https://api.example.com").is_err());
        assert!(check_host_pattern("api.example.com:8443").is_err());
        assert!(check_host_pattern("api.*.com").is_err());
        assert!(check_host_pattern("*.com").is_err());
        assert!(check_host_pattern("1.2.3.4").is_err());
        assert!(check_host_pattern("localhost").is_err());

        assert!(check_local_entry("192.168.1.50").is_ok());
        assert!(check_local_entry("inverter.lan").is_ok());
        assert!(check_local_entry("192.168.20.0/24").is_ok());
        assert!(
            check_local_entry("192.168.0.0/16").is_err(),
            "wider than a /24"
        );
        assert!(check_local_entry("127.0.0.1").is_err());
        assert!(check_local_entry("169.254.169.254").is_err());
        assert!(
            check_local_entry("8.8.8.8").is_err(),
            "not the home network"
        );
        assert!(check_local_entry("8.8.8.0/24").is_err());
    }

    fn broker(host: &str, tls: TlsMode) -> MqttPolicy {
        let permissions = Permissions {
            mqtt: vec![MqttBroker {
                name: "b".into(),
                host: host.into(),
                port: None,
                tls,
            }],
            ..Default::default()
        };
        MqttPolicy::new(&permissions, |key| {
            (key == "address").then(|| "http://192.168.1.40:8883/".to_string())
        })
    }

    fn target(policy: &MqttPolicy) -> MqttTarget {
        policy.target("b").unwrap().unwrap()
    }

    #[test]
    fn a_broker_named_for_verify_is_on_the_internet_as_an_http_host_is() {
        let t = target(&broker("broker.example.com", TlsMode::Verify));
        assert_eq!(t.port, 8883);
        assert!(t.allows(ip("93.184.216.34")).is_ok());
        assert!(
            t.allows(ip("192.168.1.10")).is_err(),
            "rebound to the house"
        );
        assert!(t.allows(ip("127.0.0.1")).is_err());
        assert!(t.allows(ip("169.254.169.254")).is_err());
        assert!(
            broker("broker.example.com", TlsMode::Verify)
                .target("other")
                .is_none()
        );
    }

    #[test]
    fn trust_device_and_plain_brokers_are_on_the_home_network() {
        for tls in [TlsMode::TrustDevice, TlsMode::Plain] {
            let t = target(&broker("setting:address", tls));
            assert_eq!(
                t.host, "192.168.1.40",
                "the scheme, port and path are dropped"
            );
            assert_eq!(t.literal(), Some(ip("192.168.1.40")));
            let named = target(&broker("printer.lan", tls));
            assert!(named.allows(ip("192.168.1.40")).is_ok());
            assert!(named.allows(ip("fd12::40")).is_ok());
            assert!(named.allows(ip("93.184.216.34")).is_err());
            assert!(named.allows(ip("::ffff:127.0.0.1")).is_err());
        }
    }

    #[test]
    fn an_address_the_household_enters_for_verify_may_be_anywhere() {
        let t = target(&broker("setting:address", TlsMode::Verify));
        assert!(t.allows(ip("192.168.1.40")).is_ok());
        assert!(t.allows(ip("93.184.216.34")).is_ok());
        assert!(t.allows(ip("127.0.0.1")).is_err());
    }

    #[test]
    fn an_empty_or_unreachable_setting_is_said_at_connect() {
        let permissions = Permissions {
            mqtt: vec![MqttBroker {
                name: "b".into(),
                host: "setting:address".into(),
                port: None,
                tls: TlsMode::TrustDevice,
            }],
            ..Default::default()
        };
        let empty = MqttPolicy::new(&permissions, |_| None);
        assert!(empty.target("b").unwrap().unwrap_err().contains("empty"));
        let public = MqttPolicy::new(&permissions, |_| Some("8.8.8.8".into()));
        assert!(
            public
                .target("b")
                .unwrap()
                .unwrap_err()
                .contains("home network")
        );
        let loopback = MqttPolicy::new(&permissions, |_| Some("127.0.0.1".into()));
        assert!(loopback.target("b").unwrap().is_err());
        assert!(
            loopback.allowing_loopback().target("b").unwrap().is_ok(),
            "only the dev override lets it through"
        );
        let localhost = MqttPolicy::new(&permissions, |_| Some("localhost".into()));
        assert!(localhost.target("b").unwrap().is_err());
        assert!(localhost.allowing_loopback().target("b").unwrap().is_ok());
    }

    #[test]
    fn a_setting_that_names_this_machine_is_reached_only_when_dev_allows_it() {
        for address in ["127.0.0.1:8080", "localhost", "[::1]:8080"] {
            let p = policy(&[], &["setting:address"], Some(address));
            assert!(
                p.admit(&url("http://127.0.0.1:8080/")).is_err(),
                "{address}"
            );
            assert!(
                p.admit(&url("http://localhost:8080/")).is_err(),
                "{address}"
            );

            let dev = p.allowing_loopback();
            let ok = dev.admit(&url("http://127.0.0.1:8080/status")).unwrap();
            assert!(dev.allows(&ok, ip("127.0.0.1")).is_ok(), "{address}");
            let ok = dev.admit(&url("http://localhost:8080/status")).unwrap();
            assert!(dev.allows(&ok, ip("::1")).is_ok(), "{address}");
            assert!(dev.admit(&url("http://192.168.1.51/")).is_err());
        }
    }

    #[test]
    fn dev_loopback_reaches_nothing_the_manifest_does_not_point_at() {
        // A plugin with only internet hosts, or a setting naming a real
        // device, gains nothing on this machine from the dev flag.
        let internet = policy(&["api.example.com"], &[], None).allowing_loopback();
        assert!(internet.admit(&url("http://127.0.0.1/")).is_err());
        let ok = internet.admit(&url("https://api.example.com/")).unwrap();
        assert!(internet.allows(&ok, ip("127.0.0.1")).is_err());

        let device = policy(&[], &["setting:address"], Some("192.168.1.50")).allowing_loopback();
        assert!(device.admit(&url("http://127.0.0.1/")).is_err());
    }

    #[test]
    fn mqtt_hosts_in_the_manifest_are_checked() {
        assert!(check_mqtt_host("broker.example.com", TlsMode::Verify).is_ok());
        assert!(check_mqtt_host("192.168.1.40", TlsMode::TrustDevice).is_ok());
        assert!(check_mqtt_host("[fd12::40]", TlsMode::Plain).is_ok());
        assert!(check_mqtt_host("8.8.8.8", TlsMode::TrustDevice).is_err());
        assert!(check_mqtt_host("169.254.169.254", TlsMode::Verify).is_err());
        assert!(check_mqtt_host("mqtt://x.example.com", TlsMode::Verify).is_err());
    }

    #[test]
    fn page_images_come_from_http_hosts_only() {
        let p = policy(&["*.example.net"], &["192.168.1.50"], None);
        assert!(p.declares_host("img.example.net"));
        assert!(!p.declares_host("192.168.1.50"));
    }
}
