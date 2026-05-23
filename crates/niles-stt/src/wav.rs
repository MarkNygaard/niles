//! Minimal PCM-to-WAV header encoder.
//!
//! Wyoming `audio-chunk` events carry raw PCM bytes with sample-rate,
//! sample-width, and channel-count declared in the preceding
//! `audio-start`. Groq's transcription endpoint expects a complete
//! container (WAV/MP3/FLAC/...) — wrapping the PCM in a standard
//! RIFF/WAVE header is the cheapest way to satisfy that.

use crate::error::{Error, Result};

/// Description of a raw PCM buffer's format. ESPHome's
/// voice_assistant component currently sends 16-bit signed
/// little-endian, mono, 16 kHz; the struct carries the others so we
/// don't have to revisit when a different satellite shows up.
#[derive(Debug, Clone, Copy)]
pub struct PcmFormat {
    pub sample_rate_hz: u32,
    pub bits_per_sample: u16,
    pub channels: u16,
}

/// Wrap raw PCM samples in a standard 44-byte RIFF/WAVE header.
/// Output layout is `[header || samples]`. The header declares
/// `audio format = 1` (uncompressed PCM); compressed formats are out
/// of scope.
pub fn pcm_to_wav(pcm: &[u8], fmt: PcmFormat) -> Result<Vec<u8>> {
    if fmt.channels == 0 || fmt.sample_rate_hz == 0 || fmt.bits_per_sample == 0 {
        return Err(Error::Wav {
            reason: format!(
                "invalid format: rate={}, bits={}, channels={}",
                fmt.sample_rate_hz, fmt.bits_per_sample, fmt.channels
            ),
        });
    }
    if !fmt.bits_per_sample.is_multiple_of(8) {
        return Err(Error::Wav {
            reason: format!(
                "bits_per_sample {} is not a multiple of 8",
                fmt.bits_per_sample
            ),
        });
    }
    let bytes_per_sample = fmt.bits_per_sample / 8;
    let block_align = fmt
        .channels
        .checked_mul(bytes_per_sample)
        .ok_or_else(|| Error::Wav {
            reason: "block_align overflows u16".into(),
        })?;
    let byte_rate = fmt
        .sample_rate_hz
        .checked_mul(block_align as u32)
        .ok_or_else(|| Error::Wav {
            reason: "byte_rate overflows u32".into(),
        })?;
    let data_size: u32 = pcm.len().try_into().map_err(|_| Error::Wav {
        reason: format!("payload too large for 32-bit WAV: {} bytes", pcm.len()),
    })?;
    let riff_size = 36u32.checked_add(data_size).ok_or_else(|| Error::Wav {
        reason: "RIFF chunk size overflows u32".into(),
    })?;

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM subchunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
    out.extend_from_slice(&fmt.channels.to_le_bytes());
    out.extend_from_slice(&fmt.sample_rate_hz.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&fmt.bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend_from_slice(pcm);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32_le(buf: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
    }
    fn read_u16_le(buf: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(buf[at..at + 2].try_into().unwrap())
    }

    #[test]
    fn header_for_16k_mono_16bit_matches_canonical_layout() {
        let pcm = vec![0u8; 32_000]; // 1s of 16k mono 16-bit
        let fmt = PcmFormat {
            sample_rate_hz: 16_000,
            bits_per_sample: 16,
            channels: 1,
        };
        let wav = pcm_to_wav(&pcm, fmt).unwrap();

        assert_eq!(wav.len(), 44 + pcm.len());
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");

        // RIFF chunk size = 36 + data_size
        assert_eq!(read_u32_le(&wav, 4), 36 + 32_000);
        // PCM subchunk size = 16
        assert_eq!(read_u32_le(&wav, 16), 16);
        // audio format = 1 (PCM)
        assert_eq!(read_u16_le(&wav, 20), 1);
        // channels = 1
        assert_eq!(read_u16_le(&wav, 22), 1);
        // sample rate = 16000
        assert_eq!(read_u32_le(&wav, 24), 16_000);
        // byte rate = 32000 (16000 * 1 * 2)
        assert_eq!(read_u32_le(&wav, 28), 32_000);
        // block align = 2
        assert_eq!(read_u16_le(&wav, 32), 2);
        // bits per sample = 16
        assert_eq!(read_u16_le(&wav, 34), 16);
        // data size = 32000
        assert_eq!(read_u32_le(&wav, 40), 32_000);
    }

    #[test]
    fn header_for_48k_stereo_24bit_has_correct_derived_fields() {
        let pcm = vec![0u8; 600]; // ignored content, just a sized buffer
        let fmt = PcmFormat {
            sample_rate_hz: 48_000,
            bits_per_sample: 24,
            channels: 2,
        };
        let wav = pcm_to_wav(&pcm, fmt).unwrap();
        // block_align = channels (2) * bytes_per_sample (3) = 6
        assert_eq!(read_u16_le(&wav, 32), 6);
        // byte_rate = 48000 * 6 = 288_000
        assert_eq!(read_u32_le(&wav, 28), 288_000);
    }

    #[test]
    fn empty_pcm_still_produces_valid_header() {
        let wav = pcm_to_wav(
            &[],
            PcmFormat {
                sample_rate_hz: 16_000,
                bits_per_sample: 16,
                channels: 1,
            },
        )
        .unwrap();
        assert_eq!(wav.len(), 44);
        assert_eq!(read_u32_le(&wav, 40), 0); // data size
        assert_eq!(read_u32_le(&wav, 4), 36); // RIFF size
    }

    #[test]
    fn rejects_zeroed_format_fields() {
        let pcm = vec![0u8; 100];
        assert!(
            pcm_to_wav(
                &pcm,
                PcmFormat {
                    sample_rate_hz: 0,
                    bits_per_sample: 16,
                    channels: 1
                }
            )
            .is_err()
        );
        assert!(
            pcm_to_wav(
                &pcm,
                PcmFormat {
                    sample_rate_hz: 16_000,
                    bits_per_sample: 0,
                    channels: 1
                }
            )
            .is_err()
        );
        assert!(
            pcm_to_wav(
                &pcm,
                PcmFormat {
                    sample_rate_hz: 16_000,
                    bits_per_sample: 16,
                    channels: 0
                }
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_non_byte_aligned_sample_width() {
        let pcm = vec![0u8; 100];
        let err = pcm_to_wav(
            &pcm,
            PcmFormat {
                sample_rate_hz: 16_000,
                bits_per_sample: 17,
                channels: 1,
            },
        );
        assert!(err.is_err());
    }
}
