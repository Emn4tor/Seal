//! Real-time voice channels: mic capture -> resample -> optional pitch
//! shift -> ADPCM encode -> broadcast to every other participant's raw
//! `libp2p-stream`, and the reverse on receive, mixed and played back.
//!
//! Topology is full mesh (every participant opens a direct stream to every
//! other participant), an explicit scale ceiling appropriate for this
//! project's small-group scope, not an SFU. Mixing is a fixed 20ms-tick,
//! drop-if-late scheme (a participant with no frame ready for a given tick
//! is just treated as silent for it) rather than an adaptive jitter
//! buffer: a deliberate, disclosed simplification, not a professional-
//! grade VoIP mixer.
//!
//! Encryption here is transport-level Noise only (the same
//! authenticated, forward-secret channel every P2P connection already
//! gets); audio is *not* additionally wrapped in Olm/Megolm the way chat
//! messages are. See `docs/THREAT_MODEL.md`.
//!
//! Voice activity: the mic threshold is a noise gate applied at the
//! *sender*: a frame below threshold is simply never encoded or sent.
//! This means "is someone speaking" needs no separate detection on the
//! receive side: any frame that arrives at all is, by construction, one
//! the sender's own gate already decided was speech. Each connection just
//! tracks when its last frame arrived and reports "speaking" while that's
//! recent (a short hangover so brief gaps/jitter between frames don't
//! flicker the indicator).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use audio::adpcm::{
    BYTES_PER_FRAME, Decoder as AdpcmDecoder, Encoder as AdpcmEncoder, SAMPLES_PER_FRAME,
};
use audio::pitch_shift::{DEFAULT_DISGUISE_RATIO, PitchShifter};
use audio::resample::{FromTargetRate, ToTargetRate};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures::StreamExt as _;
use futures::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use libp2p::{PeerId, Stream};
use libp2p_stream::Control;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

/// How often a participant re-announces themselves while a call is active
/// (see `GroupPayload::VoicePresence`).
pub const PRESENCE_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(5);

/// A reasonable default noise-gate threshold: sensitive enough for normal
/// speech, not so sensitive that typical room hum/fan noise trips it.
pub const DEFAULT_MIC_THRESHOLD_DB: f32 = -50.0;
/// How long after a connection's last received frame it's still shown as
/// "speaking": bridges brief network jitter without being sluggish.
const SPEAKING_HANGOVER: std::time::Duration = std::time::Duration::from_millis(400);

const MIXER_TICK: std::time::Duration = std::time::Duration::from_millis(20);
const CAPTURE_POLL: std::time::Duration = std::time::Duration::from_millis(5);
/// How often an encoded frame actually goes out on the wire, one at a time —
/// matches `MIXER_TICK` (both are one `SAMPLES_PER_FRAME`/`SAMPLE_RATE_HZ`
/// frame's worth of real time: 320/16000s = 20ms), kept as its own named
/// constant since it's pacing a conceptually different thing (egress, not
/// the receive-side mixer). See `run_send_pacer_loop`'s doc comment for why
/// this exists at all.
const SEND_PACING_INTERVAL: std::time::Duration = std::time::Duration::from_millis(20);
/// How many encoded frames `run_send_pacer_loop` will hold before it starts
/// dropping the oldest rather than let the queue (and so the audio's
/// end-to-end latency) grow further — 240ms (12 frames * 20ms/frame),
/// generous enough to absorb a real scheduling hiccup without accumulating
/// a noticeable, ever-growing delay the way an unbounded queue would.
const MAX_QUEUED_OUTBOUND_FRAMES: usize = 12;
/// Roughly 1 second of headroom at a typical device rate: generous
/// without being unbounded; real backpressure is handled by dropping (with
/// a log line) rather than growing further.
const RING_BUFFER_CAPACITY: usize = 48_000;

type JitterMap = Arc<Mutex<HashMap<u64, VecDeque<[i16; SAMPLES_PER_FRAME]>>>>;
type Writers = Arc<AsyncMutex<Vec<(u64, WriteHalf<Stream>)>>>;
type OutboundQueue = Arc<Mutex<VecDeque<[u8; BYTES_PER_FRAME]>>>;
type LastFrameMap = Arc<Mutex<HashMap<u64, Instant>>>;

/// dBFS of a block of `[-1.0, 1.0]`-range samples, via RMS. Silence maps to
/// a very negative (not infinite/NaN) value so threshold comparisons stay
/// well-behaved.
fn dbfs(samples: &[f32]) -> f32 {
    let mean_sq = samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32;
    20.0 * mean_sq.sqrt().max(1e-9).log10()
}

/// One active voice call: owns local audio I/O and every open peer stream.
/// Dropping this tears the whole thing down: that's the "leave" mechanism.
pub struct VoiceCallState {
    pub group_id: String,
    pub channel_id: String,
    control: Control,
    muted: Arc<AtomicBool>,
    changer_enabled: Arc<AtomicBool>,
    hear_self: Arc<AtomicBool>,
    mic_threshold_db: Arc<Mutex<f32>>,
    local_speaking: Arc<AtomicBool>,
    participants: Arc<Mutex<Vec<String>>>,
    connected_peers: Arc<Mutex<HashSet<PeerId>>>,
    peer_user_ids: Arc<Mutex<HashMap<PeerId, String>>>,
    connection_user_ids: Arc<Mutex<HashMap<u64, String>>>,
    connection_last_frame_at: LastFrameMap,
    next_connection_id: Arc<AtomicU64>,
    jitter: JitterMap,
    writers: Writers,
    running: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
}

fn downmix_to_mono(frame: &[f32], channels: usize) -> f32 {
    if channels <= 1 {
        return frame.first().copied().unwrap_or(0.0);
    }
    frame.iter().take(channels).sum::<f32>() / channels as f32
}

/// Every input device name `cpal` can currently see, in host-enumeration
/// order. Best-effort: a device without a readable name is skipped rather
/// than failing the whole listing. Used by the frontend's device picker
/// (Settings -> Voice & Audio) — call fresh each time the picker opens
/// rather than caching, since devices can be plugged/unplugged between
/// calls.
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.map(|d| d.to_string()).collect())
        .unwrap_or_default()
}

/// Same as [`list_input_devices`], for playback devices.
pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.map(|d| d.to_string()).collect())
        .unwrap_or_default()
}

/// Resolves a user-preferred device by exact name match against the given
/// enumeration, falling back to the host's default when `preferred` is
/// `None` or names a device that isn't there anymore (unplugged since the
/// preference was saved, a name typo can't happen since the frontend only
/// ever offers names `list_input_devices`/`list_output_devices` actually
/// returned, but a device *disappearing* between listing and joining a call
/// is entirely normal). Silently falling back rather than erroring out
/// matches this module's existing philosophy: a voice call should still
/// start, just without the preferred device, rather than fail outright over
/// an audio preference.
fn resolve_device(
    mut devices: impl Iterator<Item = cpal::Device>,
    default: Option<cpal::Device>,
    preferred: Option<&str>,
) -> Option<cpal::Device> {
    if let Some(name) = preferred
        && let Some(device) = devices.find(|d| d.to_string() == name)
    {
        return Some(device);
    }
    default
}

/// Spawns the OS thread that owns the actual cpal devices/streams. Kept
/// entirely inside one thread (host/device/stream construction and all)
/// since `cpal::Stream` is not `Send` on every backend; the callbacks it
/// invokes only close over `rtrb`'s `Send`-safe `Producer`/`Consumer`.
/// Never fails outright: a machine with no usable microphone/speaker (no
/// device, wrong sample format, permission denied, a headless test
/// environment) still gets a working call, just one that neither captures
/// nor plays real audio. Everything else (presence, mesh dialing, stream
/// negotiation, encode/mix loops running on silence) still works, which
/// matters for testing those independently of real hardware.
///
/// `preferred_input`/`preferred_output`, when set, are matched by exact
/// device name (see [`resolve_device`]) — chosen once, at call start:
/// unlike settings like the mic threshold, there's no live "switch device
/// mid-call" support, since that means tearing down and rebuilding the
/// actual OS audio streams, not just adjusting a value a running loop reads
/// each tick.
async fn spawn_audio_io_thread(
    mic_tx: rtrb::Producer<f32>,
    speaker_rx: rtrb::Consumer<f32>,
    running: Arc<AtomicBool>,
    preferred_input: Option<String>,
    preferred_output: Option<String>,
) -> (u32, u32) {
    // A `tokio::sync::oneshot`, not `std::sync::mpsc`: real cpal device
    // setup (`build_input_stream`/`.play()` etc.) can occasionally take a
    // while, and awaiting this channel must yield back to the runtime
    // while it waits rather than blocking whatever thread is driving this
    // call: a synchronous blocking `recv()` here would stall everything
    // else on that thread (gossipsub processing included) for however long
    // setup takes, not just this call.
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<(u32, u32)>();

    std::thread::spawn(move || {
        let mut mic_tx = mic_tx;
        let mut speaker_rx = speaker_rx;
        let result = (|| -> anyhow::Result<(cpal::Stream, cpal::Stream, u32, u32)> {
            let host = cpal::default_host();
            let input = resolve_device(
                host.input_devices().into_iter().flatten(),
                host.default_input_device(),
                preferred_input.as_deref(),
            )
            .ok_or_else(|| anyhow::anyhow!("no default microphone available"))?;
            let output = resolve_device(
                host.output_devices().into_iter().flatten(),
                host.default_output_device(),
                preferred_output.as_deref(),
            )
            .ok_or_else(|| anyhow::anyhow!("no default speaker/output device available"))?;

            let input_supported = input.default_input_config()?;
            let output_supported = output.default_output_config()?;
            if input_supported.sample_format() != cpal::SampleFormat::F32
                || output_supported.sample_format() != cpal::SampleFormat::F32
            {
                anyhow::bail!(
                    "voice calls currently require f32-capable audio devices (got input={:?}, output={:?})",
                    input_supported.sample_format(),
                    output_supported.sample_format()
                );
            }
            let input_rate = input_supported.sample_rate();
            let output_rate = output_supported.sample_rate();
            let input_channels = input_supported.channels() as usize;
            let output_channels = output_supported.channels() as usize;
            let input_config: cpal::StreamConfig = input_supported.into();
            let output_config: cpal::StreamConfig = output_supported.into();

            let input_stream = input.build_input_stream(
                input_config,
                move |data: &[f32], _| {
                    for frame in data.chunks(input_channels) {
                        let mono = downmix_to_mono(frame, input_channels);
                        // Real-time-safe: drop rather than block if the
                        // consumer (encode task) has fallen behind.
                        let _ = mic_tx.push(mono);
                    }
                },
                |err| tracing::warn!(error = %err, "voice mic capture stream error"),
                None,
            )?;

            let output_stream = output.build_output_stream(
                output_config,
                move |data: &mut [f32], _| {
                    for frame in data.chunks_mut(output_channels) {
                        let sample = speaker_rx.pop().unwrap_or(0.0);
                        for s in frame {
                            *s = sample;
                        }
                    }
                },
                |err| tracing::warn!(error = %err, "voice playback stream error"),
                None,
            )?;

            input_stream.play()?;
            output_stream.play()?;
            Ok((input_stream, output_stream, input_rate, output_rate))
        })();

        match result {
            Ok((input_stream, output_stream, input_rate, output_rate)) => {
                let _ = ready_tx.send((input_rate, output_rate));
                while running.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                drop(input_stream);
                drop(output_stream);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "no usable audio device for this voice call, continuing without local mic/speaker"
                );
                let _ = ready_tx.send((audio::SAMPLE_RATE_HZ, audio::SAMPLE_RATE_HZ));
            }
        }
    });

    ready_rx
        .await
        .unwrap_or((audio::SAMPLE_RATE_HZ, audio::SAMPLE_RATE_HZ))
}

fn f32_to_i16_frame(samples: &[f32]) -> [i16; SAMPLES_PER_FRAME] {
    let mut out = [0i16; SAMPLES_PER_FRAME];
    for (o, &s) in out.iter_mut().zip(samples.iter()) {
        *o = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
    }
    out
}

fn i16_frame_to_f32(frame: &[i16; SAMPLES_PER_FRAME]) -> Vec<f32> {
    frame.iter().map(|&s| s as f32 / i16::MAX as f32).collect()
}

impl VoiceCallState {
    /// Starts local audio I/O and the always-on tasks (acceptor, encoder,
    /// mixer). Participants join the returned `Vec` empty; the caller
    /// (`AppService`) is expected to have already sent the initial
    /// `VoicePresence { joined: true }` announcement and feeds subsequent
    /// presence events in via [`Self::note_presence`].
    pub async fn start(
        group_id: String,
        channel_id: String,
        control: Control,
        preferred_input: Option<String>,
        preferred_output: Option<String>,
    ) -> anyhow::Result<Self> {
        let running = Arc::new(AtomicBool::new(true));

        let (mic_tx, mic_rx) = rtrb::RingBuffer::<f32>::new(RING_BUFFER_CAPACITY);
        let (speaker_tx, speaker_rx) = rtrb::RingBuffer::<f32>::new(RING_BUFFER_CAPACITY);

        let (input_rate, output_rate) = spawn_audio_io_thread(
            mic_tx,
            speaker_rx,
            running.clone(),
            preferred_input,
            preferred_output,
        )
        .await;

        let muted = Arc::new(AtomicBool::new(false));
        let changer_enabled = Arc::new(AtomicBool::new(false));
        let hear_self = Arc::new(AtomicBool::new(false));
        let mic_threshold_db = Arc::new(Mutex::new(DEFAULT_MIC_THRESHOLD_DB));
        let local_speaking = Arc::new(AtomicBool::new(false));
        let jitter: JitterMap = Arc::new(Mutex::new(HashMap::new()));
        let writers: Writers = Arc::new(AsyncMutex::new(Vec::new()));
        let connected_peers = Arc::new(Mutex::new(HashSet::new()));
        let peer_user_ids = Arc::new(Mutex::new(HashMap::new()));
        let connection_user_ids = Arc::new(Mutex::new(HashMap::new()));
        let connection_last_frame_at: LastFrameMap = Arc::new(Mutex::new(HashMap::new()));
        let next_connection_id = Arc::new(AtomicU64::new(0));
        // Reserved connection id for the "hear yourself" monitor loopback;
        // never assigned to a real peer connection (those start from 0 via
        // the same counter, so this sentinel must never collide with one;
        // using `u64::MAX` guarantees that regardless of how many real
        // connections a call ever has).
        let self_monitor_id = u64::MAX;
        jitter
            .lock()
            .unwrap()
            .insert(self_monitor_id, VecDeque::new());

        let outbound: OutboundQueue = Arc::new(Mutex::new(VecDeque::new()));

        let mut tasks = Vec::new();

        tasks.push(tokio::spawn(run_encode_loop(
            mic_rx,
            input_rate,
            muted.clone(),
            changer_enabled.clone(),
            hear_self.clone(),
            mic_threshold_db.clone(),
            local_speaking.clone(),
            outbound.clone(),
            jitter.clone(),
            self_monitor_id,
            running.clone(),
        )));

        tasks.push(tokio::spawn(run_send_pacer_loop(
            outbound.clone(),
            writers.clone(),
            running.clone(),
        )));

        tasks.push(tokio::spawn(run_mixer_loop(
            jitter.clone(),
            speaker_tx,
            output_rate,
            running.clone(),
        )));

        {
            let mut acceptor_control = control.clone();
            let jitter = jitter.clone();
            let writers = writers.clone();
            let next_connection_id = next_connection_id.clone();
            let peer_user_ids = peer_user_ids.clone();
            let connection_user_ids = connection_user_ids.clone();
            let connection_last_frame_at = connection_last_frame_at.clone();
            let running = running.clone();
            tasks.push(tokio::spawn(async move {
                let mut incoming = match net::accept_voice_streams(&mut acceptor_control) {
                    Ok(incoming) => incoming,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to register inbound voice stream acceptor");
                        return;
                    }
                };
                while running.load(Ordering::Relaxed) {
                    let Some((peer_id, stream)) = incoming.next().await else {
                        break;
                    };
                    let user_id = peer_user_ids.lock().unwrap().get(&peer_id).cloned();
                    register_connection(
                        stream,
                        user_id,
                        &jitter,
                        &writers,
                        &connection_user_ids,
                        &connection_last_frame_at,
                        &next_connection_id,
                    )
                    .await;
                }
            }));
        }

        Ok(Self {
            group_id,
            channel_id,
            control,
            muted,
            changer_enabled,
            hear_self,
            mic_threshold_db,
            local_speaking,
            participants: Arc::new(Mutex::new(Vec::new())),
            connected_peers,
            peer_user_ids,
            connection_user_ids,
            connection_last_frame_at,
            next_connection_id,
            jitter,
            writers,
            running,
            tasks,
        })
    }

    pub fn participants(&self) -> Vec<String> {
        self.participants.lock().unwrap().clone()
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_changer_enabled(&self, enabled: bool) {
        self.changer_enabled.store(enabled, Ordering::Relaxed);
    }

    /// "Hear yourself": loops your own (post pitch-shift, if enabled)
    /// processed mic audio back into your own speaker output, so you can
    /// check what others are hearing without needing a second person.
    pub fn set_hear_self(&self, enabled: bool) {
        self.hear_self.store(enabled, Ordering::Relaxed);
    }

    /// The noise-gate threshold, in dBFS: a captured frame quieter than
    /// this is never encoded or sent at all. Also what drives the local
    /// "you're speaking" indicator, and (indirectly, since a gated sender
    /// simply sends nothing) every other participant's indicator too.
    pub fn set_mic_threshold_db(&self, db: f32) {
        *self.mic_threshold_db.lock().unwrap() = db;
    }

    pub fn is_local_speaking(&self) -> bool {
        self.local_speaking.load(Ordering::Relaxed)
    }

    /// User ids whose voice connection has delivered a frame recently
    /// (see the module-level note on why that's a sufficient definition of
    /// "speaking": the sender's own noise gate already decided).
    pub fn speaking_participants(&self) -> Vec<String> {
        let now = Instant::now();
        let last = self.connection_last_frame_at.lock().unwrap();
        let user_ids = self.connection_user_ids.lock().unwrap();
        last.iter()
            .filter(|(_, at)| now.duration_since(**at) < SPEAKING_HANGOVER)
            .filter_map(|(connection_id, _)| user_ids.get(connection_id).cloned())
            .collect()
    }

    /// Records which user_id a resolved peer_id belongs to: needed so an
    /// *inbound* connection (which only ever hands us a bare `PeerId`, see
    /// the acceptor loop in [`Self::start`]) can still be attributed to a
    /// user_id for [`Self::speaking_participants`]. Callers (`AppService`)
    /// already do this exact resolution via the directory to dial in the
    /// first place; this just remembers it here too.
    pub fn note_peer_identity(&self, peer_id: PeerId, user_id: String) {
        self.peer_user_ids.lock().unwrap().insert(peer_id, user_id);
    }

    /// Updates the presence-driven participant list. Returns `true` if the
    /// set actually changed (so the caller knows whether to emit
    /// `ChatEvent::VoiceParticipantsChanged`).
    pub fn note_presence(&self, user_id: &str, joined: bool) -> bool {
        let mut participants = self.participants.lock().unwrap();
        let had = participants.iter().any(|u| u == user_id);
        if joined && !had {
            participants.push(user_id.to_string());
            true
        } else if !joined && had {
            participants.retain(|u| u != user_id);
            true
        } else {
            false
        }
    }

    /// Opens a stream to a resolved participant, if we don't already have
    /// one. Callers (`AppService`) are responsible for the "who dials whom"
    /// tie-break (lexicographically smaller user_id initiates) and for
    /// having already registered `peer_id`'s address on the swarm.
    pub async fn ensure_connected(&self, peer_id: PeerId, user_id: String) -> anyhow::Result<()> {
        {
            let mut connected = self.connected_peers.lock().unwrap();
            if connected.contains(&peer_id) {
                return Ok(());
            }
            connected.insert(peer_id);
        }
        let mut control = self.control.clone();
        let stream = net::open_voice_stream(&mut control, peer_id).await?;
        register_connection(
            stream,
            Some(user_id),
            &self.jitter,
            &self.writers,
            &self.connection_user_ids,
            &self.connection_last_frame_at,
            &self.next_connection_id,
        )
        .await;
        Ok(())
    }

    /// Same dial as [`Self::ensure_connected`], but fire-and-forget on an
    /// independent task instead of awaited inline.
    ///
    /// `Control::open_stream` needs the *swarm* to be polled again while it
    /// awaits its own internal channel round-trip: fine when something
    /// else keeps driving the swarm concurrently, but `ChatNode::next_event`
    /// is the *only* thing that ever polls it, called request/response
    /// style from the outside. Awaiting `ensure_connected` from anywhere
    /// inside that same call chain (as `AppService::handle_voice_presence`
    /// does, reacting to a just-received `VoicePresence`) is a genuine
    /// deadlock: the swarm can't be polled again until this call returns,
    /// and this call can't return until the swarm is polled again.
    /// Spawning it lets `next_event` return immediately and keep polling
    /// the swarm on the caller's own next iteration, which is what
    /// eventually unblocks the spawned dial.
    pub fn spawn_ensure_connected(&self, peer_id: PeerId, user_id: String) {
        {
            let mut connected = self.connected_peers.lock().unwrap();
            if connected.contains(&peer_id) {
                return;
            }
            connected.insert(peer_id);
        }
        let mut control = self.control.clone();
        let jitter = self.jitter.clone();
        let writers = self.writers.clone();
        let connection_user_ids = self.connection_user_ids.clone();
        let connection_last_frame_at = self.connection_last_frame_at.clone();
        let next_connection_id = self.next_connection_id.clone();
        tokio::spawn(async move {
            match net::open_voice_stream(&mut control, peer_id).await {
                Ok(stream) => {
                    register_connection(
                        stream,
                        Some(user_id),
                        &jitter,
                        &writers,
                        &connection_user_ids,
                        &connection_last_frame_at,
                        &next_connection_id,
                    )
                    .await
                }
                Err(e) => {
                    tracing::warn!(error = %e, %peer_id, "failed to open a voice stream to a participant")
                }
            }
        });
    }
}

impl Drop for VoiceCallState {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn register_connection(
    stream: Stream,
    user_id: Option<String>,
    jitter: &JitterMap,
    writers: &Writers,
    connection_user_ids: &Arc<Mutex<HashMap<u64, String>>>,
    connection_last_frame_at: &LastFrameMap,
    next_id: &Arc<AtomicU64>,
) {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let (read_half, write_half) = stream.split();
    jitter.lock().unwrap().insert(id, VecDeque::new());
    if let Some(user_id) = user_id {
        connection_user_ids.lock().unwrap().insert(id, user_id);
    }
    writers.lock().await.push((id, write_half));
    tokio::spawn(run_reader_loop(
        id,
        read_half,
        jitter.clone(),
        connection_last_frame_at.clone(),
    ));
}

/// Reads fixed-size ADPCM frames off one peer's stream, decodes them with a
/// decoder scoped to just this connection (ADPCM state must not be shared
/// across peers — see `audio::adpcm`), and queues the decoded PCM for the
/// mixer.
async fn run_reader_loop(
    connection_id: u64,
    mut read_half: ReadHalf<Stream>,
    jitter: JitterMap,
    last_frame_at: LastFrameMap,
) {
    let mut decoder = AdpcmDecoder::new();
    let mut buf = [0u8; BYTES_PER_FRAME];
    loop {
        if read_half.read_exact(&mut buf).await.is_err() {
            break; // peer disconnected/left — just stop feeding the mixer for this connection.
        }
        let frame = decoder.decode_frame(&buf);
        last_frame_at
            .lock()
            .unwrap()
            .insert(connection_id, Instant::now());
        if let Some(queue) = jitter.lock().unwrap().get_mut(&connection_id) {
            queue.push_back(frame);
            // Bound memory if the mixer somehow stalls; a real jitter
            // buffer would size this adaptively, this is the simple version.
            while queue.len() > 50 {
                queue.pop_front();
            }
        }
    }
    jitter.lock().unwrap().remove(&connection_id);
    last_frame_at.lock().unwrap().remove(&connection_id);
}

/// Drains captured mic audio, resamples to the fixed internal rate,
/// optionally pitch-shifts, gates it against the noise threshold, and
/// ADPCM-encodes it — handing every resulting frame to `outbound` rather
/// than writing it to peers directly (see `run_send_pacer_loop` for why
/// that split exists), plus, if "hear yourself" is on, looping a copy into
/// the mixer's self-monitor queue.
#[allow(clippy::too_many_arguments)]
async fn run_encode_loop(
    mut mic_rx: rtrb::Consumer<f32>,
    input_rate: u32,
    muted: Arc<AtomicBool>,
    changer_enabled: Arc<AtomicBool>,
    hear_self: Arc<AtomicBool>,
    mic_threshold_db: Arc<Mutex<f32>>,
    local_speaking: Arc<AtomicBool>,
    outbound: OutboundQueue,
    jitter: JitterMap,
    self_monitor_id: u64,
    running: Arc<AtomicBool>,
) {
    let mut resampler = ToTargetRate::new(input_rate, SAMPLES_PER_FRAME);
    let mut shifter = PitchShifter::new(DEFAULT_DISGUISE_RATIO);
    let mut shifted_acc: Vec<f32> = Vec::new();
    let mut encoder = AdpcmEncoder::new();
    let mut drained = Vec::with_capacity(4096);

    while running.load(Ordering::Relaxed) {
        tokio::time::sleep(CAPTURE_POLL).await;
        drained.clear();
        while let Ok(sample) = mic_rx.pop() {
            drained.push(sample);
        }
        if drained.is_empty() {
            continue;
        }
        if muted.load(Ordering::Relaxed) {
            local_speaking.store(false, Ordering::Relaxed);
            continue; // still drain the ring buffer above, just don't encode/send it.
        }

        for chunk in resampler.push(&drained) {
            let frame_samples: Vec<f32> = if changer_enabled.load(Ordering::Relaxed) {
                shifted_acc.extend(shifter.push(&chunk));
                if shifted_acc.len() < SAMPLES_PER_FRAME {
                    continue;
                }
                shifted_acc.drain(..SAMPLES_PER_FRAME).collect()
            } else {
                chunk
            };

            let threshold = *mic_threshold_db.lock().unwrap();
            let speaking = dbfs(&frame_samples) >= threshold;
            local_speaking.store(speaking, Ordering::Relaxed);
            if !speaking {
                continue; // the noise gate: below threshold, don't send this frame at all.
            }

            if hear_self.load(Ordering::Relaxed)
                && let Some(queue) = jitter.lock().unwrap().get_mut(&self_monitor_id)
            {
                queue.push_back(f32_to_i16_frame(&frame_samples));
                while queue.len() > 10 {
                    queue.pop_front();
                }
            }

            let frame = f32_to_i16_frame(&frame_samples);
            let encoded = encoder.encode_frame(&frame);

            let mut queue = outbound.lock().unwrap();
            queue.push_back(encoded);
            // Same "drop the oldest, not the newest" backpressure policy as
            // the receive-side jitter buffer: if this is backing up, the
            // frames worth keeping are the most recent ones, not whatever's
            // been waiting longest.
            while queue.len() > MAX_QUEUED_OUTBOUND_FRAMES {
                queue.pop_front();
            }
        }
    }
}

/// The other half of `run_encode_loop`: takes whatever frames it queued and
/// actually puts them on the wire, exactly one per tick.
///
/// This split exists because processing (capture -> resample -> pitch-shift
/// -> gate -> encode) and transmission have different timing needs.
/// Processing only needs to keep up *on average*; a `CAPTURE_POLL` cycle
/// delayed by scheduling jitter (another task hogging the runtime, a GC-like
/// pause, anything) just processes a bigger batch next time, which is fine
/// for CPU work. But writing straight to the peer socket the moment each
/// frame was ready — what this code used to do — turns that same delay into
/// several frames' worth of audio landing on the wire back-to-back, then a
/// gap, then another burst: audible as exactly the stutter/lag this exists
/// to fix, since the mixer on the *receiving* end pulls at most one frame
/// per fixed 20ms tick (see `run_mixer_loop`) and has no jitter buffer to
/// smooth a bursty arrival pattern back out. Pacing egress here — one frame
/// per tick, unconditionally, regardless of how processing happened to be
/// batched — keeps what actually reaches the network evenly spaced, which
/// is what the receiving mixer actually needs.
async fn run_send_pacer_loop(outbound: OutboundQueue, writers: Writers, running: Arc<AtomicBool>) {
    let mut ticker = tokio::time::interval(SEND_PACING_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    while running.load(Ordering::Relaxed) {
        ticker.tick().await;

        let Some(encoded) = outbound.lock().unwrap().pop_front() else {
            continue; // nothing captured this tick — legitimately silent, not an underrun to react to.
        };

        let mut guard = writers.lock().await;
        let mut failed = Vec::new();
        for (id, writer) in guard.iter_mut() {
            if writer.write_all(&encoded).await.is_err() {
                failed.push(*id);
            }
        }
        if !failed.is_empty() {
            guard.retain(|(id, _)| !failed.contains(id));
        }
    }
}

/// Every tick, sums whatever each connected peer (plus the self-monitor
/// loopback, if enabled) has ready — silence if nothing's arrived yet, the
/// "drop if late" half of this mixer's deliberately simple jitter handling
/// — resamples the mix to the output device's native rate, and pushes it
/// to the playback ring buffer.
async fn run_mixer_loop(
    jitter: JitterMap,
    mut speaker_tx: rtrb::Producer<f32>,
    output_rate: u32,
    running: Arc<AtomicBool>,
) {
    let mut resampler = FromTargetRate::new(output_rate, SAMPLES_PER_FRAME);
    let mut ticker = tokio::time::interval(MIXER_TICK);

    while running.load(Ordering::Relaxed) {
        ticker.tick().await;

        let mut mixed = [0i32; SAMPLES_PER_FRAME];
        let mut any = false;
        {
            let mut guard = jitter.lock().unwrap();
            for queue in guard.values_mut() {
                if let Some(frame) = queue.pop_front() {
                    any = true;
                    for (m, s) in mixed.iter_mut().zip(frame.iter()) {
                        *m += *s as i32;
                    }
                }
            }
        }
        if !any {
            continue;
        }

        let clamped: [i16; SAMPLES_PER_FRAME] =
            std::array::from_fn(|i| mixed[i].clamp(i16::MIN as i32, i16::MAX as i32) as i16);
        let native_samples = resampler.push_frame(&i16_frame_to_f32(&clamped));
        for s in native_samples {
            let _ = speaker_tx.push(s);
        }
    }
}
