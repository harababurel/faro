//! Voice note duration, 16-bar RMS waveform extraction, and pure-Rust WAV
//! inspection (M10, D80).

/// Number of amplitude bars stored and drawn for a voice note's waveform.
pub const WAVEFORM_BARS: usize = 16;

/// Eight single-cell block levels (`0..=7`) used to render a voice note's
/// waveform in every terminal and emoji mode (D80).
pub const WAVEFORM_GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Extracted duration and 16-bar waveform (`0..=7` per bar) of an audio clip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInfo {
    pub duration_secs: Option<u32>,
    pub waveform: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleKind {
    Pcm,
    Float,
}

#[derive(Debug, Clone, Copy)]
struct WavFormat {
    kind: SampleKind,
    channels: usize,
    sample_rate: u32,
    bits_per_sample: u16,
}

/// Inspect a RIFF/WAVE byte slice in pure Rust (8/16/24/32-bit PCM or 32-bit
/// IEEE float), returning its duration in seconds and a 16-bar RMS waveform.
pub fn inspect_wav(bytes: &[u8]) -> Option<AudioInfo> {
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    let mut fmt: Option<WavFormat> = None;
    while let Some(header) = bytes.get(pos..pos.checked_add(8)?) {
        let &[id0, id1, id2, id3, s0, s1, s2, s3] = header else {
            return None;
        };
        let id = [id0, id1, id2, id3];
        let size = usize::try_from(u32::from_le_bytes([s0, s1, s2, s3])).ok()?;
        let data_start = pos.checked_add(8)?;
        let data_end = data_start.checked_add(size)?.min(bytes.len());
        let payload = bytes.get(data_start..data_end)?;
        if &id == b"fmt " {
            fmt = parse_fmt(payload);
        } else if &id == b"data"
            && let Some(format) = fmt
        {
            return analyze_wav_data(payload, format);
        }
        let padded = size.saturating_add(size & 1);
        pos = data_start.checked_add(padded)?;
    }
    None
}

fn parse_fmt(payload: &[u8]) -> Option<WavFormat> {
    let &[
        t0,
        t1,
        c0,
        c1,
        r0,
        r1,
        r2,
        r3,
        _b0,
        _b1,
        _b2,
        _b3,
        _a0,
        _a1,
        s0,
        s1,
        ..,
    ] = payload
    else {
        return None;
    };
    let raw_tag = u16::from_le_bytes([t0, t1]);
    let channels = usize::from(u16::from_le_bytes([c0, c1]));
    let sample_rate = u32::from_le_bytes([r0, r1, r2, r3]);
    let bits_per_sample = u16::from_le_bytes([s0, s1]);
    if channels == 0 || sample_rate == 0 {
        return None;
    }
    let tag = if raw_tag == 0xFFFE {
        let &[sub0, sub1] = payload.get(24..26)? else {
            return None;
        };
        u16::from_le_bytes([sub0, sub1])
    } else {
        raw_tag
    };
    let kind = match (tag, bits_per_sample) {
        (1, 8 | 16 | 24 | 32) => SampleKind::Pcm,
        (3, 32) => SampleKind::Float,
        _ => return None,
    };
    Some(WavFormat {
        kind,
        channels,
        sample_rate,
        bits_per_sample,
    })
}

fn analyze_wav_data(data: &[u8], format: WavFormat) -> Option<AudioInfo> {
    let bytes_per_sample = usize::from(format.bits_per_sample / 8);
    let frame_bytes = bytes_per_sample.checked_mul(format.channels)?;
    if frame_bytes == 0 {
        return None;
    }
    let total_frames = data.len() / frame_bytes;
    let duration_secs = frames_duration_secs(total_frames, format.sample_rate);
    let waveform = bucket_rms(total_frames, |frame_idx| {
        let base = frame_idx * frame_bytes;
        let mut sum = 0.0_f64;
        for ch in 0..format.channels {
            let offset = base + ch * bytes_per_sample;
            let sample = data
                .get(offset..offset + bytes_per_sample)
                .map_or(0.0, |slice| {
                    decode_sample(slice, format.kind, format.bits_per_sample)
                });
            sum += sample.abs();
        }
        sum / f64::from(u16::try_from(format.channels).unwrap_or(1))
    });
    Some(AudioInfo {
        duration_secs,
        waveform,
    })
}

fn decode_sample(bytes: &[u8], kind: SampleKind, bits: u16) -> f64 {
    match (kind, bits) {
        (SampleKind::Pcm, 8) => {
            let v = f64::from(bytes.first().copied().unwrap_or(128));
            ((v - 128.0) / 128.0).clamp(-1.0, 1.0)
        }
        (SampleKind::Pcm, 16) => {
            let [b0, b1] = [
                bytes.first().copied().unwrap_or(0),
                bytes.get(1).copied().unwrap_or(0),
            ];
            f64::from(i16::from_le_bytes([b0, b1])) / 32_768.0
        }
        (SampleKind::Pcm, 24) => {
            let b0 = i32::from(bytes.first().copied().unwrap_or(0));
            let b1 = i32::from(bytes.get(1).copied().unwrap_or(0));
            let b2 = i32::from(bytes.get(2).copied().unwrap_or(0).cast_signed());
            let sample = b0 | (b1 << 8) | (b2 << 16);
            f64::from(sample) / 8_388_608.0
        }
        (SampleKind::Pcm, 32) => {
            let [b0, b1, b2, b3] = [
                bytes.first().copied().unwrap_or(0),
                bytes.get(1).copied().unwrap_or(0),
                bytes.get(2).copied().unwrap_or(0),
                bytes.get(3).copied().unwrap_or(0),
            ];
            f64::from(i32::from_le_bytes([b0, b1, b2, b3])) / 2_147_483_648.0
        }
        (SampleKind::Float, 32) => {
            let [b0, b1, b2, b3] = [
                bytes.first().copied().unwrap_or(0),
                bytes.get(1).copied().unwrap_or(0),
                bytes.get(2).copied().unwrap_or(0),
                bytes.get(3).copied().unwrap_or(0),
            ];
            let v = f64::from(f32::from_le_bytes([b0, b1, b2, b3]));
            if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        }
        (SampleKind::Pcm | SampleKind::Float, _) => 0.0,
    }
}

/// Compute duration and a 16-bar RMS waveform from raw mono 16-bit signed
/// little-endian PCM samples (e.g. decoded by `ffmpeg -f s16le`).
pub fn pcm_s16le_info(bytes: &[u8], sample_rate: u32) -> Option<AudioInfo> {
    if sample_rate == 0 {
        return None;
    }
    let total_frames = bytes.len() / 2;
    let duration_secs = frames_duration_secs(total_frames, sample_rate);
    let waveform = bucket_rms(total_frames, |idx| {
        let offset = idx * 2;
        let b0 = bytes.get(offset).copied().unwrap_or(0);
        let b1 = bytes.get(offset + 1).copied().unwrap_or(0);
        (f64::from(i16::from_le_bytes([b0, b1])) / 32_768.0).abs()
    });
    Some(AudioInfo {
        duration_secs,
        waveform,
    })
}

fn frames_duration_secs(total_frames: usize, sample_rate: u32) -> Option<u32> {
    let sr = u64::from(sample_rate);
    if sr == 0 {
        return None;
    }
    let frames = u64::try_from(total_frames).ok()?;
    let secs = if frames == 0 {
        0
    } else {
        (frames.saturating_add(sr / 2) / sr).max(1)
    };
    u32::try_from(secs).ok()
}

fn bucket_rms(total_frames: usize, sample_at: impl Fn(usize) -> f64) -> Vec<u8> {
    if total_frames == 0 {
        return vec![0; WAVEFORM_BARS];
    }
    let mut rms = [0.0_f64; WAVEFORM_BARS];
    let mut peak = 0.0_f64;
    for (bar_idx, slot) in rms.iter_mut().enumerate() {
        let start = bar_idx * total_frames / WAVEFORM_BARS;
        let end = ((bar_idx + 1) * total_frames / WAVEFORM_BARS)
            .max(start.saturating_add(1))
            .min(total_frames);
        let span = end.saturating_sub(start).max(1);
        let step = (span / 512).max(1);
        let mut sum_sq = 0.0_f64;
        let mut count = 0_u32;
        let mut i = start;
        while i < end {
            let amp = sample_at(i);
            sum_sq += amp * amp;
            count = count.saturating_add(1);
            i = i.saturating_add(step);
        }
        let val = if count > 0 {
            (sum_sq / f64::from(count)).sqrt()
        } else {
            0.0
        };
        if val > peak {
            peak = val;
        }
        *slot = val;
    }
    if peak <= 1e-6 {
        return vec![0; WAVEFORM_BARS];
    }
    rms.into_iter()
        .map(|val| {
            let ratio = (val / peak).clamp(0.0, 1.0);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "ratio is clamped to 0.0..=1.0 so (ratio * 7.0).round() is in 0.0..=7.0"
            )]
            let bar = (ratio * 7.0).round() as u8;
            bar.min(7)
        })
        .collect()
}

/// Deterministic 16-bar speech-shaped envelope (`0..=7`) derived from `seed`,
/// used when a compressed voice note cannot be decoded by `ffmpeg`.
pub fn synthetic_waveform(seed: &[u8]) -> Vec<u8> {
    let mut state = 0xcbf2_9ce4_8422_2325_u64;
    for &b in seed {
        state ^= u64::from(b);
        state = state.wrapping_mul(0x0100_0000_01b3);
    }
    (0..WAVEFORM_BARS)
        .map(|i| {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            let raw = u8::try_from(z % 7).unwrap_or(0) + 1;
            // Taper the very first and last bars slightly like spoken phrases.
            if i == 0 || i + 1 == WAVEFORM_BARS {
                raw.min(3)
            } else {
                raw
            }
        })
        .collect()
}

/// Format a duration in seconds as `m:ss` (or `h:mm:ss` when an hour or more).
pub fn format_duration(secs: u32) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let rem = secs % 60;
    if hours > 0 {
        format!("{hours}:{mins:02}:{rem:02}")
    } else {
        format!("{mins}:{rem:02}")
    }
}

/// Render amplitude bars (`0..=7`) as single-cell block characters (`▁`..`█`).
pub fn format_waveform(bars: &[u8]) -> String {
    bars.iter()
        .take(WAVEFORM_BARS)
        .map(|&b| WAVEFORM_GLYPHS.get(usize::from(b)).copied().unwrap_or('█'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_wav_s16le(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
        let data_len = u32::try_from(samples.len() * 2).unwrap_or(0);
        let riff_len = 36_u32.saturating_add(data_len);
        let byte_rate = sample_rate.saturating_mul(2);
        let mut out = Vec::with_capacity(44 + samples.len() * 2);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&riff_len.to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16_u32.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1_u16.to_le_bytes()); // 1 channel
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&byte_rate.to_le_bytes());
        out.extend_from_slice(&2_u16.to_le_bytes()); // block align
        out.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for &s in samples {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    #[test]
    fn wav_duration_and_waveform_reflect_amplitude_envelope() {
        let sr = 8_000_u32;
        // 3 seconds at 8 kHz with a rising then falling envelope.
        let total = (sr * 3) as usize;
        let samples: Vec<i16> = (0..total)
            .map(|i| {
                let bucket = i * WAVEFORM_BARS / total;
                let distance = bucket.abs_diff(WAVEFORM_BARS / 2);
                let gain = i16::try_from((WAVEFORM_BARS / 2).saturating_sub(distance)).unwrap_or(0);
                let sign = if i % 20 < 10 { 1_i16 } else { -1_i16 };
                sign * gain * 3_000
            })
            .collect();
        let wav = build_wav_s16le(sr, &samples);
        let info = inspect_wav(&wav).unwrap();
        assert_eq!(info.duration_secs, Some(3));
        assert_eq!(info.waveform.len(), WAVEFORM_BARS);
        // Peak in the middle, quiet at the start.
        assert_eq!(info.waveform[0], 0);
        assert_eq!(info.waveform[WAVEFORM_BARS / 2], 7);
        assert_eq!(format_waveform(&info.waveform).chars().count(), 16);
    }

    #[test]
    fn non_wav_bytes_return_none_and_synthetic_is_deterministic() {
        assert_eq!(inspect_wav(b"not a wav file"), None);
        let a = synthetic_waveform(b"voice-1");
        let b = synthetic_waveform(b"voice-1");
        let c = synthetic_waveform(b"voice-2");
        assert_eq!(a.len(), WAVEFORM_BARS);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.iter().all(|&v| v <= 7));
    }

    #[test]
    fn durations_format_cleanly() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(9), "0:09");
        assert_eq!(format_duration(74), "1:14");
        assert_eq!(format_duration(3_661), "1:01:01");
    }
}
