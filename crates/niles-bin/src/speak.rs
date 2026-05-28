//! Speak-back loop: synthesize text via Piper and stream the
//! resulting WAV back to a Wyoming satellite as PCM.

use anyhow::{Context, Result};
use niles_tts::PiperClient;
use niles_wyoming::{AudioFormat, WyomingSender};
use std::net::SocketAddr;

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
        let end = pos + 8 + chunk_size;

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
            fmt = Some(AudioFormat::new(sample_rate, bps, channels));
        } else if chunk_id == b"data" {
            if end > wav.len() {
                anyhow::bail!("truncated data chunk");
            }
            pcm = Some(wav[pos + 8..end].to_vec());
        }

        // Word-align chunk cursor.
        pos = end + (chunk_size & 1);
    }

    let fmt = fmt.context("missing fmt chunk")?;
    let pcm = pcm.context("missing data chunk")?;
    Ok((pcm, fmt))
}

/// Synthesize `text` via Piper, decode the returned WAV, and send
/// the PCM to `peer` through the Wyoming sender.
pub async fn speak_back(
    piper: &PiperClient,
    sender: &WyomingSender,
    peer: SocketAddr,
    text: &str,
) -> Result<()> {
    let synth = piper.synthesize(text, None).await?;
    let (pcm, format) = wav_to_pcm(&synth.audio_wav)?;
    sender.send_audio(peer, &pcm, format).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid PCM WAV in memory.
    fn make_wav(rate: u32, channels: u16, bps: u16, data: &[u8]) -> Vec<u8> {
        let data_len = data.len() as u32;
        let fmt_len = 16u32;
        let riff_len = 4 + (8 + fmt_len) + (8 + data_len);

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
        let data = vec![0x01, 0x02, 0x03];
        let mut wav = make_wav(16000, 1, 16, &data);
        // Adjust RIFF length to include padding byte.
        let new_len = (wav.len() - 8) as u32;
        wav[4..8].copy_from_slice(&new_len.to_le_bytes());
        let (pcm, fmt) = wav_to_pcm(&wav).unwrap();
        assert_eq!(pcm, data);
        assert_eq!(fmt.channels, 1);
    }
}
