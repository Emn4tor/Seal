//! IMA ADPCM: a decades-old, well-documented 4:1 lossy compressor for
//! 16-bit PCM audio. Chosen over an Opus binding since every mature Rust
//! Opus crate wraps the real C library and needs cmake/system libopus to
//! build — the same native-build fragility this project already avoided
//! once (SQLCipher -> plain bundled SQLite). ADPCM is simple enough to
//! implement correctly in a hundred-odd lines with zero dependencies, at
//! the cost of noticeably less crisp voice quality than Opus lol.
//!
//! Each `Encoder`/`Decoder` holds running state that evolves across every
//! frame passed through it — make a fresh pair per logical stream (e.g. a
//! new voice call), not shared across unrelated ones.

const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

const INDEX_TABLE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

/// One 20ms frame at 16kHz mono — the fixed unit this codec (and the voice
/// transport above it) always operates on.
pub const SAMPLES_PER_FRAME: usize = 320;
/// Two 4-bit nibbles per byte.
pub const BYTES_PER_FRAME: usize = SAMPLES_PER_FRAME / 2;

pub struct Encoder {
    predictor: i32,
    step_index: i32,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    pub fn new() -> Self {
        Self {
            predictor: 0,
            step_index: 0,
        }
    }

    /// Encodes exactly [`SAMPLES_PER_FRAME`] samples into [`BYTES_PER_FRAME`]
    /// bytes. Panics if `samples.len() != SAMPLES_PER_FRAME` — callers
    /// control framing (see `crates/core/src/voice.rs`) and always pass a
    /// full frame.
    pub fn encode_frame(&mut self, samples: &[i16]) -> [u8; BYTES_PER_FRAME] {
        assert_eq!(samples.len(), SAMPLES_PER_FRAME);
        let mut out = [0u8; BYTES_PER_FRAME];
        for (i, chunk) in samples.chunks(2).enumerate() {
            let lo = self.encode_sample(chunk[0]);
            let hi = self.encode_sample(chunk[1]);
            out[i] = lo | (hi << 4);
        }
        out
    }

    fn encode_sample(&mut self, sample: i16) -> u8 {
        let sample = sample as i32;
        let step = STEP_TABLE[self.step_index as usize];

        let mut diff = sample - self.predictor;
        let sign = if diff < 0 {
            diff = -diff;
            8
        } else {
            0
        };

        let mut nibble = 0u8;
        let mut vpdiff = step >> 3;
        let mut s = step;

        if diff >= s {
            nibble |= 4;
            diff -= s;
            vpdiff += s;
        }
        s >>= 1;
        if diff >= s {
            nibble |= 2;
            diff -= s;
            vpdiff += s;
        }
        s >>= 1;
        if diff >= s {
            nibble |= 1;
            vpdiff += s;
        }

        if sign != 0 {
            self.predictor -= vpdiff;
        } else {
            self.predictor += vpdiff;
        }
        self.predictor = self.predictor.clamp(-32768, 32767);

        nibble |= sign;
        self.step_index = (self.step_index + INDEX_TABLE[nibble as usize]).clamp(0, 88);

        nibble
    }
}

pub struct Decoder {
    predictor: i32,
    step_index: i32,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            predictor: 0,
            step_index: 0,
        }
    }

    /// Decodes exactly [`BYTES_PER_FRAME`] bytes into [`SAMPLES_PER_FRAME`]
    /// samples. Panics if `bytes.len() != BYTES_PER_FRAME`.
    pub fn decode_frame(&mut self, bytes: &[u8]) -> [i16; SAMPLES_PER_FRAME] {
        assert_eq!(bytes.len(), BYTES_PER_FRAME);
        let mut out = [0i16; SAMPLES_PER_FRAME];
        for (i, &byte) in bytes.iter().enumerate() {
            out[i * 2] = self.decode_sample(byte & 0x0f);
            out[i * 2 + 1] = self.decode_sample((byte >> 4) & 0x0f);
        }
        out
    }

    fn decode_sample(&mut self, nibble: u8) -> i16 {
        let step = STEP_TABLE[self.step_index as usize];
        let sign = nibble & 8;
        let delta = nibble & 7;

        let mut vpdiff = step >> 3;
        let mut s = step;
        if delta & 4 != 0 {
            vpdiff += s;
        }
        s >>= 1;
        if delta & 2 != 0 {
            vpdiff += s;
        }
        s >>= 1;
        if delta & 1 != 0 {
            vpdiff += s;
        }

        if sign != 0 {
            self.predictor -= vpdiff;
        } else {
            self.predictor += vpdiff;
        }
        self.predictor = self.predictor.clamp(-32768, 32767);

        self.step_index = (self.step_index + INDEX_TABLE[nibble as usize]).clamp(0, 88);

        self.predictor as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tone(freq_hz: f32, sample_rate: f32, n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (8000.0 * (2.0 * std::f32::consts::PI * freq_hz * t).sin()) as i16
            })
            .collect()
    }

    #[test]
    fn round_trip_reconstructs_a_tone_within_quantization_error() {
        let original = test_tone(220.0, 16000.0, SAMPLES_PER_FRAME * 3);
        let mut encoder = Encoder::new();
        let mut decoder = Decoder::new();

        let mut decoded = Vec::new();
        for frame in original.chunks(SAMPLES_PER_FRAME) {
            let encoded = encoder.encode_frame(frame);
            decoded.extend_from_slice(&decoder.decode_frame(&encoded));
        }

        assert_eq!(decoded.len(), original.len());

        // Worst-case single-sample error is a poor metric for a lossy
        // codec: IMA ADPCM has known "slope overload" transients right
        // after a peak, where the step size needs a few samples to shrink
        // back down. RMSE over the whole signal is the metric that
        // actually reflects perceptual fidelity, and is what distinguishes
        // "working codec" from "broken codec" (which would show up as an
        // RMSE orders of magnitude larger, not a handful of rough samples).
        let mean_sq_error: f64 = original
            .iter()
            .zip(decoded.iter())
            .map(|(a, b)| ((*a as f64) - (*b as f64)).powi(2))
            .sum::<f64>()
            / original.len() as f64;
        let rmse = mean_sq_error.sqrt();
        assert!(
            rmse < 500.0,
            "RMSE {rmse} too high for a clean 8000-peak tone"
        );

        // And it shouldn't have decayed to silence.
        let decoded_peak = decoded.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(
            decoded_peak > 5000,
            "decoded signal went quiet: {decoded_peak}"
        );
    }

    #[test]
    fn silence_encodes_and_decodes_to_silence() {
        let silence = [0i16; SAMPLES_PER_FRAME];
        let mut encoder = Encoder::new();
        let mut decoder = Decoder::new();
        let encoded = encoder.encode_frame(&silence);
        let decoded = decoder.decode_frame(&encoded);
        assert!(decoded.iter().all(|&s| s.abs() < 10));
    }

    #[test]
    fn frame_size_is_correct() {
        assert_eq!(BYTES_PER_FRAME, 160);
        let mut encoder = Encoder::new();
        let encoded = encoder.encode_frame(&[0i16; SAMPLES_PER_FRAME]);
        assert_eq!(encoded.len(), BYTES_PER_FRAME);
    }
}
