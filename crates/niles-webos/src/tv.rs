//! What Niles asks of an LG TV.

use crate::error::{Error, Result};
use crate::session::{PORT, Session};
use serde::Serialize;
use serde_json::{Value, json};
use std::net::IpAddr;
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
    /// Where its sound goes, when it says: "tv_speaker" is its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound_output: Option<String>,
}

/// How loud the TV's own sound is, `0..=100`, and whether it is muted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Volume {
    pub volume: u8,
    pub muted: bool,
    /// Where the sound goes — "tv_speaker", "external_arc" — on a TV
    /// new enough to say. An older one does not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound_output: Option<String>,
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

pub(crate) const POWER_STATE: &str = "com.webos.service.tvpower/power/getPowerState";
pub(crate) const FOREGROUND_APP: &str = "com.webos.applicationManager/getForegroundAppInfo";
pub(crate) const GET_VOLUME: &str = "audio/getVolume";
pub(crate) const SET_VOLUME: &str = "audio/setVolume";
pub(crate) const SET_MUTE: &str = "audio/setMute";
/// A MAC per interface, whether or not it has a link:
/// `{"wiredInfo": {"macAddress": "…"}, "wifiInfo": {"macAddress": "…"}, "p2pInfo": …}`.
pub(crate) const CONNECTION_INFO: &str = "com.webos.service.connectionmanager/getinfo";
/// Which interface has a link, and its address, but no MACs:
/// `{"wired": {"state": "disconnected"}, "wifi": {"state": "connected",
/// "ipAddress": "192.168.1.50", …}}`. Lowercase on the wire: a 2023 TV
/// answers `getStatus` with a 404.
pub(crate) const CONNECTION_STATUS: &str = "com.webos.service.connectionmanager/getstatus";

/// How long a TV gets to answer after the wake-up is sent. An OLED42C3
/// took about 4 s from standby; an older TV without Quick Start+ boots
/// slower. Long enough for both, short enough to wait on in the app.
pub const WAKING: Duration = Duration::from_secs(10);
/// Between asks while it wakes.
const WAKING_STEP: Duration = Duration::from_secs(1);

/// The TV's MAC addresses, for waking it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Macs {
    /// The interface it is connected by, as near as it can be told.
    pub chosen: String,
    /// Every MAC it reported that a magic packet could be for,
    /// `chosen` first.
    pub all: Vec<String>,
}

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
        Self::at(host, PORT, client_key)
    }

    pub(crate) fn at(host: impl Into<String>, port: u16, client_key: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
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
                sound_output: None,
            });
        }
        let sound = self.volume().await.ok();
        let nothing_on_screen = Status {
            on,
            app: None,
            volume: sound.as_ref().map(|s| s.volume),
            muted: sound.as_ref().map(|s| s.muted),
            sound_output: sound.and_then(|s| s.sound_output),
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

    /// The TV's MAC addresses, with the one it is connected by chosen as
    /// `macs_in` tells. `None` when it reports none a packet could be for.
    pub async fn macs(&self) -> Result<Option<Macs>> {
        let mut session = self.connect().await?;
        let info = session.request(CONNECTION_INFO, json!({})).await;
        let status = match &info {
            Ok(_) => match session.request(CONNECTION_STATUS, json!({})).await {
                Ok(status) => Some(status),
                Err(e) => {
                    tracing::debug!("[tv] which interface is connected: {e}");
                    None
                }
            },
            Err(_) => None,
        };
        session.close().await;
        let addresses = addresses_of(&self.host).await;
        Ok(macs_in(&info?, status.as_ref(), &addresses))
    }

    /// Whether the screen comes on within `within`, asking every second.
    /// A TV in standby keeps its network up but turns the websocket away,
    /// so an error is a TV not awake yet rather than an answer.
    pub async fn came_on(&self, within: Duration) -> bool {
        let asking = async {
            loop {
                if let Ok(true) = self.screen_on().await {
                    return;
                }
                tokio::time::sleep(WAKING_STEP).await;
            }
        };
        tokio::time::timeout(within, asking).await.is_ok()
    }
}

/// The addresses `host` stands for: itself when it is one, otherwise
/// what it resolves to, or none when it does not.
async fn addresses_of(host: &str) -> Vec<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return vec![ip];
    }
    match tokio::net::lookup_host((host, 0)).await {
        Ok(found) => found.map(|a| a.ip()).collect(),
        Err(_) => Vec::new(),
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
        sound_output: status["soundOutput"].as_str().map(str::to_string),
    })
}

/// Each interface a TV can be woken on: its block in `getinfo`, and in
/// `getstatus`. Wired first, which is the order of the last resort
/// below. `p2pInfo` is Wi-Fi Direct, which nothing wakes.
const INTERFACES: [(&str, &str); 2] = [("wiredInfo", "wired"), ("wifiInfo", "wifi")];

/// The MACs worth a magic packet, and the one the TV is connected by.
///
/// `getinfo` lists a wired MAC on a TV that has never had a cable in, so
/// it cannot say which is in use on its own; `getstatus` says which
/// interface has a link, and at what address. In order:
///
/// 1. The interface at the address Niles reaches the TV by (`host`).
/// 2. The first connected one, wired before Wi-Fi. A TV with both up is
///    rare, since webOS drops Wi-Fi when a cable is in; when it happens
///    and neither has Niles's address, the cable is the steadier wake.
/// 3. With nothing to tell by — an older TV, or `getstatus` refused —
///    wired before Wi-Fi. Only a last resort: it is what chose a dead
///    NIC on a Wi-Fi TV, and why every MAC is woken as well.
///
/// A MAC that is empty, all zeros, or not one at all is never chosen.
fn macs_in(info: &Value, status: Option<&Value>, host: &[IpAddr]) -> Option<Macs> {
    let unknown = Value::Null;
    let found: Vec<(String, &Value)> = INTERFACES
        .iter()
        .filter_map(|(mac_key, link_key)| {
            let mac = usable_mac(info[mac_key]["macAddress"].as_str()?)?;
            Some((mac, status.map_or(&unknown, |s| &s[link_key])))
        })
        .collect();
    let connected = |link: &Value| link["state"] == "connected";
    let at_host = |link: &Value| {
        link["ipAddress"]
            .as_str()
            .and_then(|ip| ip.parse::<IpAddr>().ok())
            .is_some_and(|ip| host.contains(&ip))
    };
    let chosen = found
        .iter()
        .position(|(_, link)| at_host(link))
        .or_else(|| found.iter().position(|(_, link)| connected(link)))
        .unwrap_or(0);
    let (chosen, _) = found.get(chosen)?;
    let mut all = vec![chosen.clone()];
    all.extend(
        found
            .iter()
            .map(|(mac, _)| mac.clone())
            .filter(|mac| mac != chosen),
    );
    Some(Macs {
        chosen: chosen.clone(),
        all,
    })
}

/// `mac` in lowercase with colons, when a magic packet could be for it.
fn usable_mac(mac: &str) -> Option<String> {
    let bytes: Vec<u8> = mac
        .trim()
        .split([':', '-'])
        .map(|b| u8::from_str_radix(b, 16).ok().filter(|_| b.len() == 2))
        .collect::<Option<_>>()?;
    if bytes.len() != 6 || bytes.iter().all(|b| *b == 0) {
        return None;
    }
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Some(hex.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Network, WebOs, fake_tv, fake_tv_on};

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

    const WIRED: &str = "64:e4:a5:6c:ab:b4";
    const WIFI: &str = "84:90:0a:63:a9:94";

    /// `getinfo` as an OLED42C3 answers it, recorded in TisVlay's
    /// docs/lg/firmware-quirks.md: every interface, linked or not.
    fn info() -> Value {
        json!({
            "returnValue": true, "subscribed": false,
            "wiredInfo": { "macAddress": "64:E4:A5:6C:AB:B4" },
            "wifiInfo": { "macAddress": "84:90:0A:63:A9:94" },
            "p2pInfo": { "macAddress": "86:90:0A:63:A9:94" }
        })
    }

    /// `getstatus` with `wired` and `wifi` in the given states; the
    /// connected ones have an address, as the TV gives only then.
    fn status(wired: Option<&str>, wifi: Option<&str>) -> Value {
        let link = |ip: Option<&str>| match ip {
            Some(ip) => json!({ "state": "connected", "interfaceName": "x", "ipAddress": ip }),
            None => json!({ "state": "disconnected" }),
        };
        json!({
            "returnValue": true,
            "isInternetConnectionAvailable": true,
            "wired": link(wired),
            "wifi": link(wifi),
            "wifiDirect": { "state": "disconnected" }
        })
    }

    fn chosen(info: &Value, status: Option<&Value>, host: &str) -> Option<String> {
        let host: Vec<IpAddr> = host.parse().into_iter().collect();
        macs_in(info, status, &host).map(|m| m.chosen)
    }

    #[test]
    fn a_tv_on_wifi_is_woken_on_wifi() {
        let on_wifi = status(None, Some("192.168.4.170"));
        assert_eq!(
            chosen(&info(), Some(&on_wifi), "192.168.4.170").as_deref(),
            Some(WIFI)
        );
        // Whatever address Niles has for it.
        assert_eq!(
            chosen(&info(), Some(&on_wifi), "10.0.0.9").as_deref(),
            Some(WIFI)
        );
    }

    #[test]
    fn a_tv_on_a_cable_is_woken_on_the_cable() {
        let wired = status(Some("192.168.4.20"), None);
        assert_eq!(
            chosen(&info(), Some(&wired), "192.168.4.20").as_deref(),
            Some(WIRED)
        );
    }

    #[test]
    fn with_both_up_the_one_at_the_tvs_address_wins() {
        let both = status(Some("192.168.4.20"), Some("192.168.4.170"));
        assert_eq!(
            chosen(&info(), Some(&both), "192.168.4.170").as_deref(),
            Some(WIFI)
        );
        assert_eq!(
            chosen(&info(), Some(&both), "192.168.4.20").as_deref(),
            Some(WIRED)
        );
        // Neither at the address: the cable, every time.
        assert_eq!(
            chosen(&info(), Some(&both), "10.0.0.9").as_deref(),
            Some(WIRED)
        );
    }

    #[test]
    fn with_nothing_to_tell_by_wired_comes_first_and_every_mac_is_kept() {
        let macs = macs_in(&info(), None, &[]).unwrap();
        assert_eq!(macs.chosen, WIRED);
        assert_eq!(macs.all, [WIRED, WIFI]);
        // A status of a shape this does not know says nothing either.
        let odd = json!({ "returnValue": true });
        assert_eq!(chosen(&info(), Some(&odd), "").as_deref(), Some(WIRED));
    }

    #[test]
    fn the_chosen_mac_comes_first_and_wifi_direct_is_left_out() {
        let on_wifi = status(None, Some("192.168.4.170"));
        let macs = macs_in(&info(), Some(&on_wifi), &[]).unwrap();
        assert_eq!(macs.all, [WIFI, WIRED]);
    }

    #[test]
    fn an_empty_or_zero_mac_is_never_chosen() {
        let wired_zero = json!({
            "wiredInfo": { "macAddress": "00:00:00:00:00:00" },
            "wifiInfo": { "macAddress": "A8:23:FE:01:02:03" }
        });
        // Even when the TV says the cable is the one connected.
        let wired_up = status(Some("192.168.4.20"), None);
        let macs = macs_in(&wired_zero, Some(&wired_up), &[]).unwrap();
        assert_eq!(macs.chosen, "a8:23:fe:01:02:03");
        assert_eq!(macs.all, ["a8:23:fe:01:02:03"]);
        let wifi_empty = json!({
            "wiredInfo": { "macAddress": "64:E4:A5:6C:AB:B4" },
            "wifiInfo": { "macAddress": "" }
        });
        let on_wifi = status(None, Some("192.168.4.170"));
        assert_eq!(
            chosen(&wifi_empty, Some(&on_wifi), "192.168.4.170").as_deref(),
            Some(WIRED)
        );
        let nothing = json!({
            "wiredInfo": { "macAddress": "" },
            "wifiInfo": { "macAddress": "not a mac" }
        });
        assert_eq!(macs_in(&nothing, None, &[]), None);
        assert_eq!(macs_in(&json!({}), None, &[]), None);
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
                muted: false,
                sound_output: Some("tv_speaker".into()),
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
                muted: true,
                sound_output: Some("tv_external_speaker".into()),
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
                muted: true,
                sound_output: None,
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
                muted: false,
                sound_output: None,
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

    #[tokio::test]
    async fn a_volume_is_set_and_read_back() {
        let fake = fake_tv(WebOs::Newer("Active"), 11, "netflix").await;
        let after = fake.tv().change_volume(VolumeChange::To(20)).await.unwrap();
        assert_eq!(
            after,
            Some(Volume {
                volume: 20,
                muted: false,
                sound_output: Some("tv_speaker".into()),
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
                muted: true,
                sound_output: Some("tv_speaker".into()),
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
                muted: false,
                sound_output: None,
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
                muted: true,
                sound_output: None,
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
        assert_eq!(shown["sound_output"], "tv_speaker");
    }

    #[tokio::test]
    async fn the_status_of_a_tv_that_is_off_has_no_volume() {
        let fake = fake_tv(WebOs::Newer("Suspend"), 23, "").await;
        let status = fake.tv().status().await.unwrap();
        assert!(!status.on);
        let shown = serde_json::to_value(&status).unwrap();
        assert!(shown.get("volume").is_none());
    }

    #[tokio::test]
    async fn the_macs_are_asked_for_over_one_connection() {
        // Niles reaches the fake at 127.0.0.1, which is its Wi-Fi.
        let network = Network {
            info: Some(info()),
            status: Some(status(None, Some("127.0.0.1"))),
        };
        let fake = fake_tv_on(WebOs::Newer("Active"), 0, "", network).await;
        let macs = fake.tv().macs().await.unwrap().unwrap();
        assert_eq!(macs.chosen, WIFI);
        assert_eq!(macs.all, [WIFI, WIRED]);
        assert_eq!(fake.uris(), [CONNECTION_INFO, CONNECTION_STATUS]);
    }

    #[tokio::test]
    async fn a_tv_with_no_connection_status_still_gives_its_macs() {
        let network = Network {
            info: Some(info()),
            status: None,
        };
        let fake = fake_tv_on(WebOs::Three, 0, "", network).await;
        let macs = fake.tv().macs().await.unwrap().unwrap();
        assert_eq!(macs.chosen, WIRED);
        assert_eq!(macs.all, [WIRED, WIFI]);
    }

    #[tokio::test]
    async fn a_tv_that_is_on_has_come_on() {
        let fake = fake_tv(WebOs::Newer("Active"), 0, "").await;
        assert!(fake.tv().came_on(Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn a_tv_still_asleep_has_not_come_on() {
        let fake = fake_tv(WebOs::Newer("Active Standby"), 0, "").await;
        assert!(!fake.tv().came_on(Duration::from_millis(1500)).await);
        assert!(fake.uris().len() >= 2, "asked again: {:?}", fake.uris());
    }
}
