//! Speak-back loop: synthesize text via Piper and stream the
//! resulting WAV back to a Wyoming satellite as PCM.

use anyhow::{Context, Result};
use niles_speakers::{SonosClient, TransportState};
use niles_tts::PiperClient;
use niles_wyoming::{AudioFormat, WyomingSender};
use std::borrow::Cow;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::satellites::SatelliteRegistry;
use crate::speakers::SpeakerRegistry;

/// How quiet a playing speaker goes while Niles listens or speaks: a
/// quarter of where it was. A fixed level did nothing in a house that
/// listens below it — the living room at 20 was "turned down" to 20,
/// and at 13 it would have been turned up, so both were left alone and
/// "stop the music" was heard through Gavin DeGraw.
fn ducked(volume: u8) -> u8 {
    volume / 4
}

/// Parse a minimal RIFF/WAVE file and return its PCM payload +
/// format metadata.
///
/// Supports PCM format only (audio format 0x0001). Extra chunks
/// are skipped.
pub fn wav_to_pcm(wav: &[u8]) -> Result<(Vec<u8>, AudioFormat)> {
    if wav.len() < 12 {
        anyhow::bail!("WAV too short");
    }
    if &wav[0..4] != b"RIFF" {
        anyhow::bail!("missing RIFF header");
    }
    if &wav[8..12] != b"WAVE" {
        anyhow::bail!("missing WAVE marker");
    }

    let mut pos = 12usize;
    let mut fmt: Option<AudioFormat> = None;
    let mut pcm: Option<Vec<u8>> = None;

    while pos + 8 <= wav.len() {
        let chunk_id = &wav[pos..pos + 4];
        let chunk_size = u32::from_le_bytes(wav[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let end = pos.checked_add(8).and_then(|p| p.checked_add(chunk_size));
        let Some(end) = end else {
            anyhow::bail!("invalid chunk size");
        };

        if chunk_id == b"fmt " {
            if chunk_size < 16 || end > wav.len() {
                anyhow::bail!("truncated fmt chunk");
            }
            let body = &wav[pos + 8..end];
            let audio_format = u16::from_le_bytes(body[0..2].try_into().unwrap());
            if audio_format != 1 {
                anyhow::bail!("unsupported audio format {audio_format}, expected PCM (1)");
            }
            let channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let bps = u16::from_le_bytes(body[14..16].try_into().unwrap());
            if channels == 0 || sample_rate == 0 || bps == 0 {
                anyhow::bail!("invalid fmt: channels={channels}, bps={bps}, rate={sample_rate}");
            }
            let width = bps / 8;
            if bps % 8 != 0 || width > 4 {
                anyhow::bail!("unsupported bits_per_sample {bps} (must be 8, 16, 24, or 32)");
            }
            fmt = Some(AudioFormat::new(sample_rate, bps, channels));
        } else if chunk_id == b"data" {
            if end > wav.len() {
                anyhow::bail!("truncated data chunk");
            }
            pcm = Some(wav[pos + 8..end].to_vec());
        }

        // Word-align chunk cursor.
        pos = end.checked_add(chunk_size & 1).unwrap_or(end);
    }

    let fmt = fmt.context("missing fmt chunk")?;
    let pcm = pcm.context("missing data chunk")?;
    let bytes_per_sample = usize::from(fmt.bits_per_sample / 8);
    let frame_size = bytes_per_sample * usize::from(fmt.channels);
    if frame_size == 0 || pcm.len() % frame_size != 0 {
        anyhow::bail!(
            "PCM data length {} is not a whole number of frames (frame_size={frame_size})",
            pcm.len()
        );
    }
    Ok((pcm, fmt))
}

/// Every Sonos in the satellite's room that is playing — music or the
/// TV — turned down for the length of the answer, with what each was at
/// to restore afterwards. A room can have several, and all of them
/// would talk over Niles.
pub(crate) async fn try_duck(
    speakers: &SpeakerRegistry,
    satellites: &SatelliteRegistry,
    peer: SocketAddr,
) -> Vec<(Arc<SonosClient>, u8)> {
    let Some(room) = satellites.room_for(peer) else {
        return Vec::new();
    };
    let mut ducked = Vec::new();
    for player in speakers.in_room(room).await {
        if let Some(original) = duck_one(&player.client).await {
            ducked.push((player.client, original));
        }
    }
    ducked
}

/// If this Sonos is playing AND its volume is above the duck level,
/// lower it and return what it was. Any failure is logged and the
/// function returns `None` — speak-back will continue without ducking.
async fn duck_one(sonos: &SonosClient) -> Option<u8> {
    match sonos.get_transport_state().await {
        Ok(TransportState::Playing) => {}
        Ok(_) => return None,
        Err(e) => {
            tracing::warn!("[duck] transport read failed: {e:#}");
            return None;
        }
    }

    let current = match sonos.get_volume().await {
        // Nearly silent already: nothing to gain.
        Ok(v) if v > 2 => v,
        Ok(_) => return None,
        Err(e) => {
            tracing::warn!("[duck] volume read failed: {e:#}");
            return None;
        }
    };

    if let Err(e) = sonos.set_volume(ducked(current)).await {
        tracing::warn!("[duck] set ducked volume failed: {e:#}");
        return None;
    }
    tracing::debug!("[duck] {current} -> {}", ducked(current));
    Some(current)
}

/// The speakers turned down while somebody talks to a satellite.
///
/// Music at full volume in the room is music in the microphone: "stop
/// the music" over Gavin DeGraw came out as "stop the wishing". The
/// satellite cancels its own echo, not the Sonos's. So the room goes
/// quiet at the wake word — before the sentence, not only under the
/// answer — and comes back once Niles has replied.
///
/// A question keeps it quiet a little longer, for the answer that does
/// not need the wake word; a turn that never finishes is let go by a
/// timer. Only a speaker still at the level Niles left it is turned
/// back up: "set the volume to 50" said meanwhile is not undone.
#[derive(Default)]
pub(crate) struct Hush {
    held: Mutex<HashMap<IpAddr, Held>>,
}

#[derive(Default)]
struct Held {
    ducked: Vec<(Arc<SonosClient>, u8)>,
    /// Which wake this is, so a timer set for an earlier one does not
    /// end a later one.
    turn: u64,
}

/// How long a turn may hold the room quiet at most.
const HUSH_AT_MOST: Duration = Duration::from_secs(30);
/// How long after a question, for its answer to start.
const HUSH_FOR_ANSWER: Duration = Duration::from_secs(12);

impl Hush {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, Held>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// At the wake word: quiet every Sonos playing in the satellite's
    /// room. Speakers already quiet from this turn's wake stay as they
    /// are; their level from before is what comes back.
    pub(crate) async fn begin(
        self: &Arc<Self>,
        speakers: &SpeakerRegistry,
        satellites: &SatelliteRegistry,
        peer: SocketAddr,
    ) {
        let ducked = try_duck(speakers, satellites, peer).await;
        let turn = {
            let mut held = self.lock();
            let entry = held.entry(peer.ip()).or_default();
            entry.ducked.extend(ducked);
            entry.turn += 1;
            entry.turn
        };
        self.let_go_after(peer.ip(), turn, HUSH_AT_MOST);
    }

    /// The turn is answered. With a question, the room stays quiet a
    /// little longer for the answer, which begins a turn of its own.
    pub(crate) async fn finish(self: &Arc<Self>, peer: SocketAddr, asked: bool) {
        if asked {
            let turn = self.lock().get(&peer.ip()).map(|h| h.turn);
            if let Some(turn) = turn {
                self.let_go_after(peer.ip(), turn, HUSH_FOR_ANSWER);
            }
        } else {
            self.restore(peer.ip(), None).await;
        }
    }

    fn let_go_after(self: &Arc<Self>, ip: IpAddr, turn: u64, after: Duration) {
        let hush = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            hush.restore(ip, Some(turn)).await;
        });
    }

    /// Turn back up what this satellite's turn turned down — only for
    /// `turn`, when given, so a later wake keeps its quiet.
    async fn restore(&self, ip: IpAddr, turn: Option<u64>) {
        let turned_down = {
            let mut held = self.lock();
            match held.get(&ip) {
                Some(h) if turn.is_none_or(|t| t == h.turn) => {
                    held.remove(&ip).map(|h| h.ducked).unwrap_or_default()
                }
                _ => return,
            }
        };
        for (sonos, original) in turned_down {
            match sonos.get_volume().await {
                // Changed meanwhile — by a command, or a hand on the app.
                Ok(now) if now != ducked(original) => continue,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("[hush] volume read failed: {e:#}");
                    continue;
                }
            }
            if let Err(e) = sonos.set_volume(original).await {
                tracing::warn!("[hush] restore failed: {e:#}");
            }
        }
    }
}

/// Synthesize `text` via Piper, decode the returned WAV, and send
/// the PCM to `peer` through the Wyoming sender.
/// The same audio, quieter.
///
/// The satellite has no volume control — no button on the board, and
/// nothing in its firmware Niles can reach. But Niles makes this
/// audio, so the place to make it quieter is before it leaves.
///
/// Borrowed unchanged at 100, which is both the default and the common
/// case: the ordinary path should not copy a buffer to multiply it by
/// one.
///
/// Only 16-bit samples are scaled. Piper renders 16-bit and always
/// has; anything else is left alone rather than reinterpreted, because
/// treating 8- or 24-bit audio as `i16` would not be quiet, it would
/// be noise.
pub(crate) fn at_volume(pcm: &[u8], bits_per_sample: u16, percent: u8) -> Cow<'_, [u8]> {
    if percent >= 100 || bits_per_sample != 16 {
        if percent < 100 {
            tracing::warn!(
                "not scaling {bits_per_sample}-bit audio; volume only applies to 16-bit"
            );
        }
        return Cow::Borrowed(pcm);
    }
    // Rounded through i32 so the quietest samples do not all collapse
    // to zero, which is what makes a scaled-down voice sound gritty
    // rather than simply softer.
    let scaled: Vec<u8> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .flat_map(|s| {
            let sample = i16::from_le_bytes(*s);
            let quieter = (i32::from(sample) * i32::from(percent) / 100) as i16;
            quieter.to_le_bytes()
        })
        .collect();
    Cow::Owned(scaled)
}

/// The rate the satellite plays at: its whole I2S bus.
///
/// It listens while it talks, so its microphone and speaker share one
/// clock. That clock used to be the microphone's 16 kHz, which cut off
/// everything above 8 kHz — the "s" and "t" of every word. Since the
/// XVF3800 runs its 48 kHz firmware and drives the clock itself, the bus is
/// 48 kHz and the satellite decimates the microphone on its own side.
pub(crate) const SATELLITE_RATE: u32 = 48_000;

/// Speech as the satellite should be handed it: at the bus rate.
///
/// Piper renders 22.05 kHz. The satellite would otherwise convert it by
/// joining the dots between samples, which images and aliases; done here
/// with a windowed sinc, it arrives ready to play.
///
/// Mono 16-bit only, which is what Piper produces; anything else is left
/// alone for the satellite to deal with as before.
pub(crate) fn for_satellite(pcm: Vec<u8>, format: AudioFormat) -> (Vec<u8>, AudioFormat) {
    if format.sample_rate_hz == SATELLITE_RATE
        || format.bits_per_sample != 16
        || format.channels != 1
    {
        return (pcm, format);
    }
    let input: Vec<f32> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|s| f32::from(i16::from_le_bytes(*s)))
        .collect();
    let output = resample(&input, format.sample_rate_hz, SATELLITE_RATE);
    let bytes = output
        .iter()
        .flat_map(|s| (s.round().clamp(-32768.0, 32767.0) as i16).to_le_bytes())
        .collect();
    (bytes, AudioFormat::new(SATELLITE_RATE, 16, 1))
}

/// Windowed-sinc resampling, computed per output sample.
///
/// A few seconds of speech is ~50 000 output samples at 48 taps each:
/// nothing next to the synthesis that produced it, so no polyphase table.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    // Half the filter, in input samples. Long enough for a steep edge
    // just under the new Nyquist; short enough to be free.
    const HALF: i64 = 24;
    let ratio = f64::from(from) / f64::from(to);
    // Below the output's Nyquist, with room for the filter's slope.
    let cutoff = 0.92 * (f64::from(to) / f64::from(from)).min(1.0);
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let sinc = |x: f64| {
        if x.abs() < 1e-9 {
            1.0
        } else {
            (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
        }
    };
    // Blackman, over the filter's whole width.
    let window = |x: f64| {
        let n = (x / HALF as f64 + 1.0) / 2.0;
        0.42 - 0.5 * (2.0 * std::f64::consts::PI * n).cos()
            + 0.08 * (4.0 * std::f64::consts::PI * n).cos()
    };
    (0..out_len)
        .map(|n| {
            let centre = n as f64 * ratio;
            let first = centre.floor() as i64 - HALF + 1;
            let (mut sum, mut weight) = (0.0f64, 0.0f64);
            for k in first..first + 2 * HALF {
                let x = centre - k as f64;
                if x.abs() >= HALF as f64 {
                    continue;
                }
                let w = cutoff * sinc(cutoff * x) * window(x);
                if let Some(&s) = usize::try_from(k).ok().and_then(|k| input.get(k)) {
                    sum += f64::from(s) * w;
                }
                weight += w;
            }
            // Normalised by the weights actually used, so the edges of the
            // clip — where the filter hangs off the end — keep their level.
            (if weight.abs() > 1e-9 {
                sum / weight
            } else {
                0.0
            }) as f32
        })
        .collect()
}

pub async fn speak_back(
    piper: &PiperClient,
    sender: &WyomingSender,
    peer: SocketAddr,
    text: &str,
    speakers: &SpeakerRegistry,
    satellites: &SatelliteRegistry,
    // A question: ask the satellite to listen for the answer afterwards.
    listen: bool,
) -> Result<()> {
    let duck_handle = try_duck(speakers, satellites, peer).await;
    let result: Result<()> = async {
        let synth = piper.synthesize(text, None).await?;
        let (pcm, format) = wav_to_pcm(&synth.audio_wav)?;
        let (pcm, format) = for_satellite(pcm, format);
        let quieted = at_volume(&pcm, format.bits_per_sample, satellites.volume_for(peer));
        sender
            .send_audio_then(peer, &quieted, format, listen)
            .await?;
        Ok(())
    }
    .await;

    for (sonos, original) in duck_handle {
        if let Err(e) = sonos.set_volume(original).await {
            tracing::warn!("[{peer}] duck restore failed: {e:#}");
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, rate: u32, seconds: f64) -> Vec<f32> {
        (0..(f64::from(rate) * seconds) as usize)
            .map(|i| {
                (10_000.0 * (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin())
                    as f32
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        // The middle only: the ends are where the filter runs off the clip.
        let mid = &x[x.len() / 4..x.len() * 3 / 4];
        (mid.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / mid.len() as f64).sqrt()
    }

    #[test]
    fn a_second_of_speech_is_a_second_at_the_new_rate() {
        assert_eq!(resample(&vec![0.0; 22_050], 22_050, 16_000).len(), 16_000);
        assert_eq!(resample(&vec![0.0; 22_050], 22_050, 48_000).len(), 48_000);
    }

    #[test]
    fn a_voice_comes_through_at_its_own_level() {
        let input = tone(1_000.0, 22_050, 1.0);
        let output = resample(&input, 22_050, 16_000);
        let ratio = rms(&output) / rms(&input);
        assert!((0.97..1.03).contains(&ratio), "1 kHz kept at {ratio:.3}");
    }

    #[test]
    fn treble_the_satellite_cannot_play_is_removed_not_folded_back() {
        // 9.5 kHz would fold to 6.5 kHz at 16 kHz: a rasp where a hiss was.
        let output = resample(&tone(9_500.0, 22_050, 1.0), 22_050, 16_000);
        let left = rms(&output) / rms(&tone(9_500.0, 22_050, 1.0));
        assert!(left < 0.02, "9.5 kHz left at {left:.3}");
    }

    #[test]
    fn audio_already_at_the_bus_rate_is_untouched() {
        let pcm = vec![1u8, 0, 2, 0];
        let (out, fmt) = for_satellite(pcm.clone(), AudioFormat::new(48_000, 16, 1));
        assert_eq!(out, pcm);
        assert_eq!(fmt.sample_rate_hz, 48_000);
    }

    #[test]
    fn piper_is_handed_over_at_48k() {
        let pcm: Vec<u8> = vec![0u8; 22_050 * 2];
        let (out, fmt) = for_satellite(pcm, AudioFormat::new(22_050, 16, 1));
        assert_eq!(fmt.sample_rate_hz, 48_000);
        assert_eq!(out.len(), 48_000 * 2);
    }

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn samples(bytes: &[u8]) -> Vec<i16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect()
    }

    #[test]
    fn full_volume_does_not_touch_the_audio() {
        // The common case and the default. Multiplying a buffer by one
        // should not copy it.
        let audio = pcm(&[1000, -1000, 32767]);
        let out = at_volume(&audio, 16, 100);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out.as_ref(), audio.as_slice());
    }

    #[test]
    fn half_volume_halves_every_sample() {
        let audio = pcm(&[1000, -1000, 32766, 0]);
        let out = at_volume(&audio, 16, 50);
        assert_eq!(samples(&out), vec![500, -500, 16383, 0]);
    }

    #[test]
    fn silence_is_a_volume_somebody_may_want() {
        let audio = pcm(&[32767, -32768]);
        let out = at_volume(&audio, 16, 0);
        assert_eq!(samples(&out), vec![0, 0]);
    }

    #[test]
    fn a_quiet_sample_does_not_become_a_loud_one() {
        // The failure worth guarding: scaling a negative sample through
        // an unsigned or narrower type wraps it to the opposite
        // extreme, which is not quiet audio but a click.
        let audio = pcm(&[-32768, -30000]);
        let out = at_volume(&audio, 16, 10);
        let got = samples(&out);
        assert!(got.iter().all(|s| *s < 0), "{got:?}");
        assert_eq!(got, vec![-3276, -3000]);
    }

    #[test]
    fn audio_that_is_not_16_bit_is_left_alone() {
        // Reinterpreting 8- or 24-bit samples as i16 would not be
        // quieter, it would be noise.
        let audio = pcm(&[1000, -1000]);
        let out = at_volume(&audio, 24, 50);
        assert_eq!(out.as_ref(), audio.as_slice());
    }

    /// Build a minimal valid PCM WAV in memory.
    fn make_wav(rate: u32, channels: u16, bps: u16, data: &[u8]) -> Vec<u8> {
        let data_len = data.len() as u32;
        let fmt_len = 16u32;
        // RIFF length covers everything after the first 8 bytes,
        // including the padding byte for odd-sized chunks.
        let riff_len = 4 + (8 + fmt_len) + (8 + data_len + (data_len & 1));

        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&riff_len.to_le_bytes());
        out.extend_from_slice(b"WAVE");

        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&fmt_len.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * channels as u32 * bps as u32 / 8).to_le_bytes());
        out.extend_from_slice(&(channels * bps / 8).to_le_bytes());
        out.extend_from_slice(&bps.to_le_bytes());

        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.extend_from_slice(data);
        // Pad to word boundary if data length is odd.
        if data.len() % 2 == 1 {
            out.push(0);
        }

        out
    }

    #[test]
    fn valid_mono() {
        let data = vec![0x01, 0x02, 0x03, 0x04];
        let wav = make_wav(16000, 1, 16, &data);
        let (pcm, fmt) = wav_to_pcm(&wav).unwrap();
        assert_eq!(pcm, data);
        assert_eq!(fmt.sample_rate_hz, 16000);
        assert_eq!(fmt.bits_per_sample, 16);
        assert_eq!(fmt.channels, 1);
    }

    #[test]
    fn valid_stereo() {
        let data = vec![0xAB; 8];
        let wav = make_wav(44100, 2, 16, &data);
        let (pcm, fmt) = wav_to_pcm(&wav).unwrap();
        assert_eq!(pcm, data);
        assert_eq!(fmt.sample_rate_hz, 44100);
        assert_eq!(fmt.bits_per_sample, 16);
        assert_eq!(fmt.channels, 2);
    }

    #[test]
    fn non_riff() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        wav[0] = b'X';
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn missing_fmt() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        // Zero out the "fmt " id so it's treated as an unknown chunk.
        wav[12] = b'X';
        wav[13] = b'X';
        wav[14] = b'X';
        wav[15] = b'X';
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn missing_data() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        // Zero out the "data" id.
        let data_offset = 12 + 8 + 16;
        wav[data_offset] = b'X';
        wav[data_offset + 1] = b'X';
        wav[data_offset + 2] = b'X';
        wav[data_offset + 3] = b'X';
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn non_pcm_format() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        // Change audio format at offset 20 from 1 to 3 (IEEE float).
        wav[20] = 3;
        wav[21] = 0;
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn extra_chunk() {
        let data = vec![0u8; 4];
        let fmt_len = 16u32;
        let data_len = data.len() as u32;
        let list_len = 4u32;
        let riff_len = 4 + (8 + fmt_len) + (8 + list_len) + (8 + data_len);

        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&riff_len.to_le_bytes());
        out.extend_from_slice(b"WAVE");

        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&fmt_len.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // channels
        out.extend_from_slice(&16000u32.to_le_bytes());
        out.extend_from_slice(&(16000u32 * 16 / 8).to_le_bytes());
        out.extend_from_slice(&(2u16).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());

        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&list_len.to_le_bytes());
        out.extend_from_slice(b"xxxx");

        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.extend_from_slice(&data);

        let (pcm, fmt) = wav_to_pcm(&out).unwrap();
        assert_eq!(pcm, data);
        assert_eq!(fmt.sample_rate_hz, 16000);
    }

    #[test]
    fn missing_wave_marker() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        wav[8] = b'X';
        wav[9] = b'X';
        wav[10] = b'X';
        wav[11] = b'X';
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn zero_channels() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        wav[22] = 0;
        wav[23] = 0;
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn zero_sample_rate() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        wav[24] = 0;
        wav[25] = 0;
        wav[26] = 0;
        wav[27] = 0;
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn zero_bits_per_sample() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        wav[34] = 0;
        wav[35] = 0;
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn non_multiple_of_8_bps() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        wav[34] = 12; // 12 bits per sample
        wav[35] = 0;
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn data_not_whole_frames() {
        // 16-bit mono requires 2 bytes/frame; 3 bytes is malformed.
        let wav = make_wav(16000, 1, 16, &[0x01, 0x02, 0x03]);
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn bps_too_large() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        wav[34] = 64; // 64 bits per sample
        wav[35] = 0;
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn truncated_fmt_chunk() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        // Set fmt chunk size to 15 (needs at least 16 for PCM fmt).
        wav[16] = 15;
        wav[17] = 0;
        wav[18] = 0;
        wav[19] = 0;
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn truncated_data_chunk() {
        let mut wav = make_wav(16000, 1, 16, &[0u8; 4]);
        // Claim data is longer than it is.
        let data_offset = 12 + 8 + 16 + 4;
        wav[data_offset] = 0xFF;
        wav[data_offset + 1] = 0xFF;
        wav[data_offset + 2] = 0x00;
        wav[data_offset + 3] = 0x00;
        assert!(wav_to_pcm(&wav).is_err());
    }

    #[test]
    fn odd_sized_data() {
        // 8-bit mono: frame_size=1, so 3 bytes = 3 complete frames.
        // The RIFF container pads odd-sized chunks; that pad byte must
        // not appear in the returned PCM slice.
        let data = vec![0x01, 0x02, 0x03];
        let wav = make_wav(16000, 1, 8, &data);
        let (pcm, fmt) = wav_to_pcm(&wav).unwrap();
        assert_eq!(pcm, data);
        assert_eq!(fmt.channels, 1);
    }

    // -----------------------------------------------------------------
    // RecordingTransport fixture + try_duck tests
    // -----------------------------------------------------------------

    use async_trait::async_trait;
    use niles_core::RoomName;
    use niles_speakers::{Error as SonosError, SonosTransport};
    use std::collections::VecDeque;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Mutex;

    #[derive(Clone)]
    struct RecordingTransport {
        calls: Arc<Mutex<Vec<(String, String, String)>>>,
        responses: Arc<Mutex<VecDeque<Result<String, SonosError>>>>,
    }

    impl RecordingTransport {
        fn with_responses(responses: Vec<Result<String, SonosError>>) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            }
        }

        fn calls(&self) -> Vec<(String, String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SonosTransport for RecordingTransport {
        async fn send_action(
            &self,
            endpoint: &str,
            soap_action: &str,
            soap_body: &str,
        ) -> Result<String, SonosError> {
            self.calls.lock().unwrap().push((
                endpoint.to_string(),
                soap_action.to_string(),
                soap_body.to_string(),
            ));
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("RecordingTransport ran out of scripted responses")
        }
    }

    fn xml_transport_info(state: &str) -> String {
        format!("<CurrentTransportState>{state}</CurrentTransportState>")
    }

    fn xml_volume(v: u8) -> String {
        format!("<CurrentVolume>{v}</CurrentVolume>")
    }

    fn on(transport: RecordingTransport) -> SonosClient {
        SonosClient::with_transport("0.0.0.0", Arc::new(transport))
    }

    /// The household as Sonos describes it: the soundbar and a speaker
    /// at the back, both placed in the living room by `PLACED`.
    fn xml_household() -> String {
        let state = r#"<ZoneGroupState><ZoneGroups><ZoneGroup Coordinator="RINCON_BAR" ID="g"><ZoneGroupMember UUID="RINCON_BAR" Location="http://10.0.0.2:1400/x.xml" ZoneName="Living Room"/><ZoneGroupMember UUID="RINCON_BACK" Location="http://10.0.0.6:1400/x.xml" ZoneName="Living Room Back"/></ZoneGroup></ZoneGroups></ZoneGroupState>"#;
        let escaped = state
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        format!("<ZoneGroupState>{escaped}</ZoneGroupState>")
    }

    const PLACED: &str = r#"
[speakers]
host = "10.0.0.2"
[speakers.sonos.RINCON_BAR]
room = "living_room"
[speakers.sonos.RINCON_BACK]
room = "living_room"
"#;

    fn registry(toml: &str, transport: RecordingTransport) -> SpeakerRegistry {
        let config = niles_config::ConfigStore::from_str_in_memory(toml).unwrap();
        SpeakerRegistry::with_transport(Arc::new(config), Arc::new(transport))
    }

    fn make_satellite_registry(ip: IpAddr, room: RoomName) -> SatelliteRegistry {
        let mut reg = SatelliteRegistry::default();
        reg.by_ip
            .insert(ip, crate::satellites::Satellite { room, volume: 100 });
        reg
    }

    fn test_peer() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1234)
    }

    #[tokio::test]
    async fn duck_playing_above_threshold_lowers_volume_and_returns_original() {
        let mock = RecordingTransport::with_responses(vec![
            Ok(xml_transport_info("PLAYING")),
            Ok(xml_volume(60)),
            Ok(String::new()),
        ]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_some());
        let original = result.unwrap();
        assert_eq!(original, 60);

        let calls = mock.calls();
        assert_eq!(calls.len(), 3);
        assert!(calls[0].1.contains("GetTransportInfo"));
        assert!(calls[1].1.contains("GetVolume"));
        assert!(calls[2].1.contains("SetVolume"));
        assert!(calls[2].2.contains("<DesiredVolume>15</DesiredVolume>"));
    }

    #[tokio::test]
    async fn duck_paused_returns_none_after_one_call() {
        let mock =
            RecordingTransport::with_responses(vec![Ok(xml_transport_info("PAUSED_PLAYBACK"))]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_none());
        assert_eq!(mock.calls().len(), 1);
    }

    #[tokio::test]
    async fn duck_stopped_returns_none() {
        let mock = RecordingTransport::with_responses(vec![Ok(xml_transport_info("STOPPED"))]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_none());
        assert_eq!(mock.calls().len(), 1);
    }

    #[tokio::test]
    async fn duck_transitioning_returns_none() {
        let mock =
            RecordingTransport::with_responses(vec![Ok(xml_transport_info("TRANSITIONING"))]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_none());
        assert_eq!(mock.calls().len(), 1);
    }

    #[tokio::test]
    async fn duck_unknown_returns_none() {
        let mock = RecordingTransport::with_responses(vec![Ok(xml_transport_info("UNKNOWN"))]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_none());
        assert_eq!(mock.calls().len(), 1);
    }

    #[test]
    fn a_quiet_room_still_goes_quieter() {
        // The living room listened at 20, then 13; a fixed 20 left both.
        assert_eq!(ducked(20), 5);
        assert_eq!(ducked(13), 3);
        assert_eq!(ducked(60), 15);
    }

    #[tokio::test]
    async fn duck_a_speaker_nearly_silent_returns_none() {
        let mock = RecordingTransport::with_responses(vec![
            Ok(xml_transport_info("PLAYING")),
            Ok(xml_volume(2)),
        ]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_none());
        assert_eq!(mock.calls().len(), 2);
    }

    #[tokio::test]
    async fn duck_a_silent_speaker_returns_none() {
        let mock = RecordingTransport::with_responses(vec![
            Ok(xml_transport_info("PLAYING")),
            Ok(xml_volume(0)),
        ]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_none());
        assert_eq!(mock.calls().len(), 2);
    }

    #[tokio::test]
    async fn duck_transport_read_err_returns_none() {
        let mock = RecordingTransport::with_responses(vec![Err(SonosError::SoapFault {
            code: "500".into(),
            reason: "Internal Server Error".into(),
        })]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_none());
        assert_eq!(mock.calls().len(), 1);
    }

    #[tokio::test]
    async fn duck_volume_read_err_returns_none() {
        let mock = RecordingTransport::with_responses(vec![
            Ok(xml_transport_info("PLAYING")),
            Err(SonosError::SoapFault {
                code: "500".into(),
                reason: "Internal Server Error".into(),
            }),
        ]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_none());
        assert_eq!(mock.calls().len(), 2);
    }

    #[tokio::test]
    async fn duck_set_volume_err_returns_none() {
        let mock = RecordingTransport::with_responses(vec![
            Ok(xml_transport_info("PLAYING")),
            Ok(xml_volume(60)),
            Err(SonosError::SoapFault {
                code: "500".into(),
                reason: "Internal Server Error".into(),
            }),
        ]);

        let result = duck_one(&on(mock.clone())).await;
        assert!(result.is_none());
        // Only 3 calls: no restore attempted.
        assert_eq!(mock.calls().len(), 3);
    }

    #[tokio::test]
    async fn duck_turns_down_every_playing_sonos_in_the_room() {
        // The soundbar plays the TV; the speaker at the back is paused.
        let peer = test_peer();
        let room = RoomName::parse("living_room").unwrap();
        let mock = RecordingTransport::with_responses(vec![
            Ok(xml_household()),
            Ok(xml_transport_info("PLAYING")),
            Ok(xml_volume(45)),
            Ok(String::new()),
            Ok(xml_transport_info("PAUSED_PLAYBACK")),
        ]);
        let speakers = registry(PLACED, mock.clone());
        let satellites = make_satellite_registry(peer.ip(), room);

        let ducked = try_duck(&speakers, &satellites, peer).await;
        assert_eq!(ducked.len(), 1);
        assert_eq!(ducked[0].1, 45);
        let calls = mock.calls();
        assert!(calls[3].0.contains("10.0.0.2"), "{calls:?}");
        assert!(calls[4].0.contains("10.0.0.6"), "{calls:?}");
    }

    #[tokio::test]
    async fn duck_nothing_placed_in_the_room() {
        let peer = test_peer();
        let mock = RecordingTransport::with_responses(vec![]);
        let speakers = registry(PLACED, mock.clone());
        let satellites = make_satellite_registry(peer.ip(), RoomName::parse("kitchen").unwrap());

        assert!(try_duck(&speakers, &satellites, peer).await.is_empty());
        assert_eq!(mock.calls().len(), 0);
    }

    #[tokio::test]
    async fn duck_no_satellite_mapping() {
        let peer = test_peer();
        let mock = RecordingTransport::with_responses(vec![]);
        let speakers = registry(PLACED, mock.clone());
        let satellites = SatelliteRegistry::default();

        assert!(try_duck(&speakers, &satellites, peer).await.is_empty());
        assert_eq!(mock.calls().len(), 0);
    }

    fn one_speaker_playing() -> Vec<Result<String, SonosError>> {
        // The household, then the soundbar playing at 60 and the speaker
        // at the back paused.
        vec![
            Ok(xml_household()),
            Ok(xml_transport_info("PLAYING")),
            Ok(xml_volume(60)),
            Ok(String::new()),
            Ok(xml_transport_info("PAUSED_PLAYBACK")),
        ]
    }

    #[tokio::test]
    async fn the_room_goes_quiet_at_the_wake_and_comes_back_after() {
        let peer = test_peer();
        let room = RoomName::parse("living_room").unwrap();
        let mut responses = one_speaker_playing();
        responses.extend([Ok(xml_volume(15)), Ok(String::new())]);
        let mock = RecordingTransport::with_responses(responses);
        let speakers = registry(PLACED, mock.clone());
        let satellites = make_satellite_registry(peer.ip(), room);
        let hush = Arc::new(Hush::default());

        hush.begin(&speakers, &satellites, peer).await;
        assert!(
            mock.calls()[3]
                .2
                .contains("<DesiredVolume>15</DesiredVolume>")
        );
        hush.finish(peer, false).await;
        let calls = mock.calls();
        let last = calls.last().unwrap();
        assert!(
            last.1.contains("SetVolume") && last.2.contains("<DesiredVolume>60</DesiredVolume>")
        );
    }

    #[tokio::test]
    async fn a_volume_set_meanwhile_is_not_undone() {
        let peer = test_peer();
        let room = RoomName::parse("living_room").unwrap();
        let mut responses = one_speaker_playing();
        // "Set the volume to 50" happened during the turn.
        responses.push(Ok(xml_volume(50)));
        let mock = RecordingTransport::with_responses(responses);
        let speakers = registry(PLACED, mock.clone());
        let satellites = make_satellite_registry(peer.ip(), room);
        let hush = Arc::new(Hush::default());

        hush.begin(&speakers, &satellites, peer).await;
        hush.finish(peer, false).await;
        let sets = mock
            .calls()
            .iter()
            .filter(|c| c.1.contains("SetVolume"))
            .count();
        assert_eq!(sets, 1, "only the duck, no restore");
    }

    #[tokio::test(start_paused = true)]
    async fn a_question_keeps_the_room_quiet_for_the_answer() {
        let peer = test_peer();
        let room = RoomName::parse("living_room").unwrap();
        let mut responses = one_speaker_playing();
        responses.extend([Ok(xml_volume(15)), Ok(String::new())]);
        let mock = RecordingTransport::with_responses(responses);
        let speakers = registry(PLACED, mock.clone());
        let satellites = make_satellite_registry(peer.ip(), room);
        let hush = Arc::new(Hush::default());

        hush.begin(&speakers, &satellites, peer).await;
        let before = mock.calls().len();
        hush.finish(peer, true).await;
        assert_eq!(mock.calls().len(), before, "still quiet");
        // No answer came: let go after the grace period.
        tokio::time::sleep(HUSH_FOR_ANSWER + Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        let calls = mock.calls();
        assert!(
            calls
                .last()
                .unwrap()
                .2
                .contains("<DesiredVolume>60</DesiredVolume>"),
            "{calls:?}"
        );
    }
}
