//! One conversation with the TV: connect, register, ask, hang up.
//!
//! webOS speaks JSON over a websocket on port 3001, behind TLS with a
//! certificate the TV made itself. Nobody signed it, so there is nothing
//! to check it against: the connection is encrypted, and trusting it is
//! the same trust Niles gives the TV's address in the config. Newer
//! firmware leaves port 3000 open and never answers on it, which is why
//! there is no plain fallback.
//!
//! A session per command rather than one held open: a TV is off most of
//! the day, and a connection that has to notice that and come back is
//! more machinery than a handshake every time somebody says "Netflix".

use crate::error::{Error, Result};
use futures_util::{SinkExt, StreamExt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

/// Long enough for a TV on a busy network; short enough that a command
/// to a TV that is off fails while the person is still listening.
const CONNECT: Duration = Duration::from_secs(4);
/// How long one request may take once connected.
const ANSWER: Duration = Duration::from_secs(6);

/// What Niles asks to be allowed. The list LG's own tools ask for; the
/// TV shows it on the pairing prompt.
fn registration(client_key: Option<&str>) -> Value {
    json!({
        "type": "register",
        "id": "register_0",
        "payload": {
            "forcePairing": false,
            "pairingType": "PROMPT",
            "client-key": client_key,
            "manifest": {
                "appVersion": "1.1",
                "manifestVersion": 1,
                "permissions": [
                    "APP_TO_APP", "CLOSE", "CONTROL_AUDIO", "CONTROL_DISPLAY",
                    "CONTROL_INPUT_JOYSTICK", "CONTROL_INPUT_MEDIA_PLAYBACK",
                    "CONTROL_INPUT_MEDIA_RECORDING", "CONTROL_INPUT_TEXT", "CONTROL_INPUT_TV",
                    "CONTROL_MOUSE_AND_KEYBOARD", "CONTROL_POWER", "CONTROL_TV_SCREEN",
                    "LAUNCH", "LAUNCH_WEBAPP", "READ_APP_STATUS", "READ_COUNTRY_INFO",
                    "READ_CURRENT_CHANNEL", "READ_INPUT_DEVICE_LIST", "READ_INSTALLED_APPS",
                    "READ_LGE_SDX", "READ_LGE_TV_INPUT_EVENTS", "READ_NETWORK_STATE",
                    "READ_NOTIFICATIONS", "READ_POWER_STATE", "READ_RUNNING_APPS",
                    "READ_SETTINGS", "READ_TV_CHANNEL_LIST", "READ_TV_CURRENT_TIME",
                    "READ_UPDATE_INFO", "SEARCH", "TEST_OPEN", "TEST_PROTECTED", "TEST_SECURE",
                    "UPDATE_FROM_REMOTE_APP", "WRITE_NOTIFICATION_ALERT",
                    "WRITE_NOTIFICATION_TOAST", "WRITE_SETTINGS"
                ]
            }
        }
    })
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(crate) struct Session {
    socket: Socket,
    next: u32,
}

impl Session {
    /// Connect and register. With a key the TV registers at once; without
    /// one it shows the pairing prompt, and this waits `pairing` for
    /// somebody to accept it — returning the key the TV issued.
    pub(crate) async fn open(
        host: &str,
        client_key: Option<&str>,
        pairing: Duration,
    ) -> Result<(Session, String)> {
        let unreachable = |reason: String| Error::Unreachable {
            host: host.to_string(),
            reason,
        };
        let connector = Connector::Rustls(Arc::new(trusting_client()));
        let (socket, _) = tokio::time::timeout(
            CONNECT,
            tokio_tungstenite::connect_async_tls_with_config(
                format!("wss://{host}:3001"),
                None,
                false,
                Some(connector),
            ),
        )
        .await
        .map_err(|_| unreachable("no answer".into()))?
        .map_err(|e| unreachable(e.to_string()))?;
        let mut session = Session { socket, next: 1 };
        session.send(registration(client_key)).await?;
        let key = tokio::time::timeout(pairing.max(ANSWER), session.registered())
            .await
            .map_err(|_| Error::Refused("nobody accepted the prompt on the TV".into()))??;
        Ok((session, key))
    }

    /// Wait for "registered", passing the prompt notice on the way.
    async fn registered(&mut self) -> Result<String> {
        loop {
            let message = self.receive().await?;
            match message["type"].as_str() {
                Some("registered") => {
                    return message["payload"]["client-key"]
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| Error::Protocol("registered without a key".into()));
                }
                Some("error") => {
                    return Err(Error::Refused(
                        message["error"].as_str().unwrap_or("refused").to_string(),
                    ));
                }
                // "response" with pairingType PROMPT: the TV is asking.
                _ => continue,
            }
        }
    }

    /// One request, and its answer's payload.
    pub(crate) async fn request(&mut self, uri: &str, payload: Value) -> Result<Value> {
        let id = format!("niles_{}", self.next);
        self.next += 1;
        self.send(json!({
            "type": "request",
            "id": id,
            "uri": format!("ssap://{uri}"),
            "payload": payload,
        }))
        .await?;
        let failed = |reason: String| Error::Request {
            request: uri.to_string(),
            reason,
        };
        let answer = tokio::time::timeout(ANSWER, async {
            loop {
                let message = self.receive().await?;
                if message["id"].as_str() == Some(id.as_str()) {
                    return Ok::<Value, Error>(message);
                }
            }
        })
        .await
        .map_err(|_| failed("no answer".into()))??;
        if answer["type"].as_str() == Some("error") {
            return Err(failed(
                answer["error"].as_str().unwrap_or("refused").to_string(),
            ));
        }
        let payload = answer["payload"].clone();
        if payload["returnValue"].as_bool() == Some(false) {
            return Err(failed(
                payload["errorText"]
                    .as_str()
                    .unwrap_or("refused")
                    .to_string(),
            ));
        }
        Ok(payload)
    }

    pub(crate) async fn close(mut self) {
        let _ = self.socket.close(None).await;
    }

    async fn send(&mut self, message: Value) -> Result<()> {
        self.socket
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(|e| Error::Protocol(e.to_string()))
    }

    async fn receive(&mut self) -> Result<Value> {
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    return serde_json::from_str(&text).map_err(|e| Error::Protocol(e.to_string()));
                }
                Some(Ok(Message::Close(_))) | None => {
                    return Err(Error::Protocol("the TV hung up".into()));
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(Error::Protocol(e.to_string())),
            }
        }
    }
}

/// TLS that accepts the TV's own certificate. Signatures are still
/// checked; only who issued the certificate is not, because nobody did.
fn trusting_client() -> rustls::ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SelfSigned(provider)))
        .with_no_client_auth()
}

#[derive(Debug)]
struct SelfSigned(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for SelfSigned {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_with_a_key_once_it_has_one() {
        let first = registration(None);
        assert!(first["payload"]["client-key"].is_null());
        assert_eq!(first["payload"]["pairingType"], "PROMPT");
        let again = registration(Some("abc"));
        assert_eq!(again["payload"]["client-key"], "abc");
    }

    #[test]
    fn asks_for_power_and_notifications() {
        let permissions = registration(None)["payload"]["manifest"]["permissions"].clone();
        let permissions: Vec<&str> = permissions
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(permissions.contains(&"CONTROL_POWER"));
        assert!(permissions.contains(&"WRITE_NOTIFICATION_TOAST"));
    }
}
