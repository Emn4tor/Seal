//! Sample-rate conversion between whatever rate the local audio device
//! runs at and the fixed [`crate::SAMPLE_RATE_HZ`] every voice call operates
//! on internally. Thin wrapper around `rubato`'s polynomial async
//! resampler (no anti-aliasing sinc filter) — the library's own docs
//! recommend exactly that mode for "a voice application such as a video
//! call," trading a little quality for speed.

use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Async, FixedAsync, PolynomialDegree, Resampler as _};
use std::collections::VecDeque;

const CHANNELS: usize = 1;
// Async resamplers need an upper bound on how far the ratio may later be
// nudged; we never adjust it after construction, so any value >= 1.0 works.
const MAX_RELATIVE_RATIO: f64 = 1.0;

/// Converts a stream of mono samples at some device-native rate into
/// fixed-size [`crate::adpcm::SAMPLES_PER_FRAME`]-sample chunks at
/// [`crate::SAMPLE_RATE_HZ`], ready for the pitch shifter / encoder.
pub struct ToTargetRate {
    inner: Async<f32>,
    pending_input: VecDeque<f32>,
    input_buf: Vec<Vec<f32>>,
    output_buf: Vec<Vec<f32>>,
}

impl ToTargetRate {
    pub fn new(source_rate_hz: u32, output_chunk_frames: usize) -> Self {
        let ratio = crate::SAMPLE_RATE_HZ as f64 / source_rate_hz as f64;
        let inner = Async::<f32>::new_poly(
            ratio,
            MAX_RELATIVE_RATIO,
            PolynomialDegree::Cubic,
            output_chunk_frames,
            CHANNELS,
            FixedAsync::Output,
        )
        .expect("valid resampler parameters");
        let input_buf = vec![vec![0.0f32; inner.input_frames_max()]; CHANNELS];
        let output_buf = vec![vec![0.0f32; inner.output_frames_max()]; CHANNELS];
        Self {
            inner,
            pending_input: VecDeque::new(),
            input_buf,
            output_buf,
        }
    }

    /// Feeds newly captured native-rate samples. Returns zero or more
    /// complete `output_chunk_frames`-sample chunks at the target rate —
    /// zero if not enough source audio has accumulated yet.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        self.pending_input.extend(samples.iter().copied());
        let mut chunks = Vec::new();
        loop {
            let need = self.inner.input_frames_next();
            if self.pending_input.len() < need {
                break;
            }
            for i in 0..need {
                self.input_buf[0][i] = self.pending_input.pop_front().unwrap();
            }
            let input_adapter = SequentialSliceOfVecs::new(&self.input_buf, CHANNELS, need)
                .expect("input buffer sized to input_frames_next");
            let output_len = self.output_buf[0].len();
            let mut output_adapter =
                SequentialSliceOfVecs::new_mut(&mut self.output_buf, CHANNELS, output_len)
                    .expect("output buffer sized to output_frames_max");
            let (_nbr_in, nbr_out) = self
                .inner
                .process_into_buffer(&input_adapter, &mut output_adapter, None)
                .expect("fixed-ratio resample never fails once constructed");
            chunks.push(self.output_buf[0][..nbr_out].to_vec());
        }
        chunks
    }
}

/// Converts fixed-size [`crate::SAMPLE_RATE_HZ`] frames (as produced by
/// [`crate::adpcm::Decoder`]) back to a device's native playback rate.
pub struct FromTargetRate {
    inner: Async<f32>,
    input_buf: Vec<Vec<f32>>,
    output_buf: Vec<Vec<f32>>,
}

impl FromTargetRate {
    pub fn new(target_rate_hz: u32, input_chunk_frames: usize) -> Self {
        let ratio = target_rate_hz as f64 / crate::SAMPLE_RATE_HZ as f64;
        let inner = Async::<f32>::new_poly(
            ratio,
            MAX_RELATIVE_RATIO,
            PolynomialDegree::Cubic,
            input_chunk_frames,
            CHANNELS,
            FixedAsync::Input,
        )
        .expect("valid resampler parameters");
        let input_buf = vec![vec![0.0f32; inner.input_frames_max()]; CHANNELS];
        let output_buf = vec![vec![0.0f32; inner.output_frames_max()]; CHANNELS];
        Self {
            inner,
            input_buf,
            output_buf,
        }
    }

    /// Converts exactly one `input_chunk_frames`-sample frame at
    /// [`crate::SAMPLE_RATE_HZ`] into the equivalent (variable-length) span
    /// of native-rate samples.
    pub fn push_frame(&mut self, frame: &[f32]) -> Vec<f32> {
        let need = self.inner.input_frames_next();
        assert_eq!(
            frame.len(),
            need,
            "caller must feed exactly one fixed-size frame"
        );
        self.input_buf[0][..need].copy_from_slice(frame);
        let input_adapter = SequentialSliceOfVecs::new(&self.input_buf, CHANNELS, need)
            .expect("input buffer sized to input_frames_next");
        let output_len = self.output_buf[0].len();
        let mut output_adapter =
            SequentialSliceOfVecs::new_mut(&mut self.output_buf, CHANNELS, output_len)
                .expect("output buffer sized to output_frames_max");
        let (_nbr_in, nbr_out) = self
            .inner
            .process_into_buffer(&input_adapter, &mut output_adapter, None)
            .expect("fixed-ratio resample never fails once constructed");
        self.output_buf[0][..nbr_out].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tone(freq_hz: f32, sample_rate: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * freq_hz * t).sin()
            })
            .collect()
    }

    #[test]
    fn downsamples_48k_to_16k_preserving_roughly_a_third_the_samples() {
        let mut down = ToTargetRate::new(48_000, 320);
        let tone = test_tone(440.0, 48_000.0, 48_000 * 2); // 2 seconds
        let chunks = down.push(&tone);
        let total_out: usize = chunks.iter().map(|c| c.len()).sum();
        // 2s of 48kHz audio should become ~2s of 16kHz audio (~32000 samples).
        let expected = 16_000 * 2;
        let tolerance = 2_000;
        assert!(
            (total_out as i64 - expected as i64).unsigned_abs() < tolerance,
            "expected ~{expected} output samples, got {total_out}"
        );
    }

    #[test]
    fn upsamples_16k_to_48k_preserving_roughly_three_times_the_samples() {
        let mut up = FromTargetRate::new(48_000, 320);
        let tone = test_tone(440.0, 16_000.0, 320);
        let mut total_out = 0usize;
        for _ in 0..50 {
            total_out += up.push_frame(&tone).len();
        }
        // 50 frames * 320 samples = 16000 samples (1s) at 16kHz -> ~48000 at 48kHz.
        let expected = 48_000;
        let tolerance = 3_000;
        assert!(
            (total_out as i64 - expected as i64).unsigned_abs() < tolerance,
            "expected ~{expected} output samples, got {total_out}"
        );
    }

    #[test]
    fn round_trip_through_both_rates_stays_non_silent() {
        let mut down = ToTargetRate::new(44_100, 320);
        let mut up = FromTargetRate::new(44_100, 320);
        let tone = test_tone(220.0, 44_100.0, 44_100); // 1 second
        let mut reconstructed = Vec::new();
        for chunk16k in down.push(&tone) {
            reconstructed.extend(up.push_frame(&chunk16k));
        }
        assert!(!reconstructed.is_empty());
        let peak = reconstructed.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            peak > 0.3,
            "round-tripped tone lost too much amplitude: peak={peak}"
        );
    }
}
