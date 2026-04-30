//! Compiled default values for the application configuration.
//!
//! This is the ONE place where Wren's default configuration lives. Every
//! other subsystem reads the resolved values from `AppConfig` via Tauri state.
//! Changing a default here propagates to a fresh first-run config file and to
//! any field a user has left unset or left empty in their existing file.

/// Default Ollama HTTP endpoint (loopback, standard port).
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

/// Built-in secretary persona prompt. User overrides via `[prompt] system` in
/// the config file. The slash-command appendix is composed on top at load time
/// and is never written back to the file.
pub const DEFAULT_SYSTEM_PROMPT_BASE: &str = include_str!("../../prompts/system_prompt.txt");

/// Generated appendix listing supported slash commands. Composed on top of
/// the user-editable base prompt at load time so built-in command knowledge
/// stays in sync with the registry even when the persona prompt is overridden.
pub const SLASH_COMMAND_PROMPT_APPENDIX: &str =
    include_str!("../../prompts/generated/slash_commands.txt");

/// Window defaults (logical pixels). Only the user-tunable knobs live here;
/// the collapsed-bar height and the close-animation deadline are baked into
/// `App.tsx` because their effective range is invisible to users (see the
/// rationale comment on `WindowSection` in `schema.rs`).
pub const DEFAULT_OVERLAY_WIDTH: f64 = 600.0;
pub const DEFAULT_MAX_CHAT_HEIGHT: f64 = 648.0;

/// Quote display defaults.
pub const DEFAULT_QUOTE_MAX_DISPLAY_LINES: u32 = 4;
pub const DEFAULT_QUOTE_MAX_DISPLAY_CHARS: u32 = 300;
pub const DEFAULT_QUOTE_MAX_CONTEXT_LENGTH: u32 = 4096;

/// Numeric sanity bounds used by the loader to reject values that would brick
/// the UI. Out-of-bounds values fall back to compiled defaults. The bounds
/// themselves are intentionally generous: the intent is to catch typos
/// (zeros, missing digits), not to second-guess tasteful customization.
pub const BOUNDS_OVERLAY_WIDTH: (f64, f64) = (200.0, 2000.0);
pub const BOUNDS_MAX_CHAT_HEIGHT: (f64, f64) = (200.0, 2000.0);
pub const BOUNDS_QUOTE_MAX_DISPLAY_LINES: (u32, u32) = (1, 100);
pub const BOUNDS_QUOTE_MAX_DISPLAY_CHARS: (u32, u32) = (1, 10_000);
pub const BOUNDS_QUOTE_MAX_CONTEXT_LENGTH: (u32, u32) = (1, 65_536);

/// Search service default URLs. Match the Docker sandbox bindings in
/// `sandbox/docker-compose.yml`. Users running SearXNG or the reader
/// service on a different port override these in `[search]` in config.toml.
pub const DEFAULT_SEARXNG_URL: &str = "http://127.0.0.1:25017";
pub const DEFAULT_READER_URL: &str = "http://127.0.0.1:25018";

/// Default values for user-configurable search pipeline tuning knobs.
/// `max_iterations` caps the search-refine loop count; `top_k_urls` limits
/// how many reranked URLs are forwarded to the reader;
/// `searxng_max_results` caps how many results each SearXNG query
/// contributes before reranking. All are overridable under `[search]` in
/// config.toml.
pub const DEFAULT_MAX_ITERATIONS: u32 = 3;
pub const DEFAULT_TOP_K_URLS: u32 = 10;
pub const DEFAULT_SEARXNG_MAX_RESULTS: u32 = 10;

/// Defense-in-depth caps on data flowing in/out of SearXNG. These are NOT
/// exposed in config.toml: `MAX_QUERY_CHARS` bounds outgoing queries to the
/// external engines (so a malformed prompt cannot DOS them), and
/// `MAX_SNIPPET_CHARS` bounds the per-result text Wren accepts back (so a
/// malicious search result cannot flood the rerank prompt). Both apply
/// before any user-controllable knob, in unicode scalar values.
pub const DEFAULT_MAX_SNIPPET_CHARS: usize = 500;
pub const DEFAULT_MAX_QUERY_CHARS: usize = 500;

// Pipeline-internal defaults: not exposed in config.toml because they are
// part of the prompt and retry contract. Changing these values alters output
// shape and quality, not only latency, so they are intentionally not
// user-tunable at runtime.

/// Gap-filling queries generated per iteration round. Drives the judge
/// normalization cap in `search::judge::normalize_verdict`.
pub const DEFAULT_GAP_QUERIES_PER_ROUND: usize = 3;
/// Approximate token budget for each retrieved page chunk. Drives the
/// chunker split heuristic; downstream prompts assume this exact size.
pub const DEFAULT_CHUNK_TOKEN_SIZE: usize = 500;
/// Number of highest-scoring chunks forwarded to the synthesis prompt.
pub const DEFAULT_TOP_K_CHUNKS: usize = 8;
/// Milliseconds before retrying a failed reader fetch.
pub const DEFAULT_READER_RETRY_DELAY_MS: u64 = 500;

/// Search timeout defaults (seconds).
pub const DEFAULT_SEARCH_TIMEOUT_S: u64 = 20;
pub const DEFAULT_READER_PER_URL_TIMEOUT_S: u64 = 10;
pub const DEFAULT_READER_BATCH_TIMEOUT_S: u64 = 30;
pub const DEFAULT_JUDGE_TIMEOUT_S: u64 = 30;
pub const DEFAULT_ROUTER_TIMEOUT_S: u64 = 45;

/// Bounds for search pipeline counts.
pub const BOUNDS_MAX_ITERATIONS: (u32, u32) = (1, 10);
pub const BOUNDS_TOP_K_URLS: (u32, u32) = (1, 20);
pub const BOUNDS_SEARXNG_MAX_RESULTS: (u32, u32) = (1, 20);

/// Bounds for all search timeout fields (seconds). 300 s (5 min) is the
/// ceiling: a timeout longer than that indicates a misconfiguration, not a
/// slow service.
pub const BOUNDS_TIMEOUT_S: (u64, u64) = (1, 300);

/// Default whisper.cpp model filename for push-to-talk voice input.
/// Empty means "voice is not configured yet" — the Settings → Voice
/// panel surfaces an installer for the standard model lineup, and
/// `voice_record` refuses to run until a real file is present in the
/// `<app_data_dir>/whisper-models/` directory.
pub const DEFAULT_VOICE_MODEL: &str = "";

/// Whether voice input is enabled at all. When false, the Ctrl+Shift+
/// Space hotkey is a no-op even while the overlay is visible. Defaults
/// to off so a fresh install does not start capturing audio without
/// the user opting in.
pub const DEFAULT_VOICE_ENABLED: bool = false;

/// Whether text-to-speech (SAPI) speaks completed assistant responses
/// aloud. Off by default so a fresh install never surprises the user
/// with sound. Honours `tts_voice` and `tts_rate` once enabled.
pub const DEFAULT_TTS_ENABLED: bool = false;

/// SAPI voice name, e.g. "Microsoft David Desktop", "Microsoft Zira
/// Desktop". Empty means "use the system default voice". The Settings
/// → Voice tab populates a dropdown from `tts_list_voices` (live
/// PowerShell query of `System.Speech.Synthesis.SpeechSynthesizer.
/// GetInstalledVoices()`), so the user picks from a known-installed
/// list rather than typing.
pub const DEFAULT_TTS_VOICE: &str = "";

/// SAPI speech rate. Range matches the SAPI `Rate` property:
/// -10 (slowest) through 10 (fastest), 0 = neutral.
pub const DEFAULT_TTS_RATE: i32 = 0;

/// Bounds for `tts_rate`. Values outside the range fall back to the
/// default. Matches SAPI's exposed range exactly — anything out of
/// bounds would be clamped by SAPI anyway, so we reject early to give
/// the user a clear stderr warning instead of silently changing their
/// number.
pub const BOUNDS_TTS_RATE: (i32, i32) = (-10, 10);

/// Default `[mcp].servers_json` payload. Empty string means "no MCP
/// servers configured" — the loader parses this into an empty
/// `Vec<McpServerConfig>` and the tool catalog stays exactly as it was
/// before Phase 4. Users opt in by pasting a JSON array of server
/// definitions through Settings → AI → MCP servers.
pub const DEFAULT_MCP_SERVERS_JSON: &str = "";

/// Hard cap on the JSON blob the user can stash in `[mcp].servers_json`.
/// 64 KiB comfortably fits hundreds of server definitions while bounding
/// loader cost on a corrupt or adversarial config file. Out-of-bound
/// blobs are reset to the default and a stderr warning is emitted —
/// same behaviour as numeric out-of-bounds.
pub const MCP_SERVERS_JSON_MAX_BYTES: usize = 64 * 1024;

/// Maximum per-MCP-server name length. The name becomes part of the
/// tool name we surface to Ollama (`mcp__<server>__<tool>`); 64 bytes
/// is generous for a human-friendly label and bounds prompt size.
pub const MCP_SERVER_NAME_MAX_LEN: usize = 64;

/// Maximum per-MCP-tool name length as exposed by the server. Same
/// rationale as `MCP_SERVER_NAME_MAX_LEN`: bounds the prefixed name
/// surfaced to the model.
pub const MCP_TOOL_NAME_MAX_LEN: usize = 96;

/// Maximum bytes of text returned to the model from a single MCP tool
/// call. Tool results re-enter the model context on every loop turn,
/// so an unbounded server response can blow the context window in a
/// single call. Truncation is signalled with a clear marker so the
/// model knows there is more.
pub const MCP_TOOL_RESULT_MAX_BYTES: usize = 50_000;

/// Per-call timeout for `tools/call` requests over the stdio JSON-RPC
/// channel. Long-running tools (web search, large file reads) are
/// expected to complete well below this; a stuck server gets cut loose
/// rather than wedging the chat loop.
pub const MCP_TOOL_CALL_TIMEOUT_SECS: u64 = 60;

/// Per-call timeout for the lighter `initialize` and `tools/list`
/// JSON-RPC requests. These are query-shaped: a server that cannot
/// answer them quickly is not usable.
pub const MCP_HANDSHAKE_TIMEOUT_SECS: u64 = 15;

// Ollama API baked-in limits: not exposed in config.toml because they bound
// attacker-controlled data (response bodies from the local Ollama daemon) and
// keep the UI responsive when the daemon is hung. Changing either timeout
// value would require re-tuning the UX; changing the byte caps would require
// re-evaluating the memory budget.

/// Per-request timeout (in seconds) for the Ollama `/api/tags` GET. Guards
/// the IPC boundary: if the daemon accepts the TCP connection but never
/// responds, `get_model_picker_state` would otherwise block indefinitely and
/// wedge the UI. 5 seconds is generous for a localhost call.
pub const DEFAULT_OLLAMA_TAGS_REQUEST_TIMEOUT_SECS: u64 = 5;

/// Per-request timeout (in seconds) for the Ollama `/api/show` POST. Same
/// rationale as `DEFAULT_OLLAMA_TAGS_REQUEST_TIMEOUT_SECS`: local-loopback
/// HTTP is normally instant, but capping prevents a wedged daemon from
/// blocking picker rendering.
pub const DEFAULT_OLLAMA_SHOW_REQUEST_TIMEOUT_SECS: u64 = 5;

/// Maximum accepted body size for the Ollama `/api/tags` response. Guards
/// against a misbehaving or compromised localhost Ollama streaming an
/// unbounded response that would exhaust memory. 4 MiB comfortably fits
/// thousands of model entries.
pub const MAX_OLLAMA_TAGS_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Maximum accepted body size for the Ollama `/api/show` response. The full
/// Modelfile and parameters can be sizable, but 4 MiB is comfortably above
/// any real model and bounds attacker-controlled inputs.
pub const MAX_OLLAMA_SHOW_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Maximum accepted byte length for a model slug passed to `set_active_model`.
/// Real Ollama slugs are a handful of characters; 256 is generous while still
/// capping adversarial inputs long before any network or database work.
pub const MAX_MODEL_SLUG_LEN: usize = 256;

/// Authoritative allowlist of `(section, key)` pairs the Settings GUI is
/// permitted to write via the `set_config_field` Tauri command.
///
/// This list is the security boundary between the frontend and the on-disk
/// configuration. The command rejects any `(section, key)` not present here
/// with a typed `UnknownSection` / `UnknownField` error, preventing the GUI
/// from attempting to write fields that do not exist or that are intentionally
/// not user-tunable.
///
/// A compile-time test (`config::tests::allowed_fields_match_schema`) asserts
/// the list size matches the count of tunable fields in `AppConfig` so any
/// future schema addition must extend this list explicitly.
///
/// Order matches `AppConfig` field ordering for review-friendliness.
pub const ALLOWED_FIELDS: &[(&str, &str)] = &[
    // [inference]
    ("inference", "ollama_url"),
    // [prompt]
    ("prompt", "system"),
    // [window]
    ("window", "overlay_width"),
    ("window", "max_chat_height"),
    // [quote]
    ("quote", "max_display_lines"),
    ("quote", "max_display_chars"),
    ("quote", "max_context_length"),
    // [search]
    ("search", "searxng_url"),
    ("search", "reader_url"),
    ("search", "max_iterations"),
    ("search", "top_k_urls"),
    ("search", "searxng_max_results"),
    ("search", "search_timeout_s"),
    ("search", "reader_per_url_timeout_s"),
    ("search", "reader_batch_timeout_s"),
    ("search", "judge_timeout_s"),
    ("search", "router_timeout_s"),
    // [voice]
    ("voice", "enabled"),
    ("voice", "model"),
    ("voice", "tts_enabled"),
    ("voice", "tts_voice"),
    ("voice", "tts_rate"),
    // [mcp]
    ("mcp", "servers_json"),
];

/// Authoritative allowlist of section names accepted by `reset_config`.
/// Mirrors the top-level structure of `AppConfig`.
pub const ALLOWED_SECTIONS: &[&str] = &[
    "inference",
    "prompt",
    "window",
    "quote",
    "search",
    "voice",
    "mcp",
];

/// Special turn-boundary tokens used by the major Ollama-served model families.
/// Ollama normally parses these out of `/api/chat` responses, but some fine-tunes
/// leak them into `message.content` as plain text. If the leaked bytes are persisted
/// into history and replayed to a model from a different family on the next turn,
/// that model treats them as garbage tokens and the conversation visibly degrades.
///
/// Stripped before persisting assistant replies and again at render time so legacy
/// on-disk content stays clean visually without a migration. Exact-string match,
/// case-sensitive: these markers are not natural English, so any false-positive
/// collision would already be a bug elsewhere.
///
/// The TypeScript mirror of this list lives in `src/utils/sanitizeAssistantContent.ts`
/// (`STRIP_PATTERNS`). Keep both in sync when adding new model families.
///
/// Not user-tunable: defense-in-depth bound on external/attacker-controlled data.
/// Exposing it would let a malformed or adversarial model response disable the
/// sanitization layer.
pub const STRIP_PATTERNS: &[&str] = &[
    "<|im_start|>",
    "<|im_end|>",
    "<|begin_of_text|>",
    "<|end_of_text|>",
    "<|start_header_id|>",
    "<|end_header_id|>",
    "<|eot_id|>",
    "[INST]",
    "[/INST]",
    "<start_of_turn>",
    "<end_of_turn>",
    "<|endoftext|>",
    "<|user|>",
    "<|assistant|>",
    "<|system|>",
    "<think>",
    "</think>",
];
