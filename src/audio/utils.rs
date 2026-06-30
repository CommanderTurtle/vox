//! Utility functions for audio processing (WAV encoding, resampling, level detection).

use hound::WavSpec;

/// Encode PCM i16 samples (16 kHz, mono) into WAV bytes.
pub fn pcm_to_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut buf = Vec::new();
    {
        let mut writer = hound::WavWriter::new(std::io::Cursor::new(&mut buf), spec)?;
        for &sample in samples {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
    }
    Ok(buf)
}

/// Simple RMS energy level of a PCM buffer (for level indicator).
#[allow(dead_code)]
pub fn rms_level(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Convert RMS to a normalized 0..1 level.
#[allow(dead_code)]
pub fn normalized_level(rms: f64) -> f64 {
    // Typical speech RMS is ~1000-5000 for i16
    let level = rms / 16384.0;
    level.clamp(0.0, 1.0)
}

/// Get the duration of audio in seconds given sample count and rate.
#[allow(dead_code)]
pub fn duration_seconds(sample_count: usize, sample_rate: u32) -> f64 {
    sample_count as f64 / sample_rate as f64
}

/// Resample mono i16 PCM from `from_rate` to `to_rate` using linear
/// interpolation. If the rates are equal the input is returned unchanged.
///
/// ASR engines (and whisper.cpp) expect 16 kHz; device capture is often
/// 44.1/48 kHz, so we resample before encoding to WAV.
pub fn resample_linear(samples: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if samples.is_empty() || from_rate == to_rate {
        return samples.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = ((samples.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 / ratio;
        let idx0 = src_pos.floor() as usize;
        let idx1 = (idx0 + 1).min(samples.len() - 1);
        let frac = src_pos - idx0 as f64;
        let s0 = samples[idx0] as f64;
        let s1 = samples[idx1] as f64;
        out.push((s0 + (s1 - s0) * frac).round().clamp(-32768.0, 32767.0) as i16);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcm_to_wav_roundtrip() {
        let samples: Vec<i16> = vec![0, 1000, -1000, 0, 5000, -5000, 0];
        let wav_bytes = pcm_to_wav(&samples, 16000).unwrap();
        assert!(!wav_bytes.is_empty());

        // Verify WAV header (RIFF)
        assert_eq!(&wav_bytes[0..4], b"RIFF");
        assert_eq!(&wav_bytes[8..12], b"WAVE");
    }

    #[test]
    fn test_rms_silence() {
        let samples = vec![0i16; 100];
        assert!(rms_level(&samples) < 1.0);
    }

    #[test]
    fn test_rms_nonzero() {
        let samples = vec![10000i16; 100];
        let rms = rms_level(&samples);
        assert!(rms > 9000.0 && rms < 11000.0);
    }

    #[test]
    fn test_duration() {
        assert!((duration_seconds(16000, 16000) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_resample_identity() {
        let samples = vec![100i16, 200, 300, 400];
        let out = resample_linear(&samples, 16000, 16000);
        assert_eq!(out, samples);
    }

    #[test]
    fn test_resample_downsample() {
        // 48000 -> 16000 should yield a third of the samples
        let samples: Vec<i16> = (0..48).collect();
        let out = resample_linear(&samples, 48000, 16000);
        assert_eq!(out.len(), 16);
    }
}
