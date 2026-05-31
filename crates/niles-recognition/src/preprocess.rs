//! Log-mel filterbank preprocessing for ECAPA-TDNN Path-B models.

use ndarray::Array2;
use realfft::RealFftPlanner;
use std::sync::OnceLock;

const SAMPLE_RATE: u32 = 16_000;
const WIN_LEN: usize = 400; // 25 ms × 16 kHz
const HOP: usize = 160; // 10 ms × 16 kHz
const N_FFT: usize = 512; // next power of two ≥ WIN_LEN
pub const N_MEL: usize = 80;
const MEL_FMIN: f32 = 0.0;
const MEL_FMAX: f32 = 8_000.0;

/// Convert raw f32 PCM (range [-1.0, 1.0]) into a log-mel spectrogram
/// shaped `[N_MEL, frames]` in row-major order.
pub fn log_mel(pcm_f32: &[f32]) -> Vec<f32> {
    let n_frames = num_frames(pcm_f32.len());
    if n_frames == 0 {
        return Vec::new();
    }

    // Symmetric Hann window: w[i] = 0.5 - 0.5*cos(2π*i/(N-1))
    let window: Vec<f32> = (0..WIN_LEN)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / (WIN_LEN - 1) as f32;
            0.5 - 0.5 * phase.cos()
        })
        .collect();

    // FFT planner
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);
    let mut spectrum = fft.make_output_vec();

    // Power spectrogram bins we care about: 0..=N_FFT/2
    let n_freq_bins = N_FFT / 2 + 1;

    // Mel filterbank
    let mel_fb = mel_filterbank();

    // Compute log-mel features frame by frame
    let mut mel_energies = vec![0.0_f32; N_MEL * n_frames];
    let mut time_buf = vec![0.0_f32; N_FFT];

    for frame_idx in 0..n_frames {
        let start = frame_idx * HOP;
        let end = (start + WIN_LEN).min(pcm_f32.len());
        let frame_len = end - start;

        // Windowed frame, zero-padded to N_FFT
        time_buf.fill(0.0);
        for i in 0..frame_len {
            time_buf[i] = pcm_f32[start + i] * window[i];
        }

        fft.process(&mut time_buf, &mut spectrum)
            .expect("FFT process failed");

        // Power spectrum (computed once per freq bin)
        let mut power_spec = [0.0f32; N_FFT / 2 + 1];
        for freq_idx in 0..n_freq_bins {
            let c = spectrum[freq_idx];
            power_spec[freq_idx] = c.re * c.re + c.im * c.im;
        }

        // Apply mel filterbank
        for mel_idx in 0..N_MEL {
            let mut energy = 0.0_f32;
            for freq_idx in 0..n_freq_bins {
                energy += power_spec[freq_idx] * mel_fb[[mel_idx, freq_idx]];
            }
            // SpeechBrain / librosa default: ln(energy + ε) with a tiny
            // floor. NOT ln(1+energy) — the two diverge dramatically
            // for the small magnitudes the mel filterbank produces and
            // would feed out-of-distribution features to any model
            // trained on the standard preprocessing.
            mel_energies[mel_idx * n_frames + frame_idx] = (energy + 1e-10).ln();
        }
    }

    mel_energies
}

/// Number of frames produced for a clip of `samples` length.
pub fn num_frames(samples: usize) -> usize {
    if samples < WIN_LEN {
        0
    } else {
        1 + (samples - WIN_LEN) / HOP
    }
}

/// HTK mel formula: 2595 * log10(1 + f / 700)
fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0)
}

fn mel_filterbank() -> &'static Array2<f32> {
    static FILTERBANK: OnceLock<Array2<f32>> = OnceLock::new();
    FILTERBANK.get_or_init(|| {
        let n_freq_bins = N_FFT / 2 + 1;
        let mut fb = Array2::<f32>::zeros((N_MEL, n_freq_bins));

        let mel_min = hz_to_mel(MEL_FMIN);
        let mel_max = hz_to_mel(MEL_FMAX);
        let mel_points: Vec<f32> = (0..=N_MEL + 1)
            .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (N_MEL + 1) as f32)
            .collect();
        let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();
        let bin_points: Vec<f32> = hz_points
            .iter()
            .map(|&hz| hz * N_FFT as f32 / SAMPLE_RATE as f32)
            .collect();

        for mel_idx in 0..N_MEL {
            let left = bin_points[mel_idx];
            let center = bin_points[mel_idx + 1];
            let right = bin_points[mel_idx + 2];
            for freq_idx in 0..n_freq_bins {
                let f = freq_idx as f32;

                if f >= left && f <= center && center != left {
                    fb[[mel_idx, freq_idx]] = (f - left) / (center - left);
                } else if f >= center && f <= right && right != center {
                    fb[[mel_idx, freq_idx]] = (right - f) / (right - center);
                }
            }
        }

        fb
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn num_frames_zero_samples() {
        assert_eq!(num_frames(0), 0);
    }

    #[test]
    fn num_frames_just_below_window() {
        assert_eq!(num_frames(WIN_LEN - 1), 0);
    }

    #[test]
    fn num_frames_exactly_window() {
        assert_eq!(num_frames(WIN_LEN), 1);
    }

    #[test]
    fn num_frames_window_plus_hop() {
        assert_eq!(num_frames(WIN_LEN + HOP), 2);
    }

    #[test]
    fn num_frames_one_second() {
        // 1 second @ 16 kHz = 16_000 samples
        let expected = 1 + (16_000 - WIN_LEN) / HOP;
        assert_eq!(num_frames(16_000), expected);
    }

    #[test]
    fn log_mel_silence_is_finite() {
        let pcm = vec![0.0_f32; 16_000];
        let out = log_mel(&pcm);
        let expected_len = N_MEL * num_frames(16_000);
        assert_eq!(out.len(), expected_len);
        for &v in &out {
            assert!(v.is_finite(), "log-mel value should be finite, got {v}");
        }
    }

    #[test]
    fn log_mel_shape_independent_of_content() {
        let silence = vec![0.0_f32; 16_000];
        let sine: Vec<f32> = (0..16_000)
            .map(|i| {
                (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5
            })
            .collect();

        let out_silence = log_mel(&silence);
        let out_sine = log_mel(&sine);

        assert_eq!(out_silence.len(), out_sine.len());
    }
}
