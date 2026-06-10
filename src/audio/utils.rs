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
}
