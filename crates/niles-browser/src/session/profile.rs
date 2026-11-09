//! A login's browser profile: its cookies and storage, opaque to
//! Niles, sealed by the caller. What is kept, what is restored, and the
//! caps that bound memory are here; see ARCHITECTURE.md's "Browser
//! profiles".

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::error::{Error, Result};
use crate::session::Session;
use crate::tab::Tab;

/// The profile's format version; another one isn't restored.
const VERSION: u32 = 1;
/// At most this many cookies are kept.
const MAX_COOKIES: usize = 300;
/// At most this much serialized storage per origin is kept.
const MAX_STORAGE: usize = 512 * 1024;
/// At most this much serialized profile in all is kept.
const MAX_PROFILE: usize = 2 * 1024 * 1024;

/// A login's cookies and storage as sealed versioned JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Profile {
    v: u32,
    cookies: Vec<Cookie>,
    storage: Vec<Stored>,
}

/// One cookie, mapped from CDP's `Storage.getCookies` answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    #[serde(default)]
    expires: Option<f64>,
    #[serde(default)]
    http_only: bool,
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    same_site: Option<String>,
}

/// The localStorage of one of the origins' frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    origin: String,
    items: Vec<(String, String)>,
}

/// Whether `domain` — a cookie domain, with or without its leading
/// dot — is one of `origins`' hosts or a parent of one.
fn domain_matches(domain: &str, origins: &[String]) -> bool {
    let wanted = domain.trim_start_matches('.');
    origins.iter().any(|origin| {
        let Ok(url) = Url::parse(origin) else {
            return false;
        };
        let Some(host) = url.host_str() else {
            return false;
        };
        host == wanted || host.ends_with(&format!(".{wanted}"))
    })
}

/// One raw CDP cookie as a kept one: `None` for a partitioned, expired
/// or other-host cookie. A session cookie answers with `expires: None`.
fn kept(raw: &Value, origins: &[String], now_secs: f64) -> Option<Cookie> {
    if raw.get("partitionKey").is_some() {
        return None;
    }
    let domain = raw.get("domain").and_then(Value::as_str)?;
    if !domain_matches(domain, origins) {
        return None;
    }
    let expires = if raw.get("session").and_then(Value::as_bool) == Some(true) {
        None
    } else {
        let at = raw.get("expires").and_then(Value::as_f64);
        if at.is_some_and(|at| at <= now_secs) {
            return None;
        }
        at
    };
    Some(Cookie {
        name: raw.get("name").and_then(Value::as_str)?.to_string(),
        value: raw.get("value").and_then(Value::as_str)?.to_string(),
        domain: domain.to_string(),
        path: raw
            .get("path")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .unwrap_or("/")
            .to_string(),
        expires,
        http_only: raw
            .get("httpOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        secure: raw.get("secure").and_then(Value::as_bool).unwrap_or(false),
        same_site: raw
            .get("sameSite")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// A saved profile, or the fixed reason one wouldn't read. The wrong
/// version is never restored: the shapes may differ.
fn parse(profile: &str) -> Result<Profile> {
    match serde_json::from_str::<Profile>(profile) {
        Ok(saved) if saved.v == VERSION => Ok(saved),
        _ => Err(Error::Answer {
            method: "profile",
            reason: "the saved profile didn't read".into(),
        }),
    }
}

impl Session {
    /// The cookies of `origins`' hosts and the storage of their frames,
    /// as opaque JSON. Errors carry no values.
    pub(super) async fn profile_in(&self, origins: &[String]) -> Result<String> {
        let answer = self
            .conn
            .call(
                "Storage.getCookies",
                json!({"browserContextId": self.context}),
                None,
            )
            .await
            .map_err(|_| Error::Answer {
                method: "Storage.getCookies",
                reason: "the cookies couldn't be read".into(),
            })?;
        let raw = answer
            .get("cookies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let cookies: Vec<Cookie> = raw
            .iter()
            .filter_map(|c| kept(c, origins, now))
            .take(MAX_COOKIES)
            .collect();
        let mut storage: Vec<Stored> = Vec::new();
        if let Some(frames) = self.on_frames(origins).await? {
            for (_, context) in frames {
                let answer = match self.call_in(context, "storage", vec![json!(origins)]).await {
                    Ok(answer) => answer,
                    Err(crate::error::Error::Protocol { .. }) => continue,
                    Err(e) => return Err(e),
                };
                let Ok(stored) = serde_json::from_value::<Stored>(answer) else {
                    continue;
                };
                if storage.iter().any(|s| s.origin == stored.origin) {
                    continue;
                }
                let size = serde_json::to_string(&stored).map_or(usize::MAX, |s| s.len());
                if size > MAX_STORAGE {
                    continue;
                }
                storage.push(stored);
            }
        }
        let mut profile = Profile {
            v: VERSION,
            cookies,
            storage,
        };
        let mut text = serde_json::to_string(&profile).map_err(|_| Error::Answer {
            method: "profile",
            reason: "the profile couldn't be written".into(),
        })?;
        if text.len() > MAX_PROFILE {
            profile.storage = Vec::new();
            text = serde_json::to_string(&profile).map_err(|_| Error::Answer {
                method: "profile",
                reason: "the profile couldn't be written".into(),
            })?;
        }
        Ok(text)
    }

    /// Put a profile back — cookies for every login host, storage per
    /// frame of its origin — then open the page again with a GET,
    /// never re-POSTing.
    pub(super) async fn restore_in(&self, origins: &[String], profile: &str) -> Result<()> {
        let saved = parse(profile)?;
        let cookies: Vec<Value> = saved
            .cookies
            .iter()
            .filter(|c| domain_matches(&c.domain, origins))
            .map(|c| {
                let mut param = json!({
                    "name": c.name,
                    "value": c.value,
                    "path": c.path,
                    "secure": c.secure,
                    "httpOnly": c.http_only
                });
                if c.domain.starts_with('.') {
                    param["domain"] = json!(c.domain);
                } else {
                    param["url"] = json!(format!("https://{}{}", c.domain, c.path));
                }
                if let Some(same_site) = &c.same_site {
                    param["sameSite"] = json!(same_site);
                }
                if let Some(expires) = c.expires {
                    param["expires"] = json!(expires);
                }
                param
            })
            .collect();
        self.conn
            .call(
                "Storage.setCookies",
                json!({"cookies": cookies, "browserContextId": self.context}),
                None,
            )
            .await?;
        for frame in self.frames().await? {
            if !origins.contains(&frame.origin) {
                continue;
            }
            let Some(stored) = saved.storage.iter().find(|s| s.origin == frame.origin) else {
                continue;
            };
            let Some(context) = self.world_for(&frame).await? else {
                continue;
            };
            // A frame that fails (gone, or in another process) is
            // skipped: the rest of the profile still lands.
            let _ = self
                .call_in(
                    context,
                    "restore",
                    vec![json!(origins), json!(stored.items)],
                )
                .await;
        }
        // Open the page again with a GET, never a reload: a POST's
        // result must not be re-POSTed.
        let address = self.address().await?;
        let url = Url::parse(&address).map_err(|_| Error::Navigation {
            reason: "the page's address didn't read".into(),
        })?;
        self.open(&url).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn domain_matches_the_origins_hosts_and_their_parents() {
        let origins = vec!["https://shop.example".to_string()];
        assert!(domain_matches(".shop.example", &origins));
        assert!(domain_matches("shop.example", &origins));
        assert!(!domain_matches("evil-shop.example", &origins));
        assert!(!domain_matches("other.example", &origins));
        let www = vec!["https://www.shop.example".to_string()];
        assert!(domain_matches(".shop.example", &www));
        assert!(domain_matches("shop.example", &www));
        assert!(!domain_matches("notshop.example", &www));
    }

    #[test]
    fn kept_drops_partitioned_expired_and_other_host_cookies() {
        let origins = vec!["https://shop.example".to_string()];
        let now = 1_000_000.0;
        let session = json!({
            "name": "sid", "value": "abc", "domain": "shop.example",
            "path": "/", "session": true, "secure": true
        });
        let kept_session = kept(&session, &origins, now).expect("the session cookie");
        assert_eq!(kept_session.name, "sid");
        assert_eq!(kept_session.expires, None);
        let partitioned = json!({
            "name": "sid", "value": "abc", "domain": "shop.example",
            "path": "/", "partitionKey": {"topLevelSite": "https://shop.example"}
        });
        assert!(kept(&partitioned, &origins, now).is_none());
        let expired = json!({
            "name": "old", "value": "abc", "domain": "shop.example",
            "path": "/", "expires": now - 1.0
        });
        assert!(kept(&expired, &origins, now).is_none());
        let elsewhere = json!({
            "name": "tracker", "value": "abc", "domain": "other.example",
            "path": "/", "session": true
        });
        assert!(kept(&elsewhere, &origins, now).is_none());
    }

    #[test]
    fn a_profile_round_trips_as_json() {
        let profile = Profile {
            v: VERSION,
            cookies: vec![Cookie {
                name: "sid".into(),
                value: "abc".into(),
                domain: ".shop.example".into(),
                path: "/".into(),
                expires: None,
                http_only: true,
                secure: true,
                same_site: Some("Lax".into()),
            }],
            storage: vec![Stored {
                origin: "https://shop.example".into(),
                items: vec![("k".into(), "v".into())],
            }],
        };
        let text = serde_json::to_string(&profile).expect("serialize");
        let back = parse(&text).expect("parse");
        assert_eq!(back.cookies.len(), 1);
        assert_eq!(back.cookies[0].name, "sid");
        assert_eq!(back.storage.len(), 1);
        assert_eq!(back.storage[0].items, vec![("k".into(), "v".into())]);
    }

    #[test]
    fn a_wrong_version_isnt_restored() {
        let text = serde_json::to_string(&Profile {
            v: VERSION + 1,
            cookies: Vec::new(),
            storage: Vec::new(),
        })
        .expect("serialize");
        assert!(parse(&text).is_err());
        assert!(parse("not json").is_err());
    }
}
