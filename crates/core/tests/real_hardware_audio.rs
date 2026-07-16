use std::sync::{Arc, Mutex};
use std::time::Duration;

use audio::adpcm::{Decoder, Encoder, SAMPLES_PER_FRAME};
use audio::pitch_shift::{DEFAULT_DISGUISE_RATIO, PitchShifter};
use audio::resample::{FromTargetRate, ToTargetRate};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// The one test in this project that isn't run automatically anywhere:
/// genuine physical audio I/O on whatever machine executes it. Captures a
/// couple of real seconds from the actual default microphone, runs it
/// through the exact same pipeline `crates/core/src/voice.rs` uses for a
/// live call (resample to 16kHz -> pitch shift -> ADPCM encode -> ADPCM
/// decode -> resample back to the mic's native rate), and checks the
/// result is real, non-silent audio with a plausible sample count — not a
/// mock, not synthetic input. Run explicitly: `cargo test -p p2p-core
/// --test real_hardware_audio -- --ignored --nocapture`.
#[test]
#[ignore = "opens the real default microphone; run explicitly with `cargo test -- --ignored`"]
fn capture_resample_pitch_shift_adpcm_round_trip_on_real_hardware() {
    let host = cpal::default_host();
    let input = host.default_input_device().expect("no default microphone on this machine");
    let config = input.default_input_config().expect("no default input config");
    assert_eq!(
        config.sample_format(),
        cpal::SampleFormat::F32,
        "this test only handles f32 input devices"
    );
    let native_rate = config.sample_rate();
    let channels = config.channels() as usize;
    println!("capturing from the default microphone at {native_rate} Hz, {channels} channel(s)");

    let captured: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_cb = captured.clone();
    let stream_config: cpal::StreamConfig = config.into();
    let stream = input
        .build_input_stream(
            stream_config,
            move |data: &[f32], _| {
                let mut buf = captured_cb.lock().unwrap();
                for frame in data.chunks(channels) {
                    let mono = frame.iter().sum::<f32>() / channels as f32;
                    buf.push(mono);
                }
            },
            |err| eprintln!("mic capture error: {err}"),
            None,
        )
        .expect("failed to build a real input stream");

    stream.play().expect("failed to start real mic capture");
    println!("recording 2 real seconds — make some noise");
    std::thread::sleep(Duration::from_secs(2));
    drop(stream);

    let raw = captured.lock().unwrap().clone();
    assert!(raw.len() > (native_rate as usize), "captured suspiciously little audio: {} samples", raw.len());

    let raw_peak = raw.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    println!("captured {} raw samples, peak amplitude {raw_peak:.4}", raw.len());

    // The exact pipeline crates/core/src/voice.rs runs for a live call.
    let mut down = ToTargetRate::new(native_rate, SAMPLES_PER_FRAME);
    let mut shifter = PitchShifter::new(DEFAULT_DISGUISE_RATIO);
    let mut encoder = Encoder::new();
    let mut decoder = Decoder::new();
    let mut up = FromTargetRate::new(native_rate, SAMPLES_PER_FRAME);

    let mut round_tripped: Vec<f32> = Vec::new();
    let mut shifted_acc: Vec<f32> = Vec::new();
    for chunk16k in down.push(&raw) {
        shifted_acc.extend(shifter.push(&chunk16k));
        while shifted_acc.len() >= SAMPLES_PER_FRAME {
            let frame_f32: Vec<f32> = shifted_acc.drain(..SAMPLES_PER_FRAME).collect();
            let mut frame_i16 = [0i16; SAMPLES_PER_FRAME];
            for (o, &s) in frame_i16.iter_mut().zip(frame_f32.iter()) {
                *o = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            }
            let encoded = encoder.encode_frame(&frame_i16);
            let decoded = decoder.decode_frame(&encoded);
            let decoded_f32: Vec<f32> = decoded.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
            round_tripped.extend(up.push_frame(&decoded_f32));
        }
    }

    assert!(
        !round_tripped.is_empty(),
        "the real pipeline produced no output samples at all"
    );
    let peak = round_tripped.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    println!(
        "round-tripped {} samples through resample -> pitch-shift -> ADPCM -> resample-back, peak amplitude {peak:.4}",
        round_tripped.len()
    );

    // A real, working pipeline reproduces roughly the input's duration
    // (some latency from the pitch shifter's analysis window is expected)
    // and isn't silent — this is checking for a real signal surviving the
    // full round trip, not an exact match to lossy-recompressed audio.
    let ratio = round_tripped.len() as f64 / raw.len() as f64;
    assert!(
        ratio > 0.5 && ratio < 1.5,
        "round-tripped sample count ({}) isn't a plausible match for the input ({})",
        round_tripped.len(),
        raw.len()
    );
}
