/*!
 * Text-to-speech via Windows SAPI.
 *
 * v0 is auto-speak on assistant `Done`. The frontend collects the
 * full response, calls `tts_speak`, and Wren spawns a one-shot
 * PowerShell child that drives `System.Speech.Synthesis.
 * SpeechSynthesizer`. Each new utterance kills the previous speaker
 * so a long response cannot pile up while the user keeps talking;
 * `tts_stop` does the same on demand. `tts_list_voices` queries
 * `GetInstalledVoices()` once for the Settings dropdown.
 *
 * Why a per-utterance PowerShell spawn (and not native SAPI COM, or a
 * long-lived stdin-piped PS):
 * - Zero new Cargo deps.
 * - SAPI 5 ships in-box on every supported Windows version.
 * - PS startup is ~300 ms — fine for the v0 chat-TTS UX, well below
 *   the latency of the Ollama generation that produced the text.
 * - When we want lower latency, swapping in a long-lived stdin-piped
 *   PS process or native COM via the `windows` crate is a contained
 *   change behind these three Tauri commands.
 *
 * Defense-in-depth on the PS payload:
 * - Speech text is written to a temp file and passed to PS by path.
 *   PS reads the file with `Get-Content -Raw` and never sees the text
 *   on its own command line. This means a response containing
 *   backticks, `$()`, single quotes, or pipe characters cannot inject
 *   PowerShell commands.
 * - Voice name is single-quoted with internal apostrophes doubled
 *   (PowerShell's literal-string escape) and additionally validated
 *   through a strict allowlist. SAPI voice names are short ASCII
 *   strings like "Microsoft David Desktop"; rejecting anything outside
 *   that shape blocks injection even if the escape were ever bypassed.
 * - Rate is clamped to SAPI's documented range (-10..=10) before it
 *   reaches PS.
 *
 * On non-Windows platforms every command returns a clean error so the
 * frontend can surface a "TTS is Windows-only" notice.
 */

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

/// Hard cap on the text we hand to SAPI. SAPI itself accepts much
/// more, but clamping protects against a runaway model spitting an
/// effectively unbounded response into the speaker.
const TTS_MAX_TEXT_BYTES: usize = 16_000;

/// Hard cap on a single `tts_list_voices` PowerShell query. The PS
/// startup dominates this; the actual SAPI call is instant.
const TTS_LIST_VOICES_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum SAPI voice name length we accept. The `Microsoft <Name>
/// Desktop` shape is well under this, even with non-ASCII characters
/// in localised builds. Anything longer is rejected.
const TTS_VOICE_NAME_MAX_LEN: usize = 128;

/// State for the active speaker process. Held as Tauri-managed state
/// so cancellation (a new utterance, an explicit stop, or app exit)
/// can reach the running child.
#[derive(Default)]
pub struct TtsState {
    inner: Arc<Mutex<Option<tokio::process::Child>>>,
}

impl TtsState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the active child with `child`, returning the previous
    /// one (if any) so the caller can kill it. Splitting the
    /// "install new" and "kill old" steps means we hold the lock for
    /// as little time as possible.
    async fn swap_active(
        &self,
        child: Option<tokio::process::Child>,
    ) -> Option<tokio::process::Child> {
        let mut guard = self.inner.lock().await;
        std::mem::replace(&mut *guard, child)
    }

    /// Drops the active child if any. Returns true when something was
    /// actually running; false if there was nothing to stop.
    async fn kill_active(&self) -> bool {
        let prev = self.swap_active(None).await;
        if let Some(mut child) = prev {
            // `kill_on_drop(true)` covers the abrupt-exit path; explicit
            // start_kill here is best-effort cleanup so we do not leave
            // a speaker droning over the next utterance.
            let _ = child.start_kill();
            return true;
        }
        false
    }
}

/// One installed SAPI voice. Returned to the frontend so the Settings
/// → Voice dropdown can render a known-installed list (matches the
/// pattern used by `list_whisper_models`).
#[derive(Clone, Serialize)]
pub struct InstalledVoice {
    /// Voice name as SAPI reports it, e.g. "Microsoft David Desktop".
    /// This is also what we round-trip through `[voice].tts_voice`.
    pub name: String,
    /// Display culture, e.g. "en-US". Empty string when SAPI does not
    /// expose one (rare, but defensive — the dropdown still renders).
    pub culture: String,
}

/// Validates that `name` looks like a plausible SAPI voice name.
/// Defense-in-depth on top of the file-based payload pattern: the
/// name DOES end up on the PS command line because SAPI's
/// `SelectVoice` takes a string parameter. We reject anything outside
/// a tight allowlist so an injection cannot ride along even if the
/// quoting strategy were ever bypassed.
pub fn validate_voice_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        // Empty is the "use system default" sentinel. Validate anyway
        // so callers that pass a non-empty name go through the same
        // gate.
        return Ok(());
    }
    if name.len() > TTS_VOICE_NAME_MAX_LEN {
        return Err(format!(
            "voice name is too long ({} > {} bytes)",
            name.len(),
            TTS_VOICE_NAME_MAX_LEN
        ));
    }
    for ch in name.chars() {
        // Letters, digits, and a small set of punctuation that real
        // SAPI voice names use. Rejects backticks, `$`, parentheses,
        // pipes, ampersands, semicolons, quotes — every PowerShell
        // metacharacter.
        let allowed = ch.is_alphanumeric()
            || ch == ' '
            || ch == '-'
            || ch == '_'
            || ch == '.';
        if !allowed {
            return Err(format!(
                "voice name contains illegal character {:?}",
                ch
            ));
        }
    }
    Ok(())
}

/// Clamps `rate` to SAPI's documented `-10..=10` range. Returned as a
/// fresh value so callers do not need a mutable. Out-of-range maps to
/// 0 (neutral) rather than the nearest bound, mirroring how the
/// config loader already treats out-of-bounds rates.
pub fn clamp_rate(rate: i32) -> i32 {
    if (-10..=10).contains(&rate) {
        rate
    } else {
        0
    }
}

/// Builds the PowerShell script that SpeechSynthesizes a file. The
/// script is the second arg of `powershell.exe -Command`, but the
/// utterance text itself is read from `text_path` so it never lands
/// on the command line.
///
/// The voice name is single-quoted; PowerShell single-quoted strings
/// are literal (no expansion of `$`, backticks, or escapes), with
/// internal `'` written as `''`. Combined with `validate_voice_name`
/// above, an injection would have to pass both layers.
pub fn build_speak_script(text_path: &str, voice: &str, rate: i32) -> String {
    let rate = clamp_rate(rate);
    let select_voice = if voice.is_empty() {
        String::new()
    } else {
        let escaped = voice.replace('\'', "''");
        format!("$s.SelectVoice('{escaped}'); ")
    };
    format!(
        "Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $s.Rate = {rate}; \
         {select_voice}\
         $t = Get-Content -Raw -Encoding UTF8 -LiteralPath '{path}'; \
         $s.Speak($t); \
         $s.Dispose()",
        path = text_path.replace('\'', "''"),
    )
}

/// Builds the PowerShell script that prints installed SAPI voices as
/// pipe-delimited `name|culture` lines on stdout. The frontend parses
/// this back into `InstalledVoice` records.
fn build_list_voices_script() -> String {
    "Add-Type -AssemblyName System.Speech; \
     $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
     $s.GetInstalledVoices() | ForEach-Object { \
         $i = $_.VoiceInfo; \
         Write-Output (\"{0}|{1}\" -f $i.Name, $i.Culture) \
     }; \
     $s.Dispose()"
        .to_string()
}

/// Parses `list_voices` PowerShell output into `InstalledVoice`
/// records. Tolerates BOMs, blank lines, and missing culture fields
/// so a quirky locale never empties the dropdown.
pub fn parse_list_voices_output(stdout: &str) -> Vec<InstalledVoice> {
    let mut out = Vec::new();
    for raw_line in stdout.lines() {
        let line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let name = parts.next().unwrap_or("").trim().to_string();
        let culture = parts.next().unwrap_or("").trim().to_string();
        if name.is_empty() {
            continue;
        }
        out.push(InstalledVoice { name, culture });
    }
    out
}

#[cfg(windows)]
fn powershell_command() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
    ]);
    cmd.kill_on_drop(true);
    cmd
}

/// Tauri command: speak `text` aloud. Replaces any in-flight
/// utterance. Honours the active `[voice].tts_voice` and `tts_rate`.
///
/// Returns `Ok(())` even if the underlying PS spawn fails — the
/// frontend shows a small toast based on the returned message rather
/// than treating TTS as a hard error. The chat path must keep
/// flowing.
#[cfg(windows)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn tts_speak(
    text: String,
    app: AppHandle,
    state: State<'_, TtsState>,
    config: State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
) -> Result<(), String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let bounded: String = trimmed.chars().take(TTS_MAX_TEXT_BYTES).collect();

    let (voice, rate) = {
        let cfg = config.read();
        (cfg.voice.tts_voice.clone(), cfg.voice.tts_rate)
    };
    validate_voice_name(&voice)?;

    // Cancel any in-flight utterance before starting a new one.
    state.kill_active().await;

    // Stage the payload as a UTF-8 temp file so PS reads it instead of
    // taking it on the command line.
    let dir = tts_temp_dir(&app)?;
    let payload = dir.join(format!("speak-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&payload, bounded.as_bytes())
        .map_err(|e| format!("could not stage speech payload: {e}"))?;

    let script = build_speak_script(&payload.to_string_lossy(), &voice, rate);
    let mut cmd = powershell_command();
    cmd.arg(&script);
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not spawn powershell.exe: {e}"))?;

    let _previous = state.swap_active(Some(child)).await;
    Ok(())
}

#[cfg(not(windows))]
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn tts_speak(
    _text: String,
    _app: AppHandle,
    _state: State<'_, TtsState>,
    _config: State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
) -> Result<(), String> {
    Err("text-to-speech is only supported on Windows".to_string())
}

/// Tauri command: stop the in-flight utterance, if any. Idempotent —
/// returns false when nothing was speaking.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn tts_stop(state: State<'_, TtsState>) -> Result<bool, String> {
    Ok(state.kill_active().await)
}

/// Tauri command: list installed SAPI voices. Cached on the frontend;
/// the Settings tab refresh button re-invokes.
#[cfg(windows)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn tts_list_voices() -> Result<Vec<InstalledVoice>, String> {
    let mut cmd = powershell_command();
    cmd.arg(build_list_voices_script());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not spawn powershell.exe: {e}"))?;

    let output = match tokio::time::timeout(TTS_LIST_VOICES_TIMEOUT, child.wait_with_output())
        .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("could not read powershell output: {e}")),
        Err(_) => return Err("listing SAPI voices timed out".to_string()),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("powershell failed: {stderr}"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_list_voices_output(&stdout))
}

#[cfg(not(windows))]
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn tts_list_voices() -> Result<Vec<InstalledVoice>, String> {
    Err("text-to-speech is only supported on Windows".to_string())
}

/// Resolves the on-disk directory used to stage speech-payload temp
/// files. Lives under `<app_data_dir>/tts/`, created on first use.
fn tts_temp_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not resolve app data dir: {e}"))?;
    let dir = base.join("tts");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create tts staging dir: {e}"))?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_voice_name ─────────────────────────────────────────────

    #[test]
    fn voice_name_empty_is_allowed() {
        // Empty is the "use system default" sentinel.
        assert!(validate_voice_name("").is_ok());
    }

    #[test]
    fn voice_name_realistic_names_are_allowed() {
        assert!(validate_voice_name("Microsoft David Desktop").is_ok());
        assert!(validate_voice_name("Microsoft Zira Desktop").is_ok());
        assert!(validate_voice_name("Microsoft Hazel Desktop").is_ok());
        assert!(validate_voice_name("en-US-AriaNeural").is_ok());
        assert!(validate_voice_name("Voice_2").is_ok());
        assert!(validate_voice_name("Vocalizer 5.0").is_ok());
    }

    #[test]
    fn voice_name_rejects_powershell_metacharacters() {
        for bad in [
            "Microsoft;Drop",
            "$(Get-Process)",
            "`whoami`",
            "Voice|cmd",
            "Voice&calc",
            "Voice'\"x",
            "Voice<x>",
            "Voice/x",
            "Voice\\x",
            "Voice(x)",
            "Voice\nNew",
        ] {
            assert!(
                validate_voice_name(bad).is_err(),
                "expected reject for {bad:?}"
            );
        }
    }

    #[test]
    fn voice_name_rejects_oversized_input() {
        let big = "A".repeat(TTS_VOICE_NAME_MAX_LEN + 1);
        assert!(validate_voice_name(&big).is_err());
        let exact = "A".repeat(TTS_VOICE_NAME_MAX_LEN);
        assert!(validate_voice_name(&exact).is_ok());
    }

    // ── clamp_rate ──────────────────────────────────────────────────────

    #[test]
    fn clamp_rate_passes_in_range() {
        for r in -10..=10 {
            assert_eq!(clamp_rate(r), r);
        }
    }

    #[test]
    fn clamp_rate_resets_out_of_range() {
        assert_eq!(clamp_rate(-11), 0);
        assert_eq!(clamp_rate(11), 0);
        assert_eq!(clamp_rate(i32::MIN), 0);
        assert_eq!(clamp_rate(i32::MAX), 0);
    }

    // ── build_speak_script ──────────────────────────────────────────────

    #[test]
    fn build_speak_script_no_voice_omits_select_call() {
        let s = build_speak_script("C:/tmp/x.txt", "", 0);
        assert!(s.contains("Add-Type -AssemblyName System.Speech"));
        assert!(s.contains("$s.Rate = 0"));
        assert!(!s.contains("SelectVoice"));
        assert!(s.contains("'C:/tmp/x.txt'"));
    }

    #[test]
    fn build_speak_script_includes_voice_when_set() {
        let s = build_speak_script(
            "C:/tmp/x.txt",
            "Microsoft David Desktop",
            -3,
        );
        assert!(s.contains("$s.SelectVoice('Microsoft David Desktop')"));
        assert!(s.contains("$s.Rate = -3"));
    }

    #[test]
    fn build_speak_script_clamps_rate() {
        let s = build_speak_script("C:/tmp/x.txt", "", 999);
        assert!(s.contains("$s.Rate = 0"));
        let s = build_speak_script("C:/tmp/x.txt", "", -50);
        assert!(s.contains("$s.Rate = 0"));
    }

    #[test]
    fn build_speak_script_doubles_apostrophes_in_paths() {
        let s = build_speak_script("C:/tmp/it's.txt", "", 0);
        // PS literal-string escape doubles the inner apostrophe.
        assert!(s.contains("'C:/tmp/it''s.txt'"));
    }

    #[test]
    fn build_speak_script_doubles_apostrophes_in_voice() {
        // Only ASCII apostrophes are excluded by validate_voice_name (it
        // already rejects them), but the script builder must still
        // escape defensively in case a future path through skips
        // validation.
        let s = build_speak_script("C:/tmp/x.txt", "It's a Voice", 0);
        assert!(s.contains("$s.SelectVoice('It''s a Voice')"));
    }

    // ── parse_list_voices_output ────────────────────────────────────────

    #[test]
    fn parse_voices_handles_normal_output() {
        let stdout = "Microsoft David Desktop|en-US\nMicrosoft Zira Desktop|en-US\n";
        let parsed = parse_list_voices_output(stdout);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "Microsoft David Desktop");
        assert_eq!(parsed[0].culture, "en-US");
        assert_eq!(parsed[1].name, "Microsoft Zira Desktop");
    }

    #[test]
    fn parse_voices_skips_blank_lines_and_bom() {
        let stdout = "\u{feff}Microsoft David Desktop|en-US\n\n  \nMicrosoft Hazel|en-GB\n";
        let parsed = parse_list_voices_output(stdout);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "Microsoft David Desktop");
        assert_eq!(parsed[1].name, "Microsoft Hazel");
        assert_eq!(parsed[1].culture, "en-GB");
    }

    #[test]
    fn parse_voices_tolerates_missing_culture() {
        let stdout = "Plain Voice\nAnother|de-DE\n";
        let parsed = parse_list_voices_output(stdout);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "Plain Voice");
        assert_eq!(parsed[0].culture, "");
        assert_eq!(parsed[1].culture, "de-DE");
    }

    #[test]
    fn parse_voices_drops_lines_with_empty_name() {
        let stdout = "|en-US\n  | en-US\nReal Voice|en-US\n";
        let parsed = parse_list_voices_output(stdout);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Real Voice");
    }

    #[test]
    fn parse_voices_empty_input_yields_empty_vec() {
        assert!(parse_list_voices_output("").is_empty());
        assert!(parse_list_voices_output("\n  \n").is_empty());
    }

    // ── TtsState ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn tts_state_kill_active_with_no_child_returns_false() {
        let state = TtsState::new();
        assert!(!state.kill_active().await);
    }

    #[tokio::test]
    async fn tts_state_swap_active_returns_previous() {
        let state = TtsState::new();
        let first = state.swap_active(None).await;
        assert!(first.is_none());
    }
}
