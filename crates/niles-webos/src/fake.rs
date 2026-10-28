//! A TV on loopback, for the tests here and in the crates that drive one.

#![allow(
    clippy::unwrap_used,
    reason = "a test double: a failure fails the test using it"
)]

use crate::tv::{
    CONNECTION_INFO, CONNECTION_STATUS, FOREGROUND_APP, GET_VOLUME, POWER_STATE, SET_MUTE,
    SET_VOLUME, Tv,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message;

/// Which webOS a fake TV answers as.
#[derive(Debug, Clone, Copy)]
pub enum WebOs {
    /// webOS 3: the volume at the top, and no power service.
    Three,
    /// webOS 5 and later: the volume nested, and a power state.
    Newer(&'static str),
}

/// What the connection manager says, for a fake TV that has one.
#[derive(Debug, Clone, Default)]
pub struct Network {
    /// The `getinfo` answer: a MAC per interface.
    pub info: Option<Value>,
    /// The `getstatus` answer: which interface is connected, at what
    /// address. `None` is a TV that has no such method.
    pub status: Option<Value>,
}

pub struct FakeTv {
    port: u16,
    /// Every request, by URI without `ssap://`, with its payload.
    asked: Arc<Mutex<Vec<(String, Value)>>>,
}

impl FakeTv {
    /// The TV, as Niles reaches it, paired.
    pub fn tv(&self) -> Tv {
        Tv::at("127.0.0.1", self.port, "key")
    }

    pub fn asked(&self) -> Vec<(String, Value)> {
        self.asked.lock().unwrap().clone()
    }

    pub fn uris(&self) -> Vec<String> {
        self.asked().into_iter().map(|(uri, _)| uri).collect()
    }
}

struct Sound {
    volume: u64,
    muted: bool,
}

/// A TV that registers anybody, keeps a volume, and answers the audio
/// requests the way `webos` does. Anything else is the error a TV gives
/// for a service it does not have.
pub async fn fake_tv(webos: WebOs, volume: u64, app: &'static str) -> FakeTv {
    fake_tv_on(webos, volume, app, Network::default()).await
}

/// The same, with a connection manager that answers as `network` says.
pub async fn fake_tv_on(webos: WebOs, volume: u64, app: &'static str, network: Network) -> FakeTv {
    let made = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
    let cert = rustls::pki_types::CertificateDer::from(made.cert.der().to_vec());
    let key = rustls::pki_types::PrivateKeyDer::try_from(made.signing_key.serialize_der()).unwrap();
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
    let network = Arc::new(network);
    let kept = asked.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let asked = kept.clone();
            let sound = sound.clone();
            let network = network.clone();
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
                        let answered = answer(
                            webos,
                            app,
                            &network,
                            &mut sound.lock().unwrap(),
                            uri,
                            &payload,
                        );
                        match answered {
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
    network: &Network,
    sound: &mut Sound,
    uri: &str,
    payload: &Value,
) -> Option<Value> {
    match (uri, webos) {
        (POWER_STATE, WebOs::Newer(state)) => Some(json!({ "returnValue": true, "state": state })),
        (FOREGROUND_APP, _) => Some(json!({ "returnValue": true, "appId": app })),
        (CONNECTION_INFO, _) => network.info.clone(),
        (CONNECTION_STATUS, _) => network.status.clone(),
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
