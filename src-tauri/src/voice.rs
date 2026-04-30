/*!
 * Local voice input.
 *
 * Push-to-talk transcription using whisper.cpp via the `whisper-rs` crate.
 * Audio capture goes through `cpal` (WASAPI on Windows, CoreAudio on
 * macOS); samples are downmixed to mono and resampled to the 16 kHz f32
 * PCM format whisper expects, then fed to the model on release.
 *
 * v0 is push-to-talk only: hold the hotkey, talk, release, get the full
 * transcript piped into the input field. Streaming partials are not
 * emitted — whisper inference on a non-trivial model (>= base) is CPU
 * heavy enough that running it on every chunk would compete with the
 * model the user is about to chat with. Partials land in a future
 * iteration once we have a smaller streaming-friendly model wired up.
 *
 * Models live at `<app_data_dir>/whisper-models/<filename>` and are
 * downloaded on first use through `download_whisper_model` (mirrors the
 * pattern in `model_pull.rs`). The user picks which one is active in
 * Settings → Voice.
 */

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// Sample rate required by whisper.cpp.
const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Per-chunk no-progress timeout for the model download stream. If the
/// HF mirror goes silent the user gets a clear error instead of a
/// frozen progress bar.
const DOWNLOAD_CHUNK_TIMEOUT: Duration = Duration::from_secs(60);

/// Top-level cap on a single download. Generous; only here to catch
/// connections that hang forever.
const DOWNLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(60 * 30);

/// Hard cap on how long a single push-to-talk recording can run. Above
/// this the transcription cost grows linearly and the user is almost
/// certainly stuck — release the hotkey already.
const MAX_RECORDING_SECS: u64 = 300;

/// Whisper.cpp model file base URL. The repo hosts every quantization
/// variant under predictable filenames (`ggml-<size>[.en][-q5_1].bin`).
const WHISPER_MODEL_BASE_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Returns the on-disk directory for whisper model files. Creates it
/// if missing.
fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data dir: {e}"))?;
    let dir = base.join("whisper-models");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create models dir: {e}"))?;
    }
    Ok(dir)
}

/// Validates that a filename looks like a whisper.cpp ggml model and
/// contains no path separators. Defense-in-depth against a malicious or
/// buggy frontend asking us to write outside the models directory.
fn validate_model_filename(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("model filename is empty".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("model filename contains illegal characters".to_string());
    }
    if !name.starts_with("ggml-") || !name.ends_with(".bin") {
        return Err("model filename must start with 'ggml-' and end with '.bin'".to_string());
    }
    Ok(())
}

// ─── Recording session state ────────────────────────────────────────────

/// Signal sent from the frontend's release-or-cancel handler into the
/// recording loop.
enum FinishMode {
    /// Stop capture and run whisper on the captured audio.
    Finalize,
    /// Stop capture and discard.
    Cancel,
}

/// Live recording session, parked in `VoiceState` while audio is being
/// captured. Only one session may exist at a time per Wren instance.
struct VoiceSession {
    /// Sender used by `voice_finalize` / `voice_cancel` to wake the
    /// recording loop.
    finish_tx: Option<oneshot::Sender<FinishMode>>,
}

/// Tauri-managed voice state. A new `VoiceSession` is parked here when
/// `voice_record` starts; the same session is taken back out by
/// `voice_finalize` / `voice_cancel` to send the stop signal.
#[derive(Default)]
pub struct VoiceState {
    inner: Mutex<Option<VoiceSession>>,
    /// Cancellation token for an in-flight model download. Reset on
    /// each `download_whisper_model` invocation.
    download_cancel: Mutex<Option<CancellationToken>>,
}

impl VoiceState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces (or installs) the active session, returning any prior
    /// session's finish sender so the caller can drain it on the
    /// "previous capture got stuck" code path.
    fn install(&self, session: VoiceSession) -> Option<VoiceSession> {
        let mut guard = self.inner.lock().expect("voice state mutex poisoned");
        guard.replace(session)
    }

    /// Removes the active session and returns its finish sender if one
    /// exists. Used by the finalize/cancel commands.
    fn take_finish(&self) -> Option<oneshot::Sender<FinishMode>> {
        let mut guard = self.inner.lock().expect("voice state mutex poisoned");
        guard.as_mut().and_then(|s| s.finish_tx.take())
    }

    /// Drops the parked session entirely. Called from the recording
    /// loop on its own way out so a stale entry doesn't survive past
    /// the loop.
    fn clear(&self) {
        let mut guard = self.inner.lock().expect("voice state mutex poisoned");
        *guard = None;
    }

    fn set_download_cancel(&self, token: CancellationToken) {
        let mut guard = self
            .download_cancel
            .lock()
            .expect("voice download mutex poisoned");
        *guard = Some(token);
    }

    fn take_download_cancel(&self) -> Option<CancellationToken> {
        let mut guard = self
            .download_cancel
            .lock()
            .expect("voice download mutex poisoned");
        guard.take()
    }
}

// ─── Recording / transcription events ───────────────────────────────────

/// Stream events emitted to the frontend during a record-and-transcribe
/// session. Mirrors the typed-Channel pattern used by `model_pull`.
#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum VoiceEvent {
    /// Capture has started. The frontend should show a "listening"
    /// indicator and arm its release handler.
    Listening,
    /// Capture has stopped; whisper is running. Useful so the UI can
    /// show "transcribing…" while the model warms up.
    Transcribing,
    /// Final transcript ready. Carries the trimmed text; empty-string
    /// is permitted (whisper emits it for silence) and the UI should
    /// treat it as a no-op rather than overwriting the input field.
    Final(String),
    /// User cancelled (released cancel hotkey, closed overlay, etc.).
    /// No transcription was attempted.
    Cancelled,
    /// Recoverable failure surfaced as a user-facing message. The
    /// session is over either way.
    Error(String),
}

// ─── Model download events ──────────────────────────────────────────────

/// Stream events emitted while a whisper model is being pulled from
/// HuggingFace. Same shape as `PullEvent` in `model_pull.rs` but kept
/// separate so the two pipelines can evolve independently.
#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum VoiceModelDownloadEvent {
    Status(String),
    Progress { total: u64, completed: u64 },
    Done,
    Cancelled,
    Error(String),
}

// ─── Installed models listing ───────────────────────────────────────────

/// One-line summary of an installed whisper model file. Returned to the
/// Settings UI by `list_whisper_models`.
#[derive(Clone, Serialize, Deserialize)]
pub struct InstalledWhisperModel {
    /// Filename on disk (e.g. `ggml-base.en-q5_1.bin`). Round-tripped
    /// straight back into `voice_record` and `delete_whisper_model`.
    pub filename: String,
    /// Size on disk in bytes.
    pub size: u64,
}

/// Returns a sorted list of every `ggml-*.bin` file in the models
/// directory. Errors are non-fatal: a missing or unreadable directory
/// returns an empty list.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn list_whisper_models(app: AppHandle) -> Vec<InstalledWhisperModel> {
    let Ok(dir) = models_dir(&app) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<InstalledWhisperModel> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if validate_model_filename(&name).is_err() {
                return None;
            }
            let size = e.metadata().ok().map(|m| m.len()).unwrap_or(0);
            Some(InstalledWhisperModel { filename: name, size })
        })
        .collect();
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    out
}

/// Removes a single model file from disk. Validates the filename to
/// prevent path traversal. No-op if the file is already gone.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn delete_whisper_model(app: AppHandle, filename: String) -> Result<(), String> {
    validate_model_filename(&filename)?;
    let dir = models_dir(&app)?;
    let path = dir.join(&filename);
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path)
        .map_err(|e| format!("could not delete {filename}: {e}"))?;
    Ok(())
}

// ─── Model download ────────────────────────────────────────────────────

/// Pulls a whisper model file from the public ggerganov/whisper.cpp
/// HuggingFace repo, streaming byte progress to the frontend. The file
/// lands at `<app_data_dir>/whisper-models/<filename>`.
///
/// Progress is approximate: HF returns Content-Length on the redirect
/// target, so `total` is known up front.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn download_whisper_model(
    app: AppHandle,
    filename: String,
    on_event: Channel<VoiceModelDownloadEvent>,
    client: State<'_, reqwest::Client>,
    voice: State<'_, VoiceState>,
) -> Result<(), String> {
    if let Err(e) = validate_model_filename(&filename) {
        let _ = on_event.send(VoiceModelDownloadEvent::Error(e));
        return Ok(());
    }

    let dir = match models_dir(&app) {
        Ok(d) => d,
        Err(e) => {
            let _ = on_event.send(VoiceModelDownloadEvent::Error(e));
            return Ok(());
        }
    };
    let final_path = dir.join(&filename);
    if final_path.exists() {
        let _ = on_event.send(VoiceModelDownloadEvent::Status(
            "Already installed.".to_string(),
        ));
        let _ = on_event.send(VoiceModelDownloadEvent::Done);
        return Ok(());
    }
    let temp_path = dir.join(format!("{filename}.part"));

    let cancel = CancellationToken::new();
    voice.set_download_cancel(cancel.clone());

    let url = format!("{WHISPER_MODEL_BASE_URL}/{filename}");
    let _ = on_event.send(VoiceModelDownloadEvent::Status(format!(
        "Downloading {filename}…"
    )));

    let res = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let _ = on_event.send(VoiceModelDownloadEvent::Cancelled);
            return Ok(());
        }
        r = client.get(&url).timeout(DOWNLOAD_TOTAL_TIMEOUT).send() => r,
    };

    let response = match res {
        Ok(r) => r,
        Err(e) => {
            let _ = on_event.send(VoiceModelDownloadEvent::Error(format!(
                "Could not reach HuggingFace: {e}"
            )));
            return Ok(());
        }
    };

    if !response.status().is_success() {
        let _ = on_event.send(VoiceModelDownloadEvent::Error(format!(
            "HuggingFace returned HTTP {}",
            response.status().as_u16()
        )));
        return Ok(());
    }

    let total = response.content_length().unwrap_or(0);
    let mut completed: u64 = 0;
    let mut stream = response.bytes_stream();

    use std::io::Write;
    let mut file = match std::fs::File::create(&temp_path) {
        Ok(f) => f,
        Err(e) => {
            let _ = on_event.send(VoiceModelDownloadEvent::Error(format!(
                "Could not open destination file: {e}"
            )));
            return Ok(());
        }
    };

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                drop(file);
                let _ = std::fs::remove_file(&temp_path);
                let _ = on_event.send(VoiceModelDownloadEvent::Cancelled);
                return Ok(());
            }
            timed = tokio::time::timeout(DOWNLOAD_CHUNK_TIMEOUT, stream.next()) => {
                let chunk_opt = match timed {
                    Ok(opt) => opt,
                    Err(_) => {
                        drop(file);
                        let _ = std::fs::remove_file(&temp_path);
                        let _ = on_event.send(VoiceModelDownloadEvent::Error(format!(
                            "Download stalled: no progress for {}s.",
                            DOWNLOAD_CHUNK_TIMEOUT.as_secs()
                        )));
                        return Ok(());
                    }
                };
                match chunk_opt {
                    Some(Ok(bytes)) => {
                        if let Err(e) = file.write_all(&bytes) {
                            drop(file);
                            let _ = std::fs::remove_file(&temp_path);
                            let _ = on_event.send(VoiceModelDownloadEvent::Error(format!(
                                "Disk write failed: {e}"
                            )));
                            return Ok(());
                        }
                        completed += bytes.len() as u64;
                        let _ = on_event.send(VoiceModelDownloadEvent::Progress {
                            total,
                            completed,
                        });
                    }
                    Some(Err(e)) => {
                        drop(file);
                        let _ = std::fs::remove_file(&temp_path);
                        let _ = on_event.send(VoiceModelDownloadEvent::Error(format!(
                            "Network error during download: {e}"
                        )));
                        return Ok(());
                    }
                    None => {
                        if let Err(e) = file.flush() {
                            drop(file);
                            let _ = std::fs::remove_file(&temp_path);
                            let _ = on_event.send(VoiceModelDownloadEvent::Error(format!(
                                "Could not flush: {e}"
                            )));
                            return Ok(());
                        }
                        drop(file);
                        if let Err(e) = std::fs::rename(&temp_path, &final_path) {
                            let _ = std::fs::remove_file(&temp_path);
                            let _ = on_event.send(VoiceModelDownloadEvent::Error(format!(
                                "Could not finalize file: {e}"
                            )));
                            return Ok(());
                        }
                        let _ = on_event.send(VoiceModelDownloadEvent::Done);
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// Cancels an in-flight `download_whisper_model`. No-op if no download
/// is running.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn cancel_whisper_download(voice: State<'_, VoiceState>) {
    if let Some(token) = voice.take_download_cancel() {
        token.cancel();
    }
}

// ─── Recording + transcription ─────────────────────────────────────────

/// Starts capturing audio from the default input device and parks the
/// session in `VoiceState`. Returns immediately — the recording loop
/// runs as a Tokio task spawned by this command.
///
/// The frontend signals end-of-capture by calling `voice_finalize` (run
/// the model) or `voice_cancel` (drop the audio).
///
/// Idempotent on the front end: if a session is already parked, this
/// command takes its finish sender and signals Cancel before installing
/// the new one. That means a hotkey re-press while a stuck session is
/// alive resets cleanly instead of stacking sessions.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn voice_record(
    app: AppHandle,
    model_filename: String,
    on_event: Channel<VoiceEvent>,
    voice: State<'_, VoiceState>,
) -> Result<(), String> {
    if let Err(e) = validate_model_filename(&model_filename) {
        let _ = on_event.send(VoiceEvent::Error(e));
        return Ok(());
    }
    let dir = match models_dir(&app) {
        Ok(d) => d,
        Err(e) => {
            let _ = on_event.send(VoiceEvent::Error(e));
            return Ok(());
        }
    };
    let model_path = dir.join(&model_filename);
    if !model_path.exists() {
        let _ = on_event.send(VoiceEvent::Error(format!(
            "Voice model not installed: {model_filename}. Install it from Settings → Voice."
        )));
        return Ok(());
    }

    // Drain a previous session if one is still parked. Should not
    // happen in normal use — the frontend tracks its own armed state —
    // but kills runaway sessions instead of stacking them.
    let (finish_tx, finish_rx) = oneshot::channel::<FinishMode>();
    if let Some(prev) = voice.install(VoiceSession {
        finish_tx: Some(finish_tx),
    }) {
        if let Some(tx) = prev.finish_tx {
            let _ = tx.send(FinishMode::Cancel);
        }
    }

    let _ = on_event.send(VoiceEvent::Listening);

    // Spawn the audio capture + transcription pipeline. The cpal
    // `Stream` is !Send so it must live on a dedicated OS thread; the
    // async outer task is the one that owns the oneshot.
    let voice_state_handle = app.state::<VoiceState>();
    let _ = voice_state_handle; // touch to confirm it exists in tests; not used downstream
    let app_for_clear = app.clone();

    tauri::async_runtime::spawn(async move {
        let result = run_recording_session(model_path, on_event.clone(), finish_rx).await;
        // Always clear the parked session — even on error — so the
        // next hotkey press starts cleanly.
        if let Some(state) = app_for_clear.try_state::<VoiceState>() {
            state.clear();
        }
        if let Err(e) = result {
            let _ = on_event.send(VoiceEvent::Error(e));
        }
    });

    Ok(())
}

/// Signals the active recording session to stop capture and run
/// whisper on the buffered audio. No-op if no session is parked.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn voice_finalize(voice: State<'_, VoiceState>) {
    if let Some(tx) = voice.take_finish() {
        let _ = tx.send(FinishMode::Finalize);
    }
}

/// Signals the active recording session to stop and discard the audio.
/// No-op if no session is parked.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn voice_cancel(voice: State<'_, VoiceState>) {
    if let Some(tx) = voice.take_finish() {
        let _ = tx.send(FinishMode::Cancel);
    }
}

/// Runs the full record → finish-signal → transcribe pipeline. Returns
/// `Err` only on an unrecoverable failure (model load, audio device);
/// the recoverable cancel path emits `Cancelled` and returns `Ok`.
async fn run_recording_session(
    model_path: PathBuf,
    on_event: Channel<VoiceEvent>,
    finish_rx: oneshot::Receiver<FinishMode>,
) -> Result<(), String> {
    let (sample_tx, mut sample_rx) = mpsc::unbounded_channel::<Vec<f32>>();
    let (capture_stop_tx, capture_stop_rx) = oneshot::channel::<()>();

    // Pull device config off the host before the capture thread starts
    // so we know the source sample rate up front for resampling.
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No default audio input device. Plug in a microphone.".to_string())?;
    let config = device
        .default_input_config()
        .map_err(|e| format!("Could not query input device config: {e}"))?;
    // cpal 0.17 declares `SampleRate` as `u32`, so this returns the
    // raw rate already; no tuple-struct unwrap needed.
    let source_rate = config.sample_rate();
    let source_channels = config.channels() as usize;

    // cpal `Stream` is !Send and !Sync — keep it on its own OS thread.
    // The thread parks until `capture_stop_rx` is dropped via the
    // oneshot or until the channel sender disconnects.
    let stream_thread = std::thread::spawn(move || {
        capture_thread(device, config, sample_tx, capture_stop_rx);
    });

    // Accumulate samples from the capture thread until the finish
    // signal arrives or the recording cap fires.
    let mut buffer: Vec<f32> = Vec::new();
    let recording_cap = tokio::time::sleep(Duration::from_secs(MAX_RECORDING_SECS));
    tokio::pin!(recording_cap);
    tokio::pin!(finish_rx);

    let mode = loop {
        tokio::select! {
            biased;
            mode = &mut finish_rx => {
                break mode.unwrap_or(FinishMode::Cancel);
            }
            _ = &mut recording_cap => {
                break FinishMode::Finalize;
            }
            chunk = sample_rx.recv() => {
                match chunk {
                    Some(samples) => {
                        // Downmix to mono if needed, then accumulate.
                        if source_channels > 1 {
                            let mut i = 0;
                            while i + source_channels <= samples.len() {
                                let mut sum = 0.0_f32;
                                for c in 0..source_channels {
                                    sum += samples[i + c];
                                }
                                buffer.push(sum / source_channels as f32);
                                i += source_channels;
                            }
                        } else {
                            buffer.extend_from_slice(&samples);
                        }
                    }
                    None => {
                        // Capture thread exited unexpectedly. Treat as
                        // a finalize so we still try to transcribe
                        // whatever we did buffer.
                        break FinishMode::Finalize;
                    }
                }
            }
        }
    };

    // Stop the capture thread cleanly. Dropping the sender wakes any
    // recv on the other side; sending on the stop oneshot also signals
    // the thread to drop the cpal stream.
    let _ = capture_stop_tx.send(());
    drop(sample_rx);
    let _ = stream_thread.join();

    if matches!(mode, FinishMode::Cancel) {
        let _ = on_event.send(VoiceEvent::Cancelled);
        return Ok(());
    }

    if buffer.is_empty() {
        let _ = on_event.send(VoiceEvent::Final(String::new()));
        return Ok(());
    }

    // Resample mono buffer to 16 kHz f32 PCM for whisper.
    let resampled = resample_linear(&buffer, source_rate, WHISPER_SAMPLE_RATE);

    let _ = on_event.send(VoiceEvent::Transcribing);
    let transcript =
        tokio::task::spawn_blocking(move || run_whisper(&model_path, &resampled))
            .await
            .map_err(|e| format!("transcription task panicked: {e}"))??;

    let _ = on_event.send(VoiceEvent::Final(transcript));
    Ok(())
}

/// cpal capture thread. Builds an input stream that funnels every
/// callback's f32 PCM samples into `sample_tx`. Lives until either the
/// stop oneshot fires or the channel is closed.
fn capture_thread(
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
    sample_tx: mpsc::UnboundedSender<Vec<f32>>,
    stop_rx: oneshot::Receiver<()>,
) {
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();
    let err_tx = sample_tx.clone();
    let err_fn = move |err: cpal::StreamError| {
        // Surface stream errors as an empty sample chunk so the outer
        // task at least knows something went wrong via channel close.
        eprintln!("voice: capture stream error: {err}");
        let _ = err_tx.send(Vec::new());
    };

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _| {
                let _ = sample_tx.send(data.to_vec());
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _| {
                let converted: Vec<f32> = data
                    .iter()
                    .map(|s| (*s as f32) / (i16::MAX as f32))
                    .collect();
                let _ = sample_tx.send(converted);
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _| {
                let converted: Vec<f32> = data
                    .iter()
                    .map(|s| ((*s as f32) - (u16::MAX as f32 / 2.0)) / (u16::MAX as f32 / 2.0))
                    .collect();
                let _ = sample_tx.send(converted);
            },
            err_fn,
            None,
        ),
        other => {
            eprintln!("voice: unsupported sample format {other:?}");
            return;
        }
    };

    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            eprintln!("voice: build_input_stream failed: {e}");
            return;
        }
    };
    if let Err(e) = stream.play() {
        eprintln!("voice: stream play failed: {e}");
        return;
    }

    // Block until the outer task signals stop. We use a blocking recv
    // here because this thread does not own a Tokio runtime.
    let _ = stop_rx.blocking_recv();
    drop(stream);
}

/// Linear-interpolation resampler. Cheap, good enough for whisper —
/// the model itself does the heavy lifting on noisy/aliased input.
/// Returns the input untouched when `src_rate == dst_rate`.
pub fn resample_linear(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if input.is_empty() || src_rate == 0 || dst_rate == 0 {
        return Vec::new();
    }
    if src_rate == dst_rate {
        return input.to_vec();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input[idx];
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Loads the whisper model and runs inference on the given 16 kHz mono
/// f32 PCM buffer. Returns the joined transcript text trimmed.
fn run_whisper(model_path: &std::path::Path, pcm_16khz_mono: &[f32]) -> Result<String, String> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let ctx = WhisperContext::new_with_params(
        model_path
            .to_str()
            .ok_or_else(|| "model path is not valid UTF-8".to_string())?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| format!("could not load whisper model: {e}"))?;

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("could not create whisper state: {e}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    params.set_no_context(true);
    params.set_single_segment(false);

    state
        .full(params, pcm_16khz_mono)
        .map_err(|e| format!("whisper inference failed: {e}"))?;

    // whisper-rs 0.16 returns the raw segment count (no Result), and
    // segments are accessed via `get_segment(i)` returning an Option<
    // WhisperSegment>. The text comes off the segment via to_str_lossy
    // — we never want a panic on a non-UTF-8 byte stream from whisper.
    let num_segments = state.full_n_segments();
    let mut out = String::new();
    for i in 0..num_segments {
        let Some(segment) = state.get_segment(i) else {
            continue;
        };
        let seg_text = segment
            .to_str_lossy()
            .map_err(|e| format!("could not read segment {i}: {e}"))?;
        let trimmed = seg_text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(trimmed);
    }
    Ok(out.trim().to_string())
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_filename_accepts_real_models() {
        assert!(validate_model_filename("ggml-base.en-q5_1.bin").is_ok());
        assert!(validate_model_filename("ggml-tiny.bin").is_ok());
        assert!(validate_model_filename("ggml-large-v3.bin").is_ok());
    }

    #[test]
    fn validate_filename_rejects_traversal() {
        assert!(validate_model_filename("../etc/passwd").is_err());
        assert!(validate_model_filename("ggml-../boot.bin").is_err());
        assert!(validate_model_filename("/abs/ggml-base.bin").is_err());
        assert!(validate_model_filename("dir\\ggml-base.bin").is_err());
    }

    #[test]
    fn validate_filename_rejects_wrong_shape() {
        assert!(validate_model_filename("").is_err());
        assert!(validate_model_filename("model.bin").is_err());
        assert!(validate_model_filename("ggml-base.txt").is_err());
    }

    #[test]
    fn resample_passthrough_when_rates_match() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        let out = resample_linear(&input, 16_000, 16_000);
        assert_eq!(out, input);
    }

    #[test]
    fn resample_downsamples_to_correct_length() {
        let input: Vec<f32> = (0..48_000).map(|i| i as f32 / 48_000.0).collect();
        let out = resample_linear(&input, 48_000, 16_000);
        // 48k → 16k = 3:1 ratio, so 16_000 samples out for 48_000 in.
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn resample_handles_empty_input() {
        assert!(resample_linear(&[], 48_000, 16_000).is_empty());
    }

    #[test]
    fn resample_returns_empty_on_zero_rate() {
        assert!(resample_linear(&[0.5, 0.5], 0, 16_000).is_empty());
        assert!(resample_linear(&[0.5, 0.5], 16_000, 0).is_empty());
    }

    #[test]
    fn voice_state_install_replaces_prior() {
        let s = VoiceState::new();
        let (tx1, _rx1) = oneshot::channel::<FinishMode>();
        let prev = s.install(VoiceSession {
            finish_tx: Some(tx1),
        });
        assert!(prev.is_none());
        let (tx2, _rx2) = oneshot::channel::<FinishMode>();
        let prev = s.install(VoiceSession {
            finish_tx: Some(tx2),
        });
        assert!(prev.is_some());
    }

    #[test]
    fn voice_state_take_finish_consumes_once() {
        let s = VoiceState::new();
        let (tx, _rx) = oneshot::channel::<FinishMode>();
        s.install(VoiceSession {
            finish_tx: Some(tx),
        });
        assert!(s.take_finish().is_some());
        assert!(s.take_finish().is_none());
    }

    #[test]
    fn voice_state_clear_drops_session() {
        let s = VoiceState::new();
        let (tx, _rx) = oneshot::channel::<FinishMode>();
        s.install(VoiceSession {
            finish_tx: Some(tx),
        });
        s.clear();
        assert!(s.take_finish().is_none());
    }

    #[test]
    fn voice_state_download_token_round_trips() {
        let s = VoiceState::new();
        let token = CancellationToken::new();
        s.set_download_cancel(token.clone());
        let taken = s.take_download_cancel();
        assert!(taken.is_some());
        assert!(s.take_download_cancel().is_none());
    }
}
