//! Sonos UPnP/SOAP client.

use crate::error::{Error, Result};
use crate::transport::{HttpTransport, SonosTransport, extract_tag};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransportState {
    Playing,
    Paused,
    Stopped,
    Transitioning,
    Unknown,
}

/// Sonos SOAP/UPnP client for a single speaker (known IP).
#[derive(Clone)]
pub struct SonosClient {
    ip: String,
    transport: Arc<dyn SonosTransport>,
}

impl std::fmt::Debug for SonosClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SonosClient")
            .field("ip", &self.ip)
            .finish_non_exhaustive()
    }
}

impl SonosClient {
    /// Create a client using the default [`HttpTransport`].
    pub fn new(ip: impl Into<String>) -> Self {
        Self::with_transport(ip, Arc::new(HttpTransport::new()))
    }

    /// Create a client with a custom transport (useful for testing).
    pub fn with_transport(ip: impl Into<String>, transport: Arc<dyn SonosTransport>) -> Self {
        Self {
            ip: ip.into(),
            transport,
        }
    }

    /// Start playback on the speaker.
    pub async fn play(&self) -> Result<()> {
        self.invoke(
            av_endpoint(&self.ip),
            AV_TRANSPORT_SERVICE,
            "Play",
            "<InstanceID>0</InstanceID><Speed>1</Speed>",
        )
        .await?;
        Ok(())
    }

    /// Pause playback on the speaker.
    pub async fn pause(&self) -> Result<()> {
        self.invoke(
            av_endpoint(&self.ip),
            AV_TRANSPORT_SERVICE,
            "Pause",
            "<InstanceID>0</InstanceID>",
        )
        .await?;
        Ok(())
    }

    /// Skip to the next track.
    pub async fn next(&self) -> Result<()> {
        self.invoke(
            av_endpoint(&self.ip),
            AV_TRANSPORT_SERVICE,
            "Next",
            "<InstanceID>0</InstanceID>",
        )
        .await?;
        Ok(())
    }

    /// Skip to the previous track.
    pub async fn previous(&self) -> Result<()> {
        self.invoke(
            av_endpoint(&self.ip),
            AV_TRANSPORT_SERVICE,
            "Previous",
            "<InstanceID>0</InstanceID>",
        )
        .await?;
        Ok(())
    }

    /// Query the current transport state (Playing, Paused, etc.).
    pub async fn get_transport_state(&self) -> Result<TransportState> {
        let body = self
            .invoke(
                av_endpoint(&self.ip),
                AV_TRANSPORT_SERVICE,
                "GetTransportInfo",
                "<InstanceID>0</InstanceID>",
            )
            .await?;

        let state =
            extract_tag(&body, "CurrentTransportState").ok_or_else(|| Error::ParseResponse {
                reason: "missing <CurrentTransportState>".into(),
            })?;

        Ok(match state.as_str() {
            "PLAYING" => TransportState::Playing,
            // Some UPnP renderers return the non-canonical "PAUSED".
            "PAUSED_PLAYBACK" | "PAUSED" => TransportState::Paused,
            "STOPPED" => TransportState::Stopped,
            "TRANSITIONING" => TransportState::Transitioning,
            _ => TransportState::Unknown,
        })
    }

    /// Query the current volume level (0–100).
    pub async fn get_volume(&self) -> Result<u8> {
        let body = self
            .invoke(
                rc_endpoint(&self.ip),
                RENDERING_SERVICE,
                "GetVolume",
                "<InstanceID>0</InstanceID><Channel>Master</Channel>",
            )
            .await?;

        let raw = extract_tag(&body, "CurrentVolume").ok_or_else(|| Error::ParseResponse {
            reason: "missing <CurrentVolume>".into(),
        })?;

        let parsed: u8 = raw.parse().map_err(|_| Error::ParseResponse {
            reason: format!("invalid CurrentVolume: {raw}"),
        })?;

        if parsed > 100 {
            return Err(Error::ParseResponse {
                reason: format!("CurrentVolume out of range: {parsed}"),
            });
        }

        Ok(parsed)
    }

    /// Set the volume level. Values above 100 are clamped to 100.
    pub async fn set_volume(&self, volume: u8) -> Result<()> {
        let clamped = volume.min(100);
        self.invoke(
            rc_endpoint(&self.ip),
            RENDERING_SERVICE,
            "SetVolume",
            &format!("<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredVolume>{clamped}</DesiredVolume>"),
        )
        .await?;
        Ok(())
    }

    /// What it is playing, or last had loaded: a station, a queue, the
    /// TV, or another speaker it follows (`x-rincon:`).
    pub async fn media(&self) -> Result<Media> {
        let body = self
            .invoke(
                av_endpoint(&self.ip),
                AV_TRANSPORT_SERVICE,
                "GetMediaInfo",
                "<InstanceID>0</InstanceID>",
            )
            .await?;
        Ok(Media {
            uri: extract_tag(&body, "CurrentURI")
                .map(|u| crate::household::unescape(&u))
                .unwrap_or_default(),
            metadata: extract_tag(&body, "CurrentURIMetaData")
                .map(|m| crate::household::unescape(&m))
                .unwrap_or_default(),
        })
    }

    /// Load something to play — a station by its URI and the metadata
    /// Sonos wants beside it. Loaded, not started: [`Self::play`] next.
    pub async fn load(&self, uri: &str, metadata: &str) -> Result<()> {
        self.invoke(
            av_endpoint(&self.ip),
            AV_TRANSPORT_SERVICE,
            "SetAVTransportURI",
            &format!(
                "<InstanceID>0</InstanceID><CurrentURI>{}</CurrentURI><CurrentURIMetaData>{}</CurrentURIMetaData>",
                escape(uri),
                escape(metadata)
            ),
        )
        .await?;
        Ok(())
    }

    /// Play whatever `leader` plays, in step with it. Leaves any group
    /// it was in first, which Sonos does by itself.
    pub async fn join(&self, leader: &str) -> Result<()> {
        self.load(&format!("x-rincon:{leader}"), "").await
    }

    /// Leave the group it is in and play on its own, so it can lead.
    pub async fn go_solo(&self) -> Result<()> {
        self.invoke(
            av_endpoint(&self.ip),
            AV_TRANSPORT_SERVICE,
            "BecomeCoordinatorOfStandaloneGroup",
            "<InstanceID>0</InstanceID>",
        )
        .await?;
        Ok(())
    }

    /// Replace its queue with one thing — a playlist, an album — and
    /// load the queue. `own_id` is this speaker's `RINCON_…`, which the
    /// queue is named after.
    pub async fn load_queue(&self, own_id: &str, uri: &str, metadata: &str) -> Result<()> {
        self.invoke(
            av_endpoint(&self.ip),
            AV_TRANSPORT_SERVICE,
            "RemoveAllTracksFromQueue",
            "<InstanceID>0</InstanceID>",
        )
        .await?;
        self.invoke(
            av_endpoint(&self.ip),
            AV_TRANSPORT_SERVICE,
            "AddURIToQueue",
            &format!(
                "<InstanceID>0</InstanceID><EnqueuedURI>{}</EnqueuedURI><EnqueuedURIMetaData>{}</EnqueuedURIMetaData><DesiredFirstTrackNumberEnqueued>0</DesiredFirstTrackNumberEnqueued><EnqueueAsNext>0</EnqueueAsNext>",
                escape(uri),
                escape(metadata)
            ),
        )
        .await?;
        self.load(&format!("x-rincon-queue:{own_id}#0"), "").await
    }

    /// The household's Sonos Favorites. Shared by every speaker, so any
    /// one can be asked.
    pub async fn favorites(&self) -> Result<Vec<crate::Favorite>> {
        let body = self
            .invoke(
                format!("http://{}:1400/MediaServer/ContentDirectory/Control", self.ip),
                CONTENT_DIRECTORY_SERVICE,
                "Browse",
                "<ObjectID>FV:2</ObjectID><BrowseFlag>BrowseDirectChildren</BrowseFlag><Filter>*</Filter><StartingIndex>0</StartingIndex><RequestedCount>200</RequestedCount><SortCriteria></SortCriteria>",
            )
            .await?;
        let result = extract_tag(&body, "Result").ok_or_else(|| Error::ParseResponse {
            reason: "missing <Result>".into(),
        })?;
        Ok(crate::favorites::parse(&crate::household::unescape(
            &result,
        )))
    }

    async fn invoke(
        &self,
        endpoint: String,
        service: &str,
        action: &str,
        inner: &str,
    ) -> Result<String> {
        let body = soap_envelope(service, action, inner);
        let soap_action = format!("{service}#{action}");
        self.transport
            .send_action(&endpoint, &soap_action, &body)
            .await
    }
}

/// What a speaker has loaded, as [`SonosClient::media`] reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Media {
    pub uri: String,
    pub metadata: String,
}

impl Media {
    /// The TV, through a soundbar's HDMI or optical input.
    pub fn is_tv(&self) -> bool {
        self.uri.starts_with("x-sonos-htastream:")
    }

    /// A radio station: TuneIn, Sonos Radio, or a plain stream.
    pub fn is_radio(&self) -> bool {
        is_radio(&self.uri)
    }
}

/// Whether a URI is a radio station rather than a queue or a group.
pub fn is_radio(uri: &str) -> bool {
    [
        "x-sonosapi-stream:",
        "x-sonosapi-radio:",
        "x-rincon-mp3radio:",
        "x-sonosapi-hls:",
        "aac:",
        "hls-radio:",
    ]
    .iter()
    .any(|scheme| uri.starts_with(scheme))
}

/// Text placed inside a SOAP element.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const AV_TRANSPORT_SERVICE: &str = "urn:schemas-upnp-org:service:AVTransport:1";
const CONTENT_DIRECTORY_SERVICE: &str = "urn:schemas-upnp-org:service:ContentDirectory:1";
const RENDERING_SERVICE: &str = "urn:schemas-upnp-org:service:RenderingControl:1";

fn av_endpoint(ip: &str) -> String {
    format!("http://{ip}:1400/MediaRenderer/AVTransport/Control")
}

fn rc_endpoint(ip: &str) -> String {
    format!("http://{ip}:1400/MediaRenderer/RenderingControl/Control")
}

pub(crate) fn soap_envelope(service: &str, action: &str, inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
                    s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\
           <s:Body>\
             <u:{action} xmlns:u=\"{service}\">{inner}</u:{action}>\
           </s:Body>\
         </s:Envelope>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct MockTransport {
        last_endpoint: Arc<Mutex<Option<String>>>,
        last_action: Arc<Mutex<Option<String>>>,
        last_body: Arc<Mutex<Option<String>>>,
        response: Arc<Mutex<Option<Result<String>>>>,
    }

    impl MockTransport {
        fn new(response: Result<String>) -> Self {
            Self {
                last_endpoint: Arc::new(Mutex::new(None)),
                last_action: Arc::new(Mutex::new(None)),
                last_body: Arc::new(Mutex::new(None)),
                response: Arc::new(Mutex::new(Some(response))),
            }
        }

        fn last_endpoint(&self) -> Option<String> {
            self.last_endpoint.lock().unwrap().clone()
        }

        fn last_action(&self) -> Option<String> {
            self.last_action.lock().unwrap().clone()
        }

        fn last_body(&self) -> Option<String> {
            self.last_body.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SonosTransport for MockTransport {
        async fn send_action(
            &self,
            endpoint: &str,
            soap_action: &str,
            soap_body: &str,
        ) -> Result<String> {
            *self.last_endpoint.lock().unwrap() = Some(endpoint.to_string());
            *self.last_action.lock().unwrap() = Some(soap_action.to_string());
            *self.last_body.lock().unwrap() = Some(soap_body.to_string());
            self.response
                .lock()
                .unwrap()
                .take()
                .expect("mock called more than once")
        }
    }

    fn client_with(response: Result<String>) -> (MockTransport, SonosClient) {
        let mock = MockTransport::new(response);
        let client = SonosClient::with_transport("192.168.69.174", Arc::new(mock.clone()));
        (mock, client)
    }

    #[tokio::test]
    async fn play_sends_correct_soap() {
        let (mock, client) = client_with(Ok("".into()));
        client.play().await.unwrap();

        let endpoint = mock.last_endpoint().unwrap();
        assert!(endpoint.ends_with("/MediaRenderer/AVTransport/Control"));
        assert_eq!(
            mock.last_action().unwrap(),
            "urn:schemas-upnp-org:service:AVTransport:1#Play"
        );
        let body = mock.last_body().unwrap();
        assert!(body.contains("<u:Play"));
        assert!(body.contains("<InstanceID>0</InstanceID>"));
        assert!(body.contains("<Speed>1</Speed>"));
    }

    #[tokio::test]
    async fn pause_sends_correct_soap() {
        let (mock, client) = client_with(Ok("".into()));
        client.pause().await.unwrap();

        let endpoint = mock.last_endpoint().unwrap();
        assert!(endpoint.ends_with("/MediaRenderer/AVTransport/Control"));
        assert_eq!(
            mock.last_action().unwrap(),
            "urn:schemas-upnp-org:service:AVTransport:1#Pause"
        );
        let body = mock.last_body().unwrap();
        assert!(body.contains("<u:Pause"));
        assert!(body.contains("<InstanceID>0</InstanceID>"));
        assert!(!body.contains("<Speed>"));
    }

    #[tokio::test]
    async fn next_sends_correct_soap() {
        let (mock, client) = client_with(Ok("".into()));
        client.next().await.unwrap();

        let endpoint = mock.last_endpoint().unwrap();
        assert!(endpoint.ends_with("/MediaRenderer/AVTransport/Control"));
        assert_eq!(
            mock.last_action().unwrap(),
            "urn:schemas-upnp-org:service:AVTransport:1#Next"
        );
        let body = mock.last_body().unwrap();
        assert!(body.contains("<u:Next"));
        assert!(body.contains("<InstanceID>0</InstanceID>"));
    }

    #[tokio::test]
    async fn previous_sends_correct_soap() {
        let (mock, client) = client_with(Ok("".into()));
        client.previous().await.unwrap();

        let endpoint = mock.last_endpoint().unwrap();
        assert!(endpoint.ends_with("/MediaRenderer/AVTransport/Control"));
        assert_eq!(
            mock.last_action().unwrap(),
            "urn:schemas-upnp-org:service:AVTransport:1#Previous"
        );
        let body = mock.last_body().unwrap();
        assert!(body.contains("<u:Previous"));
        assert!(body.contains("<InstanceID>0</InstanceID>"));
    }

    #[tokio::test]
    async fn get_transport_state_parses_playing() {
        let body = "<CurrentTransportState>PLAYING</CurrentTransportState>".to_string();
        let (_, client) = client_with(Ok(body));
        assert_eq!(
            client.get_transport_state().await.unwrap(),
            TransportState::Playing
        );
    }

    #[tokio::test]
    async fn get_transport_state_parses_paused_playback() {
        let body = "<CurrentTransportState>PAUSED_PLAYBACK</CurrentTransportState>".to_string();
        let (_, client) = client_with(Ok(body));
        assert_eq!(
            client.get_transport_state().await.unwrap(),
            TransportState::Paused
        );
    }

    #[tokio::test]
    async fn get_transport_state_parses_paused_alias() {
        let body = "<CurrentTransportState>PAUSED</CurrentTransportState>".to_string();
        let (_, client) = client_with(Ok(body));
        assert_eq!(
            client.get_transport_state().await.unwrap(),
            TransportState::Paused
        );
    }

    #[tokio::test]
    async fn get_volume_parses_with_whitespace() {
        let body = "<CurrentVolume> 42 </CurrentVolume>".to_string();
        let (_, client) = client_with(Ok(body));
        assert_eq!(client.get_volume().await.unwrap(), 42);
    }

    #[tokio::test]
    async fn get_transport_state_parses_stopped() {
        let body = "<CurrentTransportState>STOPPED</CurrentTransportState>".to_string();
        let (_, client) = client_with(Ok(body));
        assert_eq!(
            client.get_transport_state().await.unwrap(),
            TransportState::Stopped
        );
    }

    #[tokio::test]
    async fn get_transport_state_parses_transitioning() {
        let body = "<CurrentTransportState>TRANSITIONING</CurrentTransportState>".to_string();
        let (_, client) = client_with(Ok(body));
        assert_eq!(
            client.get_transport_state().await.unwrap(),
            TransportState::Transitioning
        );
    }

    #[tokio::test]
    async fn get_transport_state_parses_unknown() {
        let body = "<CurrentTransportState>WUT</CurrentTransportState>".to_string();
        let (_, client) = client_with(Ok(body));
        assert_eq!(
            client.get_transport_state().await.unwrap(),
            TransportState::Unknown
        );
    }

    #[tokio::test]
    async fn get_volume_parses_42() {
        let body = "<CurrentVolume>42</CurrentVolume>".to_string();
        let (_, client) = client_with(Ok(body));
        assert_eq!(client.get_volume().await.unwrap(), 42);
    }

    #[tokio::test]
    async fn get_volume_rejects_out_of_range() {
        let body = "<CurrentVolume>200</CurrentVolume>".to_string();
        let (_, client) = client_with(Ok(body));
        let err = client.get_volume().await.unwrap_err();
        assert!(matches!(err, Error::ParseResponse { .. }));
    }

    #[tokio::test]
    async fn set_volume_clamps_to_100() {
        let (mock, client) = client_with(Ok("".into()));
        client.set_volume(150).await.unwrap();
        assert!(
            mock.last_body()
                .unwrap()
                .contains("<DesiredVolume>100</DesiredVolume>")
        );
    }

    #[tokio::test]
    async fn set_volume_zero_passes_through() {
        let (mock, client) = client_with(Ok("".into()));
        client.set_volume(0).await.unwrap();
        assert!(
            mock.last_body()
                .unwrap()
                .contains("<DesiredVolume>0</DesiredVolume>")
        );
    }

    #[tokio::test]
    async fn set_volume_normal_value_passes_through() {
        let (mock, client) = client_with(Ok("".into()));
        client.set_volume(50).await.unwrap();
        assert!(
            mock.last_body()
                .unwrap()
                .contains("<DesiredVolume>50</DesiredVolume>")
        );
    }

    #[tokio::test]
    async fn get_volume_rejects_invalid() {
        let body = "<CurrentVolume>abc</CurrentVolume>".to_string();
        let (_, client) = client_with(Ok(body));
        let err = client.get_volume().await.unwrap_err();
        assert!(matches!(err, Error::ParseResponse { .. }));
    }

    #[tokio::test]
    async fn parse_error_for_missing_volume_tag() {
        let body = "<u:GetVolumeResponse></u:GetVolumeResponse>".to_string();
        let (_, client) = client_with(Ok(body));
        let err = client.get_volume().await.unwrap_err();
        assert!(matches!(err, Error::ParseResponse { .. }));
    }

    #[tokio::test]
    async fn soap_fault_propagates_as_error() {
        let fault = Error::SoapFault {
            code: "401".into(),
            reason: "Invalid Action".into(),
        };
        let (_, client) = client_with(Err(fault));
        let err = client.play().await.unwrap_err();
        assert!(
            matches!(&err, Error::SoapFault { code, reason } if code == "401" && reason == "Invalid Action")
        );
    }

    #[tokio::test]
    async fn parse_error_for_missing_tag() {
        let body = "<u:GetTransportInfoResponse></u:GetTransportInfoResponse>".to_string();
        let (_, client) = client_with(Ok(body));
        let err = client.get_transport_state().await.unwrap_err();
        assert!(matches!(err, Error::ParseResponse { .. }));
    }
}
