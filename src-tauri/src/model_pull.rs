/*!
 * Streaming model-pull command.
 *
 * Wraps Ollama's `POST /api/pull` so the frontend can install new
 * models from inside Wren — no terminal trip to `ollama pull`. The
 * endpoint returns newline-delimited JSON progress events; we
 * forward each one to the frontend as a typed `PullEvent` over a
 * Tauri Channel.
 *
 * Progress shape from Ollama looks roughly like:
 *
 *   {"status":"pulling manifest"}
 *   {"status":"downloading","digest":"sha256:abc","total":1234567,"completed":1234}
 *   {"status":"downloading","digest":"sha256:abc","total":1234567,"completed":12345}
 *   {"status":"verifying sha256 digest"}
 *   {"status":"writing manifest"}
 *   {"status":"success"}
 *
 * The first chunk after the user clicks Pull is "pulling manifest"
 * which is fast; the long pause is the layer download. Per-digest
 * total/completed bytes drive the progress bar; aggregating across
 * digests is left to the frontend so the UI stays in one place.
 */

use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;

/// Top-level cap on a single pull. A 70B-class model on a slow
/// connection can take an hour, so this is generous; what we are
/// really protecting against is a connection that hangs forever.
const PULL_TOTAL_TIMEOUT: Duration = Duration::from_secs(60 * 60 * 4);

/// Per-chunk no-progress timeout. If Ollama goes silent for longer
/// than this we surface an error; otherwise the user is staring at
/// a frozen progress bar with no idea why.
const PULL_CHUNK_TIMEOUT: Duration = Duration::from_secs(60);

/// Event types emitted to the frontend during a pull.
#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum PullEvent {
    /// Free-form status string from Ollama. Examples: "pulling manifest",
    /// "verifying sha256 digest", "writing manifest", "success".
    Status(String),
    /// Byte-level progress for a single layer (digest). Multiple
    /// digests interleave during a multi-layer pull; the frontend
    /// keeps the latest values per-digest and displays the sum.
    Progress {
        digest: String,
        total: u64,
        completed: u64,
    },
    /// Pull completed successfully.
    Done,
    /// User cancelled the pull (closed the dialog or hit cancel).
    Cancelled,
    /// Pull failed. Carries a user-facing message.
    Error(String),
}

/// Raw shape of an Ollama `/api/pull` line.
#[derive(Deserialize)]
struct OllamaPullLine {
    status: Option<String>,
    digest: Option<String>,
    total: Option<u64>,
    completed: Option<u64>,
    error: Option<String>,
}

/// Streams a `POST /api/pull` to Ollama and forwards each progress
/// line to the frontend Channel as a typed `PullEvent`.
///
/// Cancellation: the caller stores its own `CancellationToken` in
/// `crate::commands::GenerationState` so the existing
/// `cancel_generation` command can interrupt an in-flight pull.
/// (We piggyback on the chat cancellation token for now — only one
/// long-running operation runs at a time per Wren window.)
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn pull_model(
    name: String,
    on_event: Channel<PullEvent>,
    client: State<'_, reqwest::Client>,
    generation: State<'_, crate::commands::GenerationState>,
    config: State<'_, parking_lot::RwLock<AppConfig>>,
) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        let _ = on_event.send(PullEvent::Error(
            "Model name is empty. Try something like qwen3:8b.".to_string(),
        ));
        return Ok(());
    }

    let endpoint = {
        let cfg = config.read();
        format!(
            "{}/api/pull",
            cfg.inference.ollama_url.trim_end_matches('/')
        )
    };

    let cancel = CancellationToken::new();
    generation.set_token(cancel.clone());

    let payload = serde_json::json!({
        "name": trimmed,
        "stream": true,
    });

    let res = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let _ = on_event.send(PullEvent::Cancelled);
            generation.clear_token();
            return Ok(());
        }
        r = client
            .post(&endpoint)
            .json(&payload)
            .timeout(PULL_TOTAL_TIMEOUT)
            .send() => r,
    };

    let response = match res {
        Ok(r) => r,
        Err(e) => {
            let msg = if e.is_connect() {
                "Could not reach Ollama. Is it running?".to_string()
            } else if e.is_timeout() {
                "The pull timed out.".to_string()
            } else {
                format!("Network error: {e}")
            };
            let _ = on_event.send(PullEvent::Error(msg));
            generation.clear_token();
            return Ok(());
        }
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("HTTP {status}"));
        let _ = on_event.send(PullEvent::Error(format!(
            "Ollama refused the pull: {detail}",
        )));
        generation.clear_token();
        return Ok(());
    }

    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    let mut saw_success = false;

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                drop(stream);
                let _ = on_event.send(PullEvent::Cancelled);
                generation.clear_token();
                return Ok(());
            }
            timed = tokio::time::timeout(PULL_CHUNK_TIMEOUT, stream.next()) => {
                let chunk_opt = match timed {
                    Ok(opt) => opt,
                    Err(_) => {
                        drop(stream);
                        let _ = on_event.send(PullEvent::Error(format!(
                            "Pull stalled: no progress for {}s. Network or Ollama may be down.",
                            PULL_CHUNK_TIMEOUT.as_secs()
                        )));
                        generation.clear_token();
                        return Ok(());
                    }
                };
                match chunk_opt {
                    Some(Ok(bytes)) => {
                        buffer.extend_from_slice(&bytes);
                        while let Some(idx) = buffer.iter().position(|&b| b == b'\n') {
                            let line_bytes = buffer.drain(..=idx).collect::<Vec<u8>>();
                            let Ok(text) = String::from_utf8(line_bytes) else {
                                continue;
                            };
                            let trimmed = text.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            let Ok(line) = serde_json::from_str::<OllamaPullLine>(trimmed) else {
                                continue;
                            };

                            // Errors come through as `{error: "..."}` rather
                            // than a non-2xx status.
                            if let Some(err) = line.error {
                                let _ = on_event.send(PullEvent::Error(err));
                                generation.clear_token();
                                return Ok(());
                            }

                            // Per-digest byte progress.
                            if let (Some(digest), Some(total), Some(completed)) =
                                (line.digest.as_deref(), line.total, line.completed)
                            {
                                let _ = on_event.send(PullEvent::Progress {
                                    digest: digest.to_string(),
                                    total,
                                    completed,
                                });
                            }

                            // Status lines (always present). "success"
                            // marks the natural end.
                            if let Some(status) = line.status {
                                if status == "success" {
                                    saw_success = true;
                                }
                                let _ = on_event.send(PullEvent::Status(status));
                            }
                        }
                    }
                    Some(Err(e)) => {
                        let _ = on_event.send(PullEvent::Error(format!(
                            "Network error during pull: {e}"
                        )));
                        generation.clear_token();
                        return Ok(());
                    }
                    None => {
                        // Stream closed cleanly.
                        if saw_success {
                            let _ = on_event.send(PullEvent::Done);
                        } else {
                            let _ = on_event.send(PullEvent::Error(
                                "Pull ended unexpectedly without a success marker.".to_string(),
                            ));
                        }
                        generation.clear_token();
                        return Ok(());
                    }
                }
            }
        }
    }
}
