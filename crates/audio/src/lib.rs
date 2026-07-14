//! Pure DSP for voice channels: resampling, pitch shifting, and audio
//! encode/decode. No networking, no device I/O — those live in
//! `crates/net` (stream transport) and `crates/core` (cpal + orchestration)
//! respectively, so this crate stays trivially unit-testable over plain
//! sample buffers.

pub mod adpcm;
pub mod pitch_shift;
pub mod resample;

/// The fixed internal format every voice call operates on: mono, 16-bit
/// PCM, resampled to this rate immediately after capture and back to the
/// device's native rate immediately before playback.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
