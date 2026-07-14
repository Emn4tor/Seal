//! A real-time phase-vocoder pitch shifter: the "one click" voice changer.
//! Operates on a continuous mono `f32` stream at [`crate::SAMPLE_RATE_HZ`],
//! independent of the ADPCM frame size — callers can feed it any chunk
//! size and it will emit output as full analysis hops become available.
//!
//! This is the direct spectral-bin-remapping style of phase-vocoder pitch
//! shift (STFT -> remap each bin's energy and instantaneous frequency to
//! `bin * ratio`, keeping the same hop on analysis and synthesis so
//! duration is unchanged -> ISTFT), the same family of technique described
//! in Bernsee's widely-cited "pitch shifting using the Fourier transform"
//! writeup. It is a deliberately simple, well-understood approach — not a
//! rigorous voice-anonymization tool. Large shifts or sibilant/noisy
//! speech will show real "phasiness"/robotic artifacts; that's an honest
//! property of this technique, not a bug to chase out.

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::Arc;

const FRAME_SIZE: usize = 512;
const HOP_SIZE: usize = FRAME_SIZE / 4; // 75% overlap: standard COLA-compliant spacing for a Hann window.
const NUM_BINS: usize = FRAME_SIZE / 2 + 1;

/// The fixed "deep voice" shift used by the one-click voice changer.
/// -5 semitones: audibly different without collapsing speech into mush.
pub const DEFAULT_DISGUISE_RATIO: f32 = 0.7492;

fn hann_window(len: usize) -> Vec<f32> {
    (0..len)
        .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / (len - 1) as f32).cos())
        .collect()
}

/// Simulates the actual overlap-add process on the real window/hop this
/// shifter uses, and reads back the steady-state (interior, fully-covered)
/// gain. Computed rather than recalled from a formula, so it's correct by
/// construction for whatever window/hop constants are above.
fn ola_gain(window: &[f32], hop: usize) -> f32 {
    let buf_len = window.len() * 8;
    let mut acc = vec![0.0f32; buf_len];
    let mut start = 0usize;
    while start + window.len() <= buf_len {
        for (i, &w) in window.iter().enumerate() {
            acc[start + i] += w * w;
        }
        start += hop;
    }
    acc[buf_len / 2]
}

fn wrap_phase(mut phase: f32) -> f32 {
    while phase > PI {
        phase -= 2.0 * PI;
    }
    while phase < -PI {
        phase += 2.0 * PI;
    }
    phase
}

pub struct PitchShifter {
    ratio: f32,
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    ola_gain: f32,
    input_acc: VecDeque<f32>,
    output_acc: VecDeque<f32>,
    prev_analysis_phase: Vec<f32>,
    synth_phase: Vec<f32>,
    warmed_up: bool,
}

impl PitchShifter {
    pub fn new(ratio: f32) -> Self {
        let mut planner = FftPlanner::new();
        let window = hann_window(FRAME_SIZE);
        let ola_gain = ola_gain(&window, HOP_SIZE);
        Self {
            ratio,
            fft: planner.plan_fft_forward(FRAME_SIZE),
            ifft: planner.plan_fft_inverse(FRAME_SIZE),
            window,
            ola_gain,
            input_acc: VecDeque::new(),
            output_acc: VecDeque::from(vec![0.0f32; FRAME_SIZE]),
            prev_analysis_phase: vec![0.0; NUM_BINS],
            synth_phase: vec![0.0; NUM_BINS],
            warmed_up: false,
        }
    }

    /// Feeds newly available mono samples (any chunk size). Returns
    /// pitch-shifted output — same sample count as fed in, once the
    /// initial one-window analysis delay has been primed.
    pub fn push(&mut self, samples: &[f32]) -> Vec<f32> {
        self.input_acc.extend(samples.iter().copied());
        let mut out = Vec::new();
        while self.input_acc.len() >= FRAME_SIZE {
            let frame: Vec<f32> = self.input_acc.iter().take(FRAME_SIZE).copied().collect();
            self.process_frame(&frame);
            for _ in 0..HOP_SIZE {
                self.input_acc.pop_front();
            }
            for _ in 0..HOP_SIZE {
                out.push(self.output_acc.pop_front().unwrap());
                self.output_acc.push_back(0.0);
            }
            self.warmed_up = true;
        }
        out
    }

    fn process_frame(&mut self, frame: &[f32]) {
        let mut spectrum: Vec<Complex32> = frame
            .iter()
            .zip(self.window.iter())
            .map(|(&s, &w)| Complex32::new(s * w, 0.0))
            .collect();
        self.fft.process(&mut spectrum);

        let mut analysis_mag = vec![0.0f32; NUM_BINS];
        let mut analysis_true_freq = vec![0.0f32; NUM_BINS];

        for k in 0..NUM_BINS {
            let bin = spectrum[k];
            let mag = bin.norm();
            let phase = bin.arg();

            let expected_advance = 2.0 * PI * k as f32 * HOP_SIZE as f32 / FRAME_SIZE as f32;
            let deviation = wrap_phase(phase - self.prev_analysis_phase[k] - expected_advance);
            let true_freq = 2.0 * PI * k as f32 / FRAME_SIZE as f32 + deviation / HOP_SIZE as f32;

            analysis_mag[k] = mag;
            analysis_true_freq[k] = true_freq;
            self.prev_analysis_phase[k] = phase;
        }

        let mut out_mag = vec![0.0f32; NUM_BINS];
        let mut out_freq = vec![0.0f32; NUM_BINS];
        for k_out in 0..NUM_BINS {
            let k_src = (k_out as f32 / self.ratio).round() as isize;
            if k_src >= 0 && (k_src as usize) < NUM_BINS {
                out_mag[k_out] = analysis_mag[k_src as usize];
                out_freq[k_out] = analysis_true_freq[k_src as usize] * self.ratio;
            }
        }

        let mut out_spectrum = vec![Complex32::new(0.0, 0.0); FRAME_SIZE];
        for k in 0..NUM_BINS {
            self.synth_phase[k] = wrap_phase(self.synth_phase[k] + out_freq[k] * HOP_SIZE as f32);
            let (sin, cos) = self.synth_phase[k].sin_cos();
            out_spectrum[k] = Complex32::new(out_mag[k] * cos, out_mag[k] * sin);
        }
        // Real signal in => conjugate-symmetric spectrum, so the inverse FFT comes out real.
        for k in 1..NUM_BINS - 1 {
            out_spectrum[FRAME_SIZE - k] = out_spectrum[k].conj();
        }

        self.ifft.process(&mut out_spectrum);

        let norm = 1.0 / (FRAME_SIZE as f32 * self.ola_gain);
        for ((bin, &w), acc) in out_spectrum
            .iter()
            .zip(self.window.iter())
            .zip(self.output_acc.iter_mut())
        {
            *acc += bin.re * w * norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tone(freq_hz: f32, sample_rate: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * PI * freq_hz * t).sin()
            })
            .collect()
    }

    /// Finds the dominant frequency via a plain (non-streaming) FFT over
    /// the whole buffer — an independent measurement from the shifter's
    /// own internal FFT, used only to check the result.
    fn dominant_frequency(signal: &[f32], sample_rate: f32) -> f32 {
        let n = signal.len();
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(n);
        let window = hann_window(n);
        let mut buf: Vec<Complex32> = signal
            .iter()
            .zip(window.iter())
            .map(|(&s, &w)| Complex32::new(s * w, 0.0))
            .collect();
        fft.process(&mut buf);
        let (peak_bin, _) = buf[..n / 2]
            .iter()
            .enumerate()
            .skip(1)
            .max_by(|(_, a), (_, b)| a.norm().partial_cmp(&b.norm()).unwrap())
            .unwrap();
        peak_bin as f32 * sample_rate / n as f32
    }

    #[test]
    fn shifts_a_test_tone_down_to_the_expected_frequency() {
        let sample_rate = crate::SAMPLE_RATE_HZ as f32;
        let input_freq = 200.0;
        let mut shifter = PitchShifter::new(DEFAULT_DISGUISE_RATIO);

        let seconds = 2.0;
        let tone = test_tone(input_freq, sample_rate, (sample_rate * seconds) as usize);
        let mut output = Vec::new();
        for chunk in tone.chunks(256) {
            output.extend(shifter.push(chunk));
        }

        // Drop the leading analysis-window latency and trailing partial
        // hop so only steady-state shifted signal is measured.
        let steady = &output[FRAME_SIZE.min(output.len())..];
        assert!(steady.len() > 4000, "not enough steady-state output to analyze");

        let measured = dominant_frequency(steady, sample_rate);
        let expected = input_freq * DEFAULT_DISGUISE_RATIO;
        assert!(
            (measured - expected).abs() < 5.0,
            "expected peak near {expected} Hz, measured {measured} Hz"
        );
    }

    #[test]
    fn ratio_of_one_leaves_frequency_unchanged() {
        let sample_rate = crate::SAMPLE_RATE_HZ as f32;
        let input_freq = 150.0;
        let mut shifter = PitchShifter::new(1.0);

        let tone = test_tone(input_freq, sample_rate, sample_rate as usize * 2);
        let mut output = Vec::new();
        for chunk in tone.chunks(256) {
            output.extend(shifter.push(chunk));
        }

        let steady = &output[FRAME_SIZE.min(output.len())..];
        let measured = dominant_frequency(steady, sample_rate);
        assert!(
            (measured - input_freq).abs() < 5.0,
            "expected peak near {input_freq} Hz, measured {measured} Hz"
        );
    }

    #[test]
    fn output_sample_count_tracks_input_once_warmed_up() {
        let mut shifter = PitchShifter::new(DEFAULT_DISGUISE_RATIO);
        let tone = test_tone(200.0, crate::SAMPLE_RATE_HZ as f32, 16_000);
        let mut total_out = 0usize;
        for chunk in tone.chunks(256) {
            total_out += shifter.push(chunk).len();
        }
        // Every HOP_SIZE fed in eventually yields HOP_SIZE out, modulo the
        // one-window startup delay still buffered internally.
        assert!(total_out > 16_000 - FRAME_SIZE);
        assert!(total_out <= 16_000);
    }
}
