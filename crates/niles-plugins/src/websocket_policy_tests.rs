use super::*;

fn manifest(grant: &str, setting: &str) -> crate::manifest::Manifest {
    let text = format!(
        r#"
id = "feed"
name = "Feed"
version = "0.1.0"
description = "x"
authors = ["a"]
min_niles = "0.4.0"
interface = "0.4"
[permissions]
websocket = [{grant}]
local_network = ["192.168.1.0/24", "printer.lan", "fd00::1"]
{setting}
"#
    );
    crate::manifest::Manifest::parse(&text).unwrap()
}

fn policy(
    manifest: &crate::manifest::Manifest,
    values: &[(&str, &str)],
    loopback: bool,
) -> WsPolicy {
    let settings = values
        .iter()
        .map(|(k, v)| ((*k).to_owned(), toml::Value::String((*v).to_owned())))
        .collect();
    WsPolicy::new(manifest, &settings, loopback)
}

#[test]
fn settings_and_manifest_resolve_consistent_grant_boundaries() {
    let parsed = manifest(
        r#"{ name = "feed", host = "stream.example.com", tls = true }"#,
        "",
    );
    assert!(parsed.problems().is_empty());
    let target = policy(&parsed, &[], false).target("feed").unwrap().unwrap();
    assert!(target.allows("93.184.216.34".parse().unwrap()).is_ok());
    assert!(target.allows("192.168.1.10".parse().unwrap()).is_err());
    assert!(target.allows("127.0.0.1".parse().unwrap()).is_err());

    let plain = manifest(r#"{ name = "feed", host = "printer.lan" }"#, "");
    assert!(plain.problems().is_empty());
    let target = policy(&plain, &[], false).target("feed").unwrap().unwrap();
    assert!(target.allows("192.168.1.10".parse().unwrap()).is_ok());
    assert!(target.allows("93.184.216.34".parse().unwrap()).is_err());

    let entered = manifest(
        r#"{ name = "feed", host = "setting:address", tls = true }"#,
        "[[settings]]\nkey = \"address\"\nlabel = \"Address\"\ntype = \"string\"",
    );
    assert!(entered.problems().is_empty());
    let target = policy(&entered, &[("address", "  STREAM.EXAMPLE.COM.  ")], false)
        .target("feed")
        .unwrap()
        .unwrap();
    assert_eq!(target.host, "stream.example.com");
    assert!(target.allows("93.184.216.34".parse().unwrap()).is_ok());
    assert!(target.allows("192.168.1.10".parse().unwrap()).is_ok());

    let missing = policy(&entered, &[], false).target("feed").unwrap();
    assert!(missing.is_err());
}

#[test]
fn public_plaintext_is_rejected_and_explicit_public_wss_is_allowed() {
    let plain = manifest(r#"{ name = "feed", host = "stream.example.com" }"#, "");
    assert!(!plain.problems().is_empty());
    let secure = manifest(
        r#"{ name = "feed", host = "stream.example.com", tls = true }"#,
        "",
    );
    assert!(secure.problems().is_empty());
    let target = policy(&secure, &[], false).target("feed").unwrap().unwrap();
    assert!(check_url("wss://STREAM.EXAMPLE.COM./path", &target).is_ok());
}

#[test]
fn untouched_default_uses_normal_setting_resolution() {
    let parsed = manifest(
        r#"{ name = "feed", host = "setting:address", tls = true }"#,
        "[[settings]]\nkey = \"address\"\nlabel = \"Address\"\ntype = \"string\"\ndefault = \"wss://STREAM.EXAMPLE.COM:443/events\"",
    );
    let target = policy(&parsed, &[], false).target("feed").unwrap().unwrap();
    assert_eq!(target.host, "stream.example.com");
    assert!(check_url("wss://stream.example.com/events", &target).is_ok());
    assert!(target.allows("93.184.216.34".parse().unwrap()).is_ok());
}

#[test]
fn ipv6_literal_normalizes_through_manifest_policy_and_url_check() {
    let m = manifest(r#"{ name = "feed", host = "[fd00::1]" }"#, "");
    assert!(m.problems().is_empty(), "{:?}", m.problems());
    let target = policy(&m, &[], false).target("feed").unwrap().unwrap();
    assert_eq!(target.host, "fd00::1");
    assert!(target.allows("fd00::1".parse().unwrap()).is_ok());
    assert!(check_url("ws://[fd00::1]/socket", &target).is_ok());
    assert!(check_url("ws://[fd00::2]/socket", &target).is_err());
}
