//! What Niles asks of an LG TV.

use crate::error::{Error, Result};
use crate::session::{PORT, Session};
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
    /// The app the TV says is on screen while it shows this input:
    /// "com.webos.app.hdmi2".
    pub app_id: Option<String>,
}

/// Whether the TV is on, and what is on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub on: bool,
    /// The app's title — "Netflix" — or its id when the TV has no title
    /// for it, as with an input.
    pub app: Option<String>,
    /// The TV's own volume, while it is on and says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
}

/// How loud the TV's own sound is, `0..=100`, and whether it is muted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Volume {
    pub volume: u8,
    pub muted: bool,
}

/// Something to do to the TV's sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeChange {
    /// To a level, `0..=100`.
    To(u8),
    /// Up or down from where it is.
    By(i16),
    Mute(bool),
}

const POWER_STATE: &str = "com.webos.service.tvpower/power/getPowerState";
const FOREGROUND_APP: &str = "com.webos.applicationManager/getForegroundAppInfo";
const GET_VOLUME: &str = "audio/getVolume";
const SET_VOLUME: &str = "audio/setVolume";
const SET_MUTE: &str = "audio/setMute";

/// An LG webOS TV at a known address, paired with `client_key`.
#[derive(Debug, Clone)]
pub struct Tv {
    host: String,
    port: u16,
    client_key: String,
}

/// Pair with the TV at `host`: it shows a prompt, somebody accepts it
/// with the remote, and the key it issues is what every later
/// connection registers with.
pub async fn pair(host: &str) -> Result<String> {
    let (session, key) = Session::open(host, PORT, None, PAIRING).await?;
    session.close().await;
    Ok(key)
}

impl Tv {
    pub fn new(host: impl Into<String>, client_key: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: PORT,
            client_key: client_key.into(),
        }
    }

    async fn connect(&self) -> Result<Session> {
        let (session, _) = Session::open(
            &self.host,
            self.port,
            Some(&self.client_key),
            Duration::ZERO,
        )
        .await?;
        Ok(session)
    }

    async fn ask(&self, uri: &str, payload: Value) -> Result<Value> {
        let mut session = self.connect().await?;
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
        let answer = self.ask(FOREGROUND_APP, json!({})).await?;
        Ok(app_in(&answer))
    }

    /// Whether the screen is on. A TV in standby with Quick Start+ still
    /// answers, so answering is not the same as being on.
    pub async fn screen_on(&self) -> Result<bool> {
        let mut session = self.connect().await?;
        let state = power_state(&mut session).await;
        session.close().await;
        Ok(screen_on(&state?))
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
                    app_id: d["appId"].as_str().map(str::to_string),
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

    /// How loud, and whether muted.
    pub async fn volume(&self) -> Result<Volume> {
        volume_in(&self.ask(GET_VOLUME, json!({})).await?)
    }

    pub async fn set_volume(&self, level: u8) -> Result<()> {
        self.ask(SET_VOLUME, volume_payload(level)).await.map(drop)
    }

    /// One notch up, as the remote's button does.
    pub async fn volume_up(&self) -> Result<()> {
        self.ask("audio/volumeUp", json!({})).await.map(drop)
    }

    /// One notch down, as the remote's button does.
    pub async fn volume_down(&self) -> Result<()> {
        self.ask("audio/volumeDown", json!({})).await.map(drop)
    }

    pub async fn set_mute(&self, mute: bool) -> Result<()> {
        self.ask(SET_MUTE, json!({ "mute": mute })).await.map(drop)
    }

    /// Change the TV's sound and say what it is after, over one
    /// connection: a voice command waits on every handshake, and an
    /// older TV is slow to make one. `None` when the TV is off — nothing
    /// answering, or asleep and listening.
    pub async fn change_volume(&self, change: VolumeChange) -> Result<Option<Volume>> {
        let mut session = match self.connect().await {
            Ok(session) => session,
            Err(Error::Unreachable { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let changed = changed(&mut session, change).await;
        session.close().await;
        changed
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

    /// On or off, what is on screen, and how loud. Nothing answering is
    /// a TV that is off, not an error: that is most of the day.
    pub async fn status(&self) -> Result<Status> {
        let on = match self.screen_on().await {
            Ok(on) => on,
            Err(crate::Error::Unreachable { .. }) => false,
            Err(e) => return Err(e),
        };
        if !on {
            return Ok(Status {
                on,
                app: None,
                volume: None,
                muted: None,
            });
        }
        let sound = self.volume().await.ok();
        let nothing_on_screen = Status {
            on,
            app: None,
            volume: sound.map(|s| s.volume),
            muted: sound.map(|s| s.muted),
        };
        let Some(app) = self.foreground_app().await.ok().flatten() else {
            return Ok(nothing_on_screen);
        };
        // An input is on screen as an "app" the app list does not have:
        // its name is in the input list, as the TV shows it.
        let name = if app.starts_with("com.webos.app.hdmi") || app.contains("livetv") {
            let inputs = self.inputs().await.unwrap_or_default();
            input_name(&inputs, &app)
        } else {
            self.apps()
                .await
                .ok()
                .and_then(|apps| apps.into_iter().find(|a| a.id == app))
                .map(|a| a.title)
        };
        Ok(Status {
            app: Some(name.unwrap_or_else(|| readable(&app))),
            ..nothing_on_screen
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

/// The name of the input whose app is `app_id`.
fn input_name(inputs: &[Input], app_id: &str) -> Option<String> {
    inputs
        .iter()
        .find(|i| i.app_id.as_deref() == Some(app_id))
        .map(|i| i.label.clone())
}

/// Something a person can read for an app id nothing named:
/// "com.webos.app.hdmi2" is HDMI 2, "com.webos.app.livetv" is Live TV.
fn readable(app_id: &str) -> String {
    if let Some(n) = app_id.strip_prefix("com.webos.app.hdmi") {
        return format!("HDMI {n}");
    }
    if app_id.contains("livetv") {
        return "Live TV".into();
    }
    app_id.to_string()
}

fn app_in(answer: &Value) -> Option<String> {
    answer["appId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// What the TV says of its power: "Active", "Screen Off", "Active
/// Standby". An older TV may have no power service to ask, and is then
/// on when something is on screen — how Home Assistant's webOS library
/// tells, for the same TVs.
async fn power_state(session: &mut Session) -> Result<String> {
    match session.request(POWER_STATE, json!({})).await {
        Ok(answer) => Ok(answer["state"].as_str().unwrap_or_default().to_string()),
        Err(Error::Request { .. }) => {
            let answer = session.request(FOREGROUND_APP, json!({})).await?;
            let state = if app_in(&answer).is_some() {
                "Active"
            } else {
                "Suspend"
            };
            Ok(state.to_string())
        }
        Err(e) => Err(e),
    }
}

/// "Active", or a screen saver over it. "Screen Off" is sound with the
/// panel dark; "Active Standby" and "Suspend" are the TV asleep but
/// listening.
fn screen_on(state: &str) -> bool {
    matches!(state, "Active" | "Screen Saver")
}

/// Whether there is sound to change: a dark screen has not silenced it.
fn sound_on(state: &str) -> bool {
    screen_on(state) || state == "Screen Off"
}

async fn changed(session: &mut Session, change: VolumeChange) -> Result<Option<Volume>> {
    if !sound_on(&power_state(session).await?) {
        return Ok(None);
    }
    match change {
        VolumeChange::To(level) => {
            session.request(SET_VOLUME, volume_payload(level)).await?;
        }
        VolumeChange::By(step) => {
            let now = volume_in(&session.request(GET_VOLUME, json!({})).await?)?;
            session
                .request(SET_VOLUME, volume_payload(stepped(now.volume, step)))
                .await?;
        }
        VolumeChange::Mute(mute) => {
            session.request(SET_MUTE, json!({ "mute": mute })).await?;
        }
    }
    // Read back rather than assumed: a TV can cap its volume below 100.
    volume_in(&session.request(GET_VOLUME, json!({})).await?).map(Some)
}

fn volume_payload(level: u8) -> Value {
    json!({ "volume": level.min(100) })
}

fn stepped(now: u8, step: i16) -> u8 {
    (i16::from(now) + step).clamp(0, 100) as u8
}

/// The volume in either shape a TV answers with. webOS 5 and later nest
/// it — `{"volumeStatus": {"volume": 11, "muteStatus": false, …}}` —
/// while older TVs put it at the top, muted as `muted` or `mute`:
/// `{"scenario": "mastervolume_tv_speaker", "volume": 9, "muted": false}`.
fn volume_in(answer: &Value) -> Result<Volume> {
    let status = answer.get("volumeStatus").unwrap_or(answer);
    let volume = status["volume"]
        .as_u64()
        .ok_or_else(|| Error::Protocol(format!("no volume in {answer}")))?;
    let muted = ["muteStatus", "muted", "mute"]
        .iter()
        .find_map(|key| status[key].as_bool())
        .unwrap_or(false);
    Ok(Volume {
        volume: volume.min(100) as u8,
        muted,
    })
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
    use futures_util::{SinkExt, StreamExt};
    use std::sync::{Arc, Mutex};
    use tokio_tungstenite::tungstenite::Message;

    #[test]
    fn an_input_on_screen_is_called_what_the_tv_calls_it() {
        let inputs = [
            Input {
                id: "HDMI_1".into(),
                label: "PlayStation".into(),
                app_id: Some("com.webos.app.hdmi1".into()),
            },
            Input {
                id: "HDMI_2".into(),
                label: "HDMI 2".into(),
                app_id: Some("com.webos.app.hdmi2".into()),
            },
        ];
        assert_eq!(
            input_name(&inputs, "com.webos.app.hdmi1").as_deref(),
            Some("PlayStation")
        );
        assert_eq!(
            input_name(&inputs, "com.webos.app.hdmi2").as_deref(),
            Some("HDMI 2")
        );
        assert_eq!(input_name(&inputs, "netflix"), None);
    }

    #[test]
    fn an_id_nothing_named_is_still_readable() {
        assert_eq!(readable("com.webos.app.hdmi2"), "HDMI 2");
        assert_eq!(readable("com.webos.app.livetv"), "Live TV");
        assert_eq!(readable("some.other.app"), "some.other.app");
    }

    #[test]
    fn standby_is_not_on() {
        assert!(screen_on("Active"));
        assert!(!screen_on("Active Standby"));
        assert!(!screen_on(""));
    }

    #[test]
    fn a_dark_screen_still_has_sound() {
        assert!(sound_on("Screen Off"));
        assert!(sound_on("Active"));
        assert!(!sound_on("Suspend"));
        assert!(!sound_on("Active Standby"));
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

    #[test]
    fn the_volume_of_a_newer_tv_is_nested() {
        // A 2020 55GX6LA, as recorded in openhab-addons#9000.
        let answer = json!({
            "volumeStatus": {
                "cause": "volumeDown", "mode": "normal", "adjustVolume": true,
                "activeStatus": true, "muteStatus": false, "volume": 20,
                "soundOutput": "tv_speaker", "maxVolume": 100
            },
            "returnValue": true,
            "callerId": "com.webos.platformstarfish"
        });
        assert_eq!(
            volume_in(&answer).unwrap(),
            Volume {
                volume: 20,
                muted: false
            }
        );
        // node-red-contrib-lgtv#51: a 2021 TV, with the old fields
        // repeated beside the nested ones.
        let answer = json!({
            "returnValue": true,
            "volumeStatus": {
                "activeStatus": true, "adjustVolume": true, "maxVolume": 100,
                "muteStatus": true, "volume": 12, "mode": "normal",
                "soundOutput": "tv_external_speaker"
            },
            "callerId": "com.webos.service.apiadapter",
            "mute": true,
            "volume": 12
        });
        assert_eq!(
            volume_in(&answer).unwrap(),
            Volume {
                volume: 12,
                muted: true
            }
        );
    }

    #[test]
    fn the_volume_of_an_older_tv_is_at_the_top() {
        let answer = json!({
            "returnValue": true,
            "scenario": "mastervolume_tv_speaker",
            "volume": 9,
            "muted": true
        });
        assert_eq!(
            volume_in(&answer).unwrap(),
            Volume {
                volume: 9,
                muted: true
            }
        );
        // node-red-contrib-lgtv#51 recorded one with "mute".
        let answer = json!({
            "returnValue": true,
            "scenario": "mastervolume_tv_speaker",
            "volume": 98,
            "mute": false
        });
        assert_eq!(
            volume_in(&answer).unwrap(),
            Volume {
                volume: 98,
                muted: false
            }
        );
    }

    #[test]
    fn no_volume_is_an_answer_that_makes_no_sense() {
        assert!(matches!(
            volume_in(&json!({ "returnValue": true })),
            Err(Error::Protocol(_))
        ));
    }

    #[test]
    fn a_volume_is_asked_for_within_range() {
        assert_eq!(volume_payload(20), json!({ "volume": 20 }));
        assert_eq!(volume_payload(250), json!({ "volume": 100 }));
        assert_eq!(stepped(95, 10), 100);
        assert_eq!(stepped(5, -10), 0);
        assert_eq!(stepped(20, -10), 10);
    }

    // ---- A TV on loopback ---------------------------------------------------

    /// Which webOS a fake TV answers as.
    #[derive(Clone, Copy)]
    enum WebOs {
        /// webOS 3: the volume at the top, and no power service.
        Three,
        /// webOS 5 and later: the volume nested, and a power state.
        Newer(&'static str),
    }

    struct FakeTv {
        port: u16,
        /// Every request, by URI without `ssap://`, with its payload.
        asked: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl FakeTv {
        fn tv(&self) -> Tv {
            Tv {
                host: "127.0.0.1".into(),
                port: self.port,
                client_key: "key".into(),
            }
        }

        fn asked(&self) -> Vec<(String, Value)> {
            self.asked.lock().unwrap().clone()
        }

        fn uris(&self) -> Vec<String> {
            self.asked().into_iter().map(|(uri, _)| uri).collect()
        }
    }

    struct Sound {
        volume: u64,
        muted: bool,
    }

    /// A TV that registers anybody, keeps a volume, and answers the
    /// audio requests the way `webos` does. Anything else is the error
    /// a TV gives for a service it does not have.
    async fn fake_tv(webos: WebOs, volume: u64, app: &'static str) -> FakeTv {
        let made = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
        let cert = rustls::pki_types::CertificateDer::from(made.cert.der().to_vec());
        let key =
            rustls::pki_types::PrivateKeyDer::try_from(made.signing_key.serialize_der()).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let sound = Arc::new(Mutex::new(Sound {
            volume,
            muted: false,
        }));
        let kept = asked.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let asked = kept.clone();
                let sound = sound.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let Ok(mut ws) = tokio_tungstenite::accept_async(tls).await else {
                        return;
                    };
                    while let Some(Ok(Message::Text(text))) = ws.next().await {
                        let message: Value = serde_json::from_str(&text).unwrap();
                        let id = message["id"].clone();
                        let reply = if message["type"] == "register" {
                            json!({ "type": "registered", "id": id, "payload": { "client-key": "key" } })
                        } else {
                            let uri = message["uri"]
                                .as_str()
                                .unwrap()
                                .trim_start_matches("ssap://");
                            let payload = message["payload"].clone();
                            asked
                                .lock()
                                .unwrap()
                                .push((uri.to_string(), payload.clone()));
                            match answer(webos, app, &mut sound.lock().unwrap(), uri, &payload) {
                                Some(payload) => {
                                    json!({ "type": "response", "id": id, "payload": payload })
                                }
                                None => json!({
                                    "type": "error",
                                    "id": id,
                                    "error": "404 no such service or method"
                                }),
                            }
                        };
                        if ws
                            .send(Message::Text(reply.to_string().into()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        FakeTv { port, asked }
    }

    fn answer(
        webos: WebOs,
        app: &str,
        sound: &mut Sound,
        uri: &str,
        payload: &Value,
    ) -> Option<Value> {
        match (uri, webos) {
            (POWER_STATE, WebOs::Newer(state)) => {
                Some(json!({ "returnValue": true, "state": state }))
            }
            (FOREGROUND_APP, _) => Some(json!({ "returnValue": true, "appId": app })),
            (GET_VOLUME, WebOs::Three) => Some(json!({
                "returnValue": true,
                "scenario": "mastervolume_tv_speaker",
                "volume": sound.volume,
                "muted": sound.muted
            })),
            (GET_VOLUME, WebOs::Newer(_)) => Some(json!({
                "returnValue": true,
                "volumeStatus": {
                    "activeStatus": true, "adjustVolume": true, "maxVolume": 100,
                    "muteStatus": sound.muted, "volume": sound.volume,
                    "soundOutput": "tv_speaker"
                },
                "callerId": "com.webos.service.apiadapter"
            })),
            (SET_VOLUME, _) => {
                sound.volume = payload["volume"].as_u64()?;
                Some(json!({ "returnValue": true }))
            }
            (SET_MUTE, _) => {
                sound.muted = payload["mute"].as_bool()?;
                Some(json!({ "returnValue": true }))
            }
            ("audio/volumeUp", _) => {
                sound.volume += 1;
                Some(json!({ "returnValue": true }))
            }
            ("audio/volumeDown", _) => {
                sound.volume -= 1;
                Some(json!({ "returnValue": true }))
            }
            _ => None,
        }
    }

    #[tokio::test]
    async fn a_volume_is_set_and_read_back() {
        let fake = fake_tv(WebOs::Newer("Active"), 11, "netflix").await;
        let after = fake.tv().change_volume(VolumeChange::To(20)).await.unwrap();
        assert_eq!(
            after,
            Some(Volume {
                volume: 20,
                muted: false
            })
        );
        assert_eq!(
            fake.asked(),
            vec![
                (POWER_STATE.into(), json!({})),
                (SET_VOLUME.into(), json!({ "volume": 20 })),
                (GET_VOLUME.into(), json!({})),
            ]
        );
    }

    #[tokio::test]
    async fn a_step_moves_from_where_the_volume_is() {
        let fake = fake_tv(WebOs::Newer("Active"), 15, "netflix").await;
        let after = fake
            .tv()
            .change_volume(VolumeChange::By(-10))
            .await
            .unwrap();
        assert_eq!(after.map(|v| v.volume), Some(5));
        assert!(
            fake.asked()
                .contains(&(SET_VOLUME.into(), json!({ "volume": 5 })))
        );
    }

    #[tokio::test]
    async fn muting_says_muted() {
        let fake = fake_tv(WebOs::Newer("Screen Off"), 15, "").await;
        let after = fake
            .tv()
            .change_volume(VolumeChange::Mute(true))
            .await
            .unwrap();
        assert_eq!(
            after,
            Some(Volume {
                volume: 15,
                muted: true
            })
        );
        assert!(
            fake.asked()
                .contains(&(SET_MUTE.into(), json!({ "mute": true })))
        );
    }

    #[tokio::test]
    async fn an_older_tv_with_no_power_service_is_on_while_an_app_is() {
        // An OLED55B7V (2017) runs webOS 3.5.
        let fake = fake_tv(WebOs::Three, 9, "com.webos.app.livetv").await;
        let after = fake.tv().change_volume(VolumeChange::By(10)).await.unwrap();
        assert_eq!(
            after,
            Some(Volume {
                volume: 19,
                muted: false
            })
        );
        assert_eq!(
            fake.uris(),
            [
                POWER_STATE,
                FOREGROUND_APP,
                GET_VOLUME,
                SET_VOLUME,
                GET_VOLUME
            ]
        );
    }

    #[tokio::test]
    async fn a_tv_asleep_is_off_and_left_alone() {
        let fake = fake_tv(WebOs::Newer("Active Standby"), 9, "").await;
        let after = fake.tv().change_volume(VolumeChange::To(30)).await.unwrap();
        assert_eq!(after, None);
        assert_eq!(fake.uris(), [POWER_STATE]);
    }

    #[tokio::test]
    async fn a_tv_nobody_answers_for_is_off() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let tv = Tv {
            host: "127.0.0.1".into(),
            port,
            client_key: "key".into(),
        };
        assert_eq!(tv.change_volume(VolumeChange::To(30)).await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_remote_buttons_and_the_plain_calls() {
        let fake = fake_tv(WebOs::Three, 10, "netflix").await;
        let tv = fake.tv();
        tv.volume_up().await.unwrap();
        tv.volume_up().await.unwrap();
        tv.volume_down().await.unwrap();
        assert_eq!(tv.volume().await.unwrap().volume, 11);
        tv.set_volume(140).await.unwrap();
        tv.set_mute(true).await.unwrap();
        assert_eq!(
            tv.volume().await.unwrap(),
            Volume {
                volume: 100,
                muted: true
            }
        );
    }

    #[tokio::test]
    async fn the_status_of_a_tv_that_is_on_carries_its_volume() {
        let fake = fake_tv(WebOs::Newer("Active"), 23, "").await;
        let status = fake.tv().status().await.unwrap();
        assert_eq!(status.volume, Some(23));
        assert_eq!(status.muted, Some(false));
        let shown = serde_json::to_value(&status).unwrap();
        assert_eq!(shown["volume"], 23);
    }

    #[tokio::test]
    async fn the_status_of_a_tv_that_is_off_has_no_volume() {
        let fake = fake_tv(WebOs::Newer("Suspend"), 23, "").await;
        let status = fake.tv().status().await.unwrap();
        assert!(!status.on);
        let shown = serde_json::to_value(&status).unwrap();
        assert!(shown.get("volume").is_none());
    }
}
