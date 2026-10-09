//! What Niles asks of an LG TV.

use crate::error::Result;
use crate::session::Session;
use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;

/// How long the pairing prompt waits for somebody with the remote.
pub const PAIRING: Duration = Duration::from_secs(60);

/// An app the TV can open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct App {
    pub id: String,
    pub title: String,
}

/// An HDMI or other input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Input {
    pub id: String,
    /// What the TV shows for it: "HDMI 2", or a name somebody gave it.
    pub label: String,
}

/// Whether the TV is on, and what is on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub on: bool,
    /// The app's title — "Netflix" — or its id when the TV has no title
    /// for it, as with an input.
    pub app: Option<String>,
}

/// An LG webOS TV at a known address, paired with `client_key`.
#[derive(Debug, Clone)]
pub struct Tv {
    host: String,
    client_key: String,
}

/// Pair with the TV at `host`: it shows a prompt, somebody accepts it
/// with the remote, and the key it issues is what every later
/// connection registers with.
pub async fn pair(host: &str) -> Result<String> {
    let (session, key) = Session::open(host, None, PAIRING).await?;
    session.close().await;
    Ok(key)
}

impl Tv {
    pub fn new(host: impl Into<String>, client_key: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            client_key: client_key.into(),
        }
    }

    async fn ask(&self, uri: &str, payload: Value) -> Result<Value> {
        let (mut session, _) =
            Session::open(&self.host, Some(&self.client_key), Duration::ZERO).await?;
        let answer = session.request(uri, payload).await;
        session.close().await;
        answer
    }

    /// Off to standby.
    pub async fn turn_off(&self) -> Result<()> {
        self.ask("system/turnOff", json!({})).await.map(drop)
    }

    /// The app on screen: its id, "com.webos.app.hdmi2" for an input.
    pub async fn foreground_app(&self) -> Result<Option<String>> {
        let answer = self
            .ask(
                "com.webos.applicationManager/getForegroundAppInfo",
                json!({}),
            )
            .await?;
        Ok(answer["appId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_string))
    }

    /// Whether the screen is on. A TV in standby with Quick Start+ still
    /// answers, so answering is not the same as being on.
    pub async fn screen_on(&self) -> Result<bool> {
        let answer = self
            .ask("com.webos.service.tvpower/power/getPowerState", json!({}))
            .await?;
        Ok(screen_on(&answer))
    }

    pub async fn apps(&self) -> Result<Vec<App>> {
        let answer = self
            .ask("com.webos.applicationManager/listLaunchPoints", json!({}))
            .await?;
        Ok(answer["launchPoints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                Some(App {
                    id: p["id"].as_str()?.to_string(),
                    title: p["title"].as_str()?.to_string(),
                })
            })
            .collect())
    }

    pub async fn launch(&self, app_id: &str) -> Result<()> {
        self.ask("system.launcher/launch", json!({ "id": app_id }))
            .await
            .map(drop)
    }

    pub async fn inputs(&self) -> Result<Vec<Input>> {
        let answer = self.ask("tv/getExternalInputList", json!({})).await?;
        Ok(answer["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| {
                Some(Input {
                    id: d["id"].as_str()?.to_string(),
                    label: d["label"].as_str()?.to_string(),
                })
            })
            .collect())
    }

    pub async fn switch_input(&self, input_id: &str) -> Result<()> {
        self.ask("tv/switchInput", json!({ "inputId": input_id }))
            .await
            .map(drop)
    }

    pub async fn play(&self) -> Result<()> {
        self.ask("media.controls/play", json!({})).await.map(drop)
    }

    pub async fn pause(&self) -> Result<()> {
        self.ask("media.controls/pause", json!({})).await.map(drop)
    }

    /// A message in the corner of the screen, over whatever is on.
    pub async fn show(&self, message: &str) -> Result<()> {
        self.ask(
            "system.notifications/createToast",
            json!({ "message": message }),
        )
        .await
        .map(drop)
    }

    /// On or off, and what is on screen. Nothing answering is a TV that
    /// is off, not an error: that is most of the day.
    pub async fn status(&self) -> Result<Status> {
        let on = match self.screen_on().await {
            Ok(on) => on,
            Err(crate::Error::Unreachable { .. }) => false,
            Err(e) => return Err(e),
        };
        if !on {
            return Ok(Status { on, app: None });
        }
        let app = self.foreground_app().await.ok().flatten();
        let title = match &app {
            Some(id) => self
                .apps()
                .await
                .ok()
                .and_then(|apps| apps.into_iter().find(|a| &a.id == id))
                .map(|a| a.title),
            None => None,
        };
        Ok(Status {
            on,
            app: title.or(app),
        })
    }

    /// The TV's own MAC address, wired if it has one in use, for waking it.
    pub async fn mac_address(&self) -> Result<Option<String>> {
        let answer = self
            .ask("com.webos.service.connectionmanager/getinfo", json!({}))
            .await?;
        Ok(mac_in(&answer))
    }
}

fn screen_on(answer: &Value) -> bool {
    // "Active", or "Screen Off" with sound; "Active Standby" and
    // "Suspend" are the TV asleep but listening.
    matches!(
        answer["state"].as_str(),
        Some("Active") | Some("Screen Saver")
    )
}

fn mac_in(answer: &Value) -> Option<String> {
    ["wiredInfo", "wifiInfo"]
        .iter()
        .filter_map(|kind| answer[kind]["macAddress"].as_str())
        .find(|mac| !mac.is_empty() && *mac != "00:00:00:00:00:00")
        .map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standby_is_not_on() {
        assert!(screen_on(&json!({ "state": "Active" })));
        assert!(!screen_on(&json!({ "state": "Active Standby" })));
        assert!(!screen_on(&json!({ "returnValue": true })));
    }

    #[test]
    fn the_mac_in_use_is_the_one_to_wake() {
        let answer = json!({
            "wiredInfo": { "macAddress": "00:00:00:00:00:00" },
            "wifiInfo": { "macAddress": "A8:23:FE:01:02:03" }
        });
        assert_eq!(mac_in(&answer).as_deref(), Some("a8:23:fe:01:02:03"));
        assert_eq!(mac_in(&json!({})), None);
    }
}
