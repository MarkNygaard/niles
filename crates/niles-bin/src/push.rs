//! Speaking to a satellite that isn't currently talking to us.
//!
//! Every other path here answers: the satellite connects when it hears
//! its name, streams a command, and holds the socket open long enough
//! to hear the reply. Niles has never been able to start.
//!
//! That is why a timer could fire and nothing happened. The event was
//! published, the notification centre resolved a room, and delivery
//! looked for a live connection to that room and correctly found none
//! — the satellite had hung up seconds earlier.
//!
//! So when there is no live connection, dial the satellite instead. It
//! listens on a fixed port for exactly this, and the frames are the
//! same ones the reply path already sends; only the direction of the
//! first packet is new.

use niles_wyoming::{AudioFormat, Event, EventKind, WyomingWriter};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::net::TcpStream;

/// The satellite's listening port, matching `NILES_PUSH_PORT` in the
/// firmware. Not configurable: both halves ship from this repo, and a
/// setting that must match another setting is a way to have them
/// differ.
pub const PUSH_PORT: u16 = 10301;

/// How long to wait for a satellite that may be asleep, unplugged, or
/// behind a firewall rule nobody added. Short: this runs while a timer
/// is ringing, and a notification that arrives a minute late is worse
/// than one that doesn't.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Same as the reply path uses, so the satellite's buffer sees the
/// chunk size it already copes with.
const CHUNK_BYTES: usize = 2048;

/// Say something to a satellite that isn't connected to us.
pub async fn speak_to(ip: IpAddr, pcm: &[u8], format: AudioFormat) -> anyhow::Result<()> {
    let addr = SocketAddr::new(ip, PUSH_PORT);
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow::anyhow!("{addr} did not answer within {CONNECT_TIMEOUT:?}"))?
        .map_err(|e| anyhow::anyhow!("connecting to {addr}: {e}"))?;

    let mut writer = WyomingWriter::new(stream);
    writer
        .write_event(&Event {
            kind: EventKind::AudioStart,
            data: serde_json::json!({
                "rate": format.sample_rate_hz,
                "width": format.bits_per_sample / 8,
                "channels": format.channels,
            }),
            payload: Vec::new(),
            version: None,
        })
        .await?;

    // Whole frames only: half a sample would desynchronise everything
    // after it.
    let frame = (format.bits_per_sample / 8) as usize * format.channels as usize;
    let chunk = match frame {
        0 => CHUNK_BYTES,
        f if f > CHUNK_BYTES => f,
        f => (CHUNK_BYTES / f) * f,
    };
    for part in pcm.chunks(chunk) {
        writer
            .write_event(&Event {
                kind: EventKind::AudioChunk,
                data: serde_json::Value::Null,
                payload: part.to_vec(),
                version: None,
            })
            .await?;
    }

    writer
        .write_event(&Event {
            kind: EventKind::AudioStop,
            data: serde_json::Value::Null,
            payload: Vec::new(),
            version: None,
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_satellite_that_does_not_answer_fails_rather_than_hangs() {
        // This runs while a timer is ringing. Blocking on a satellite
        // that is unplugged — or behind a firewall rule nobody added —
        // would hold the notification path open for as long as the OS
        // is willing to retry.
        //
        // 203.0.113.0/24 is reserved for documentation and routes
        // nowhere, so connecting to it stalls rather than refusing.
        let err = speak_to(
            "203.0.113.1".parse().unwrap(),
            &[0u8; 16],
            AudioFormat::new(22050, 16, 1),
        )
        .await
        .expect_err("should not succeed");
        assert!(
            err.to_string().contains("did not answer"),
            "expected a timeout, got: {err}"
        );
    }
}
