//! MP3 frame arithmetic: pull the audio frames out of what Kokoro
//! sends, and join them — with hand-built silence between turns — into
//! one stream with a fresh Info/Xing header, so players see the right
//! duration. Layer III only; nothing here re-encodes.

use crate::error::{Error, Result};
use std::time::Duration;

/// An episode at most about 12 MB; 64 is already nonsense.
pub(crate) const MOST_JOINED: usize = 64 << 20;
/// The first frame is looked for within this much of the start, past
/// an ID3v2 tag and any junk before the audio.
const FIRST_FRAME_WITHIN: usize = 64 << 10;

const MPEG1_BITRATES_KBPS: [u16; 16] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
];
const MPEG2_BITRATES_KBPS: [u16; 16] = [
    0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
];
const MPEG1_RATES: [u32; 3] = [44_100, 48_000, 32_000];
const MPEG2_RATES: [u32; 3] = [22_050, 24_000, 16_000];
const MPEG25_RATES: [u32; 3] = [11_025, 12_000, 8_000];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    Mpeg1,
    Mpeg2,
    Mpeg25,
}

impl Version {
    fn from_bits(bits: u8) -> Option<Version> {
        match bits {
            0b11 => Some(Version::Mpeg1),
            0b10 => Some(Version::Mpeg2),
            0b00 => Some(Version::Mpeg25),
            _ => None,
        }
    }

    fn bits(self) -> u8 {
        match self {
            Version::Mpeg1 => 0b11,
            Version::Mpeg2 => 0b10,
            Version::Mpeg25 => 0b00,
        }
    }

    fn rates(self) -> [u32; 3] {
        match self {
            Version::Mpeg1 => MPEG1_RATES,
            Version::Mpeg2 => MPEG2_RATES,
            Version::Mpeg25 => MPEG25_RATES,
        }
    }

    /// Samples one frame carries: the encoder's granule.
    fn samples_per_frame(self) -> u32 {
        match self {
            Version::Mpeg1 => 1152,
            _ => 576,
        }
    }

    /// Frame-length coefficient: `coef * bitrate / rate + padding`.
    fn coef(self) -> u32 {
        match self {
            Version::Mpeg1 => 144,
            _ => 72,
        }
    }

    /// Side-info bytes after the header (and CRC, when there is one).
    fn side_info(self, mono: bool) -> usize {
        match self {
            Version::Mpeg1 => {
                if mono {
                    17
                } else {
                    32
                }
            }
            _ => {
                if mono {
                    9
                } else {
                    17
                }
            }
        }
    }
}

/// What segments must share to be joined: same MPEG version, sample
/// rate and channel count. Joint and plain stereo mix freely, so only
/// mono-ness is compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    version: Version,
    sample_rate: u32,
    mono: bool,
}

impl Format {
    fn describe(self) -> String {
        let rate = self.sample_rate;
        let rate = if rate >= 1000 {
            format!("{} {:03}", rate / 1000, rate % 1000)
        } else {
            format!("{rate}")
        };
        format!("{rate} Hz {}", if self.mono { "mono" } else { "stereo" })
    }
}

#[derive(Debug, Clone, Copy)]
struct Header {
    version: Version,
    bitrate_index: u8,
    sample_rate_index: u8,
    padding: u8,
    mono: bool,
    /// True when the protection bit says a 2-byte CRC follows.
    crc: bool,
}

impl Header {
    fn parse(bytes: &[u8]) -> Option<Header> {
        if bytes.len() < 4 || bytes[0] != 0xFF || (bytes[1] & 0xE0) != 0xE0 {
            return None;
        }
        let version = Version::from_bits((bytes[1] >> 3) & 0b11)?;
        // Layer III only.
        if (bytes[1] >> 1) & 0b11 != 0b01 {
            return None;
        }
        let bitrate_index = (bytes[2] >> 4) & 0x0F;
        // 0 is free format, 15 is bad; neither is joined.
        if bitrate_index == 0 || bitrate_index == 15 {
            return None;
        }
        let sample_rate_index = (bytes[2] >> 2) & 0b11;
        if sample_rate_index == 3 {
            return None;
        }
        let mono = (bytes[3] >> 6) & 0b11 == 0b11;
        Some(Header {
            version,
            bitrate_index,
            sample_rate_index,
            padding: (bytes[2] >> 1) & 1,
            mono,
            crc: bytes[1] & 1 == 0,
        })
    }

    fn sample_rate(&self) -> u32 {
        self.version.rates()[self.sample_rate_index as usize]
    }

    fn bitrate_kbps(&self) -> u32 {
        let table = match self.version {
            Version::Mpeg1 => &MPEG1_BITRATES_KBPS,
            _ => &MPEG2_BITRATES_KBPS,
        };
        table[self.bitrate_index as usize] as u32
    }

    fn frame_len(&self) -> usize {
        (self.version.coef() as usize * self.bitrate_kbps() as usize * 1000)
            / self.sample_rate() as usize
            + self.padding as usize
    }

    fn crc_len(&self) -> usize {
        if self.crc { 2 } else { 0 }
    }

    /// Where a Xing/Info tag starts inside the frame, if there is one.
    fn xing_offset(&self) -> usize {
        4 + self.crc_len() + self.version.side_info(self.mono)
    }

    fn format(&self) -> Format {
        Format {
            version: self.version,
            sample_rate: self.sample_rate(),
            mono: self.mono,
        }
    }
}

/// A silent frame in `reference`'s own format: same version, sample
/// rate and channels, so a constant-bitrate stream stays constant. The
/// side info is all zero — no bit reservoir, nothing to decode.
fn silent_frame(reference: &Header, bitrate_index: u8) -> Vec<u8> {
    let header = Header {
        version: reference.version,
        bitrate_index,
        sample_rate_index: reference.sample_rate_index,
        padding: 0,
        mono: reference.mono,
        crc: false,
    };
    let mut frame = vec![0u8; header.frame_len()];
    frame[0] = 0xFF;
    frame[1] = 0xE0 | (header.version.bits() << 3) | (0b01 << 1) | 1;
    frame[2] = (bitrate_index << 4) | (header.sample_rate_index << 2);
    // Channel mode: 3 (both bits) is mono; plain stereo is 0. Joint
    // stereo is never invented here — silence has no image to keep.
    frame[3] = if header.mono { 0xC0 } else { 0 };
    frame
}

/// The audio frames of one segment, and the format they share. ID3v2,
/// ID3v1, APE and Xing/Info/VBRI tags are dropped; a truncated last
/// frame is dropped.
pub fn frames(segment: &[u8]) -> Result<(Format, Vec<&[u8]>)> {
    let rest = skip_id3v2(segment);
    let search_end = rest.len().min(FIRST_FRAME_WITHIN);
    let mut start = None;
    for at in 0..search_end.saturating_sub(3) {
        let Some(header) = Header::parse(&rest[at..]) else {
            continue;
        };
        let len = header.frame_len();
        if at + len > rest.len() {
            continue;
        }
        // The first frame is real when a second one follows in the
        // same format, or nothing follows at all (a one-frame file).
        if at + len + 4 <= rest.len() {
            let next = Header::parse(&rest[at + len..]);
            if next.map(|h| h.format()) != Some(header.format()) {
                continue;
            }
        }
        start = Some((at, header));
        break;
    }
    let (start, first) = start.ok_or_else(|| Error::Audio {
        reason: "what came back is not MP3 audio".into(),
    })?;
    let format = first.format();
    let mut out = Vec::new();
    let mut at = start;
    while at + 4 <= rest.len() {
        let Some(header) = Header::parse(&rest[at..]) else {
            break;
        };
        if header.format() != format {
            break;
        }
        let len = header.frame_len();
        if at + len > rest.len() {
            break;
        }
        out.push(&rest[at..at + len]);
        at += len;
    }
    // A first frame that carries encoder bookkeeping, not audio.
    if let Some(first_frame) = out.first() {
        let header = Header::parse(first_frame).expect("parsed above");
        let xing = header.xing_offset();
        let is_xing = first_frame.len() >= xing + 4
            && matches!(&first_frame[xing..xing + 4], b"Xing" | b"Info");
        let is_vbri = first_frame.len() >= 40 && &first_frame[36..40] == b"VBRI";
        if is_xing || is_vbri {
            out.remove(0);
        }
    }
    if out.is_empty() {
        return Err(Error::Audio {
            reason: "what came back is not MP3 audio".into(),
        });
    }
    Ok((format, out))
}

fn skip_id3v2(segment: &[u8]) -> &[u8] {
    let Some(head) = segment.get(..10) else {
        return segment;
    };
    if &head[0..3] != b"ID3" {
        return segment;
    }
    let size = ((head[6] as usize & 0x7F) << 21)
        | ((head[7] as usize & 0x7F) << 14)
        | ((head[8] as usize & 0x7F) << 7)
        | (head[9] as usize & 0x7F);
    let mut skip = 10 + size;
    // A footer mirrors the header.
    if head[5] & 0x10 != 0 {
        skip += 10;
    }
    segment.get(skip..).unwrap_or(&[])
}

/// One joined episode: the MP3 bytes, the body frame count (the tag
/// frame excluded), and its length in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Joined {
    pub mp3: Vec<u8>,
    pub frames: u32,
    pub seconds: f64,
}

/// Join the segments into one stream: a silent lead-in, then each
/// segment with a silent pause between turns, and a fresh Info/Xing
/// frame first so players read the duration straight off the file.
pub fn join(segments: &[Vec<u8>], lead_in: Duration, pause: Duration) -> Result<Joined> {
    if segments.is_empty() {
        return Err(Error::Audio {
            reason: "there is nothing to join".into(),
        });
    }
    let too_big = || Error::Audio {
        reason: "the recording came out bigger than an episode can be".into(),
    };
    let mut input_bytes = 0usize;
    for segment in segments {
        if segment.len() > MOST_JOINED - input_bytes {
            return Err(too_big());
        }
        input_bytes += segment.len();
    }
    let parsed = segments
        .iter()
        .map(|segment| frames(segment))
        .collect::<Result<Vec<_>>>()?;
    let format = parsed[0].0;
    for (other, _) in &parsed[1..] {
        if *other != format {
            return Err(Error::Audio {
                reason: format!(
                    "the hosts' audio is not in the same format ({} and {})",
                    format.describe(),
                    other.describe()
                ),
            });
        }
    }
    let reference = Header::parse(parsed[0].1[0]).expect("frames() validated it");
    let rate = format.sample_rate as f64;
    let spf = format.version.samples_per_frame() as f64;
    let silence_count = |duration: Duration| {
        ((duration.as_millis() as f64 * rate) / (1000.0 * spf)).ceil() as usize
    };
    let lead_in_frames = silence_count(lead_in);
    let pause_frames = silence_count(pause);

    let silence_frames = pause_frames
        .checked_mul(segments.len() - 1)
        .and_then(|pauses| pauses.checked_add(lead_in_frames))
        .ok_or_else(too_big)?;
    let silence = silent_frame(&reference, reference.bitrate_index);
    let xing_room = 4 + reference.version.side_info(reference.mono) + 16;
    let tag_index = if silence.len() >= xing_room {
        reference.bitrate_index
    } else {
        (1..=14)
            .find(|&bitrate_index| {
                Header {
                    bitrate_index,
                    padding: 0,
                    crc: false,
                    ..reference
                }
                .frame_len()
                    >= xing_room
            })
            .expect("the top MPEG-1 rate, 320 kbps, always has room")
    };
    let mut tag = silent_frame(&reference, tag_index);
    let mut bytes = silence_frames
        .checked_mul(silence.len())
        .and_then(|body| body.checked_add(tag.len()))
        .filter(|&size| size <= MOST_JOINED)
        .ok_or_else(too_big)?;
    let mut body_frames_count = silence_frames;
    let mut one_bitrate = true;
    for (_, segment_frames) in &parsed {
        body_frames_count += segment_frames.len();
        for frame in segment_frames {
            if frame.len() > MOST_JOINED - bytes {
                return Err(too_big());
            }
            bytes += frame.len();
            one_bitrate &= Header::parse(frame)
                .expect("frames() validated it")
                .bitrate_index
                == reference.bitrate_index;
        }
    }
    let tag_id: &[u8] = if one_bitrate { b"Info" } else { b"Xing" };
    let at = 4 + reference.version.side_info(reference.mono);
    tag[at..at + 4].copy_from_slice(tag_id);
    tag[at + 4..at + 8].copy_from_slice(&3u32.to_be_bytes());
    tag[at + 8..at + 12].copy_from_slice(&(body_frames_count as u32).to_be_bytes());
    tag[at + 12..at + 16].copy_from_slice(&(bytes as u32).to_be_bytes());

    let mut mp3 = Vec::with_capacity(bytes);
    mp3.extend_from_slice(&tag);
    for _ in 0..lead_in_frames {
        mp3.extend_from_slice(&silence);
    }
    for (i, (_, segment_frames)) in parsed.iter().enumerate() {
        if i > 0 {
            for _ in 0..pause_frames {
                mp3.extend_from_slice(&silence);
            }
        }
        for frame in segment_frames {
            mp3.extend_from_slice(frame);
        }
    }
    Ok(Joined {
        seconds: body_frames_count as f64 * spf / rate,
        frames: body_frames_count as u32,
        mp3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kokoro's own format: MPEG-2, 24 kHz, mono, 64 kbps — verified
    /// against its real answers, and what the silence oracle decodes.
    const REFERENCE: [u8; 4] = [0xFF, 0xF3, 0x84, 0xC0];

    fn reference_header() -> Header {
        Header::parse(&REFERENCE).unwrap()
    }

    /// `count` silent frames at `bitrate_index` (64 kbps when None).
    fn silent_frames(count: usize, bitrate_index: Option<u8>) -> Vec<u8> {
        let header = reference_header();
        let frame = silent_frame(&header, bitrate_index.unwrap_or(8));
        frame.repeat(count)
    }

    fn id3v2(payload: &[u8]) -> Vec<u8> {
        let size = payload.len();
        let syncsafe = [
            ((size >> 21) & 0x7F) as u8,
            ((size >> 14) & 0x7F) as u8,
            ((size >> 7) & 0x7F) as u8,
            (size & 0x7F) as u8,
        ];
        [b"ID3\x04\x00\x00".as_slice(), &syncsafe, payload].concat()
    }

    /// A segment the way an encoder writes one: ID3v2, a bookkeeping
    /// frame, then audio.
    fn segment(frames: &[u8]) -> Vec<u8> {
        let header = reference_header();
        let mut info = silent_frame(&header, 8);
        let at = 4 + header.version.side_info(true);
        info[at..at + 4].copy_from_slice(b"Info");
        let mut out = id3v2(b"tag data");
        out.extend_from_slice(&info);
        out.extend_from_slice(frames);
        out
    }

    #[test]
    fn frames_skip_id3v2_a_leading_info_and_a_trailing_tag() {
        let mut audio = silent_frames(3, None);
        audio.extend_from_slice(b"TAGsong title");
        let mut input = id3v2(b"metadata here");
        let header = reference_header();
        let mut info = silent_frame(&header, 8);
        let at = 4 + header.version.side_info(true);
        info[at..at + 4].copy_from_slice(b"Info");
        input.extend_from_slice(&info);
        input.extend_from_slice(&audio);
        let (format, frames) = frames(&input).expect("parsed");
        assert_eq!(format.sample_rate, 24_000);
        assert!(format.mono);
        assert_eq!(frames.len(), 3);
        assert!(frames.iter().all(|f| f.len() == 192));
    }

    #[test]
    fn a_crc_protected_stream_is_walked() {
        // Protection bit 0 → 2 CRC bytes after the header; the frame
        // length is unchanged.
        let mut header = REFERENCE;
        header[1] &= !1;
        let len = Header::parse(&header).unwrap().frame_len();
        assert_eq!(len, 192);
        let frame = [header.as_slice(), &vec![0u8; len - 4]].concat();
        let stream = [frame.clone(), frame].concat();
        let (_, frames) = frames(&stream).expect("parsed");
        assert_eq!(frames.len(), 2);
    }

    #[test]
    fn free_format_is_refused() {
        let mut header = REFERENCE;
        header[2] = 0x04; // bitrate index 0: free format
        let frame = [header.as_slice(), &[0u8; 188]].concat();
        assert!(frames(&frame.repeat(2)).is_err());
    }

    #[test]
    fn garbage_is_not_mp3_audio() {
        assert!(frames(b"this is not audio at all").is_err());
        assert!(frames(b"\xff\xfa\x00\x00\xff\xfa\x00\x00").is_err());
    }

    #[test]
    fn join_counts_frames_and_writes_an_info_tag() {
        let one = silent_frames(10, None);
        let two = silent_frames(5, None);
        let joined = join(
            &[segment(&one), segment(&two)],
            Duration::from_millis(500),
            Duration::from_millis(400),
        )
        .expect("joined");
        // 21 lead-in + 10 + 17 pause + 5, plus the tag frame.
        assert_eq!(joined.frames, 21 + 10 + 17 + 5);
        assert_eq!(joined.mp3.len(), 192 * (1 + joined.frames as usize));
        assert_eq!(
            joined.seconds,
            (21.0 + 10.0 + 17.0 + 5.0) * 576.0 / 24_000.0
        );
        let tag = &joined.mp3[..192];
        let at = 4 + reference_header().version.side_info(true);
        assert_eq!(&tag[at..at + 4], b"Info");
        assert_eq!(&tag[at + 4..at + 8], &3u32.to_be_bytes());
        assert_eq!(
            &tag[at + 8..at + 12],
            &((21 + 10 + 17 + 5) as u32).to_be_bytes()
        );
        let bytes = (192 * (1 + 21 + 10 + 17 + 5)) as u32;
        assert_eq!(&tag[at + 12..at + 16], &bytes.to_be_bytes());
    }

    #[test]
    fn mixed_bitrates_get_a_xing_tag() {
        // Reference 64 kbps (index 8); index 9 is 80 kbps at 24 kHz.
        let one = silent_frames(4, Some(8));
        let two = silent_frames(4, Some(9));
        let joined = join(
            &[segment(&one), segment(&two)],
            Duration::from_millis(500),
            Duration::from_millis(400),
        )
        .expect("joined");
        let tag = &joined.mp3[..192];
        let at = 4 + reference_header().version.side_info(true);
        assert_eq!(&tag[at..at + 4], b"Xing");
        // 80 kbps at 24 kHz: 72 * 80000 / 24000 = 240 bytes a frame,
        // and the second segment is where those frames are.
        let second_segment = &joined.mp3[192 * (1 + 21 + 4 + 17)..];
        assert_eq!(Header::parse(second_segment).unwrap().frame_len(), 240);
    }

    #[test]
    fn mismatched_sample_rates_are_refused() {
        // MPEG-1, 44.1 kHz, mono, 128 kbps.
        let other = Header::parse(&[0xFF, 0xFB, 0x90, 0xC0]).unwrap();
        assert_eq!(other.frame_len(), 417);
        let foreign = silent_frame(&other, 9).repeat(3);
        let error = join(
            &[segment(&silent_frames(3, None)), segment(&foreign)],
            Duration::from_millis(500),
            Duration::from_millis(400),
        )
        .expect_err("refused");
        let message = error.to_string();
        assert!(
            message.contains("24 000 Hz mono") && message.contains("44 100 Hz mono"),
            "unexpected: {message}"
        );
    }

    #[test]
    fn the_output_keeps_no_tags_from_the_segments() {
        let joined = join(
            &[
                segment(&silent_frames(4, None)),
                segment(&silent_frames(2, None)),
            ],
            Duration::from_millis(500),
            Duration::from_millis(400),
        )
        .expect("joined");
        assert!(!joined.mp3.windows(3).any(|w| w == b"ID3"));
        assert!(
            !joined
                .mp3
                .windows(4)
                .any(|w| w == b"TAG\x00" || w == b"APET")
        );
    }

    #[test]
    fn an_empty_join_is_refused() {
        assert!(join(&[], Duration::from_millis(500), Duration::from_millis(400)).is_err());
    }

    /// The loop used while planning: probe, decode every packet, count
    /// samples. A joined episode must decode to exactly what the Info
    /// frame claims, and to digital silence, or the whole join is wrong.
    #[test]
    fn a_joined_episode_decodes_to_silence_for_its_exact_length() {
        let one = silent_frames(30, Some(8));
        let two = silent_frames(20, Some(9));
        let joined = join(
            &[segment(&one), segment(&two)],
            Duration::from_millis(500),
            Duration::from_millis(400),
        )
        .expect("joined");
        let body_frames = 21 + 30 + 17 + 20;
        assert_eq!(joined.frames, body_frames);

        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::codecs::DecoderOptions;
        use symphonia::core::errors::Error as SymphoniaError;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;
        let stream = MediaSourceStream::new(
            Box::new(std::io::Cursor::new(joined.mp3.clone())),
            Default::default(),
        );
        let probed = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .expect("probed");
        let mut format = probed.format;
        let track = format.default_track().expect("a track");
        assert_eq!(
            track.codec_params.n_frames,
            Some(body_frames as u64 * 576),
            "the Info frame's count, as the codec reads it"
        );
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .expect("a decoder");
        let mut heard = 0u64;
        let mut peak = 0.0f32;
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => panic!("decode failed: {e}"),
            };
            let decoded = decoder.decode(&packet).expect("a packet decodes");
            let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            buffer.copy_interleaved_ref(decoded);
            heard += buffer.samples().len() as u64;
            peak = peak.max(
                buffer
                    .samples()
                    .iter()
                    .map(|s| s.abs())
                    .fold(0.0f32, f32::max),
            );
        }
        assert_eq!(heard, body_frames as u64 * 576);
        assert_eq!(peak, 0.0, "silence joined is silence");
    }
}
