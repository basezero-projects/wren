//! Typed shape of the Wren configuration file.
//!
//! Serde derives the TOML mapping automatically. Each section struct carries
//! `#[serde(default)]` so a partial file (missing whole sections or fields)
//! deserializes cleanly: missing fields inherit the compiled defaults via the
//! manual `Default` impls below.
//!
//! Section structs use manual `Default` impls (NOT `#[derive(Default)]`)
//! because deriving Default would fill fields with zero/empty values
//! (`String::default() == ""`, `u64::default() == 0`), which is the opposite
//! of what the user expects. `AppConfig` itself uses `#[derive(Default)]`
//! because it delegates entirely to each section's own `Default` impl.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::defaults::{
    DEFAULT_JUDGE_TIMEOUT_S, DEFAULT_MAX_CHAT_HEIGHT, DEFAULT_MAX_ITERATIONS,
    DEFAULT_MCP_SERVERS_JSON, DEFAULT_OLLAMA_URL, DEFAULT_OVERLAY_WIDTH,
    DEFAULT_QUOTE_MAX_CONTEXT_LENGTH, DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
    DEFAULT_QUOTE_MAX_DISPLAY_LINES, DEFAULT_READER_BATCH_TIMEOUT_S,
    DEFAULT_READER_PER_URL_TIMEOUT_S, DEFAULT_READER_URL, DEFAULT_ROUTER_TIMEOUT_S,
    DEFAULT_SEARCH_TIMEOUT_S, DEFAULT_SEARXNG_MAX_RESULTS, DEFAULT_SEARXNG_URL, DEFAULT_TOP_K_URLS,
    DEFAULT_TTS_ENABLED, DEFAULT_TTS_RATE, DEFAULT_TTS_VOICE, DEFAULT_VOICE_ENABLED,
    DEFAULT_VOICE_MODEL,
};

/// Static, user-tunable inference daemon configuration.
///
/// The active model selection is NOT stored here. Active-model state is
/// runtime UI state owned by [`crate::models::ActiveModelState`] and
/// persisted in the SQLite `app_config` table under
/// [`crate::models::ACTIVE_MODEL_KEY`]. Storing a model slug in TOML would
/// duplicate ground truth from Ollama's `/api/tags` and create a staleness
/// trap: the file would happily reference a model the user has since
/// removed. This section keeps only the truly static knob, the Ollama
/// endpoint URL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct InferenceSection {
    /// HTTP base URL of the local Ollama instance.
    pub ollama_url: String,
    /// Ollama slug used for the destructive-/tool-calling route. When
    /// empty (or unset), Wren falls through to the active chat model
    /// — the user's model handles tool calls itself if it can. If the
    /// active chat model cannot tool-call, Wren uses a built-in
    /// fallback (`qwen3:8b`) so first-run still works without any
    /// configuration. Setting this to the same slug as the active
    /// chat model puts Wren in single-model mode: one model handles
    /// chat, tool calls, and (capability permitting) everything else.
    pub tool_model: String,
}

impl Default for InferenceSection {
    fn default() -> Self {
        Self {
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
            tool_model: String::new(),
        }
    }
}

/// Prompt configuration. `system` holds only the user-editable base text.
/// The slash-command appendix is composed at load time into `resolved_system`
/// and is never written back to the file. `resolved_system` is computed, not
/// serialized.
///
/// Note: `#[derive(Default)]` is correct here because both fields genuinely
/// start empty: `system` empty means "use the built-in persona", and
/// `resolved_system` is populated by the loader before any consumer reads it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct PromptSection {
    /// User-editable persona prompt. Empty means "use the built-in default".
    pub system: String,
    /// Composed runtime value (base prompt plus slash-command appendix).
    /// Not serialized; computed by the loader.
    #[serde(skip)]
    pub resolved_system: String,
}

/// Overlay window geometry. Only the user-tunable knobs live here; the
/// collapsed-bar height and the close-animation deadline are baked into the
/// frontend (see `App.tsx`) because their effective range is invisible to
/// the user (collapsed height is overwritten by the ResizeObserver within a
/// frame; the hide delay sits below normal perception across its usable
/// range and creates a visible pop if dropped below the exit-animation
/// duration).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowSection {
    /// Logical width of the overlay window.
    pub overlay_width: f64,
    /// Maximum height the expanded chat window is allowed to grow to.
    pub max_chat_height: f64,
}

impl Default for WindowSection {
    fn default() -> Self {
        Self {
            overlay_width: DEFAULT_OVERLAY_WIDTH,
            max_chat_height: DEFAULT_MAX_CHAT_HEIGHT,
        }
    }
}

/// Selected-text quote display configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct QuoteSection {
    pub max_display_lines: u32,
    pub max_display_chars: u32,
    pub max_context_length: u32,
}

impl Default for QuoteSection {
    fn default() -> Self {
        Self {
            max_display_lines: DEFAULT_QUOTE_MAX_DISPLAY_LINES,
            max_display_chars: DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
            max_context_length: DEFAULT_QUOTE_MAX_CONTEXT_LENGTH,
        }
    }
}

/// Search pipeline and service configuration.
///
/// Service URLs control where the SearXNG and reader sidecar processes live.
/// The defaults match the Docker sandbox bindings in `sandbox/docker-compose.yml`.
/// Users who remap ports or run the services on a different host set these in
/// `[search]` in config.toml; no rebuild required.
///
/// Pipeline tuning knobs (`max_iterations`, `top_k_urls`) let users trade
/// search quality against latency. Timeout fields cover slow networks and slow
/// local hardware. Values that would create an inconsistency (e.g.
/// `reader_batch_timeout_s <= reader_per_url_timeout_s`) are silently corrected
/// by the loader.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SearchSection {
    /// Base URL of the SearXNG instance (scheme + host + port, no path).
    /// The `/search` endpoint is appended automatically.
    pub searxng_url: String,
    /// Base URL of the reader/extractor sidecar (scheme + host + port, no path).
    pub reader_url: String,
    /// Maximum number of search-refine iterations before the pipeline gives up.
    pub max_iterations: u32,
    /// Number of top-ranked URLs forwarded to the reader after reranking.
    pub top_k_urls: u32,
    /// Maximum number of results each SearXNG query contributes to the
    /// reranker. Acts before rerank to bound prompt size and latency: lower
    /// values trade recall for speed; higher values give the reranker more
    /// candidates per query.
    pub searxng_max_results: u32,
    /// Seconds before a SearXNG query is abandoned.
    pub search_timeout_s: u64,
    /// Seconds allowed for a single URL fetch inside the reader.
    pub reader_per_url_timeout_s: u64,
    /// Seconds allowed for the full parallel reader batch to complete.
    /// Must exceed `reader_per_url_timeout_s`; the loader corrects violations.
    pub reader_batch_timeout_s: u64,
    /// Seconds before the judge LLM call is abandoned.
    pub judge_timeout_s: u64,
    /// Seconds before the router LLM call is abandoned.
    pub router_timeout_s: u64,
}

impl Default for SearchSection {
    fn default() -> Self {
        Self {
            searxng_url: DEFAULT_SEARXNG_URL.to_string(),
            reader_url: DEFAULT_READER_URL.to_string(),
            max_iterations: DEFAULT_MAX_ITERATIONS,
            top_k_urls: DEFAULT_TOP_K_URLS,
            searxng_max_results: DEFAULT_SEARXNG_MAX_RESULTS,
            search_timeout_s: DEFAULT_SEARCH_TIMEOUT_S,
            reader_per_url_timeout_s: DEFAULT_READER_PER_URL_TIMEOUT_S,
            reader_batch_timeout_s: DEFAULT_READER_BATCH_TIMEOUT_S,
            judge_timeout_s: DEFAULT_JUDGE_TIMEOUT_S,
            router_timeout_s: DEFAULT_ROUTER_TIMEOUT_S,
        }
    }
}

/// Voice configuration — both push-to-talk input and text-to-speech
/// output share this section because they live on the same Settings
/// → Voice tab.
///
/// `enabled` gates the Ctrl+Shift+Space hotkey: when false, the
/// hotkey is a no-op even while the overlay is visible. Defaults to
/// off so a fresh install does not start capturing audio without the
/// user opting in.
///
/// `model` is the filename of a whisper.cpp ggml model under
/// `<app_data_dir>/whisper-models/`. Empty means "voice is not yet
/// configured"; the Settings → Voice panel installs the standard
/// model lineup and `voice_record` refuses to run until a real file
/// is present.
///
/// `tts_enabled` controls whether completed assistant responses are
/// spoken aloud via SAPI. Off by default so a fresh install never
/// surprises the user with sound. `tts_voice` is a SAPI voice name
/// (empty = system default); `tts_rate` is SAPI's `Rate` property
/// in -10..=10.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceSection {
    pub enabled: bool,
    pub model: String,
    pub tts_enabled: bool,
    pub tts_voice: String,
    pub tts_rate: i32,
}

impl Default for VoiceSection {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_VOICE_ENABLED,
            model: DEFAULT_VOICE_MODEL.to_string(),
            tts_enabled: DEFAULT_TTS_ENABLED,
            tts_voice: DEFAULT_TTS_VOICE.to_string(),
            tts_rate: DEFAULT_TTS_RATE,
        }
    }
}

/// MCP (Model Context Protocol) client configuration.
///
/// Wren acts as an MCP **client**: it spawns each server as a child process,
/// speaks JSON-RPC 2.0 over stdio, and exposes the server's tool catalog to
/// the local Ollama model alongside Wren's built-in tools. Every server tool
/// shows up to the model as `mcp__<server_name>__<tool_name>` and dispatches
/// through the same approval card the destructive built-in tools use.
///
/// The on-disk shape is a single TOML string (`servers_json`) that holds a
/// JSON-encoded array of server definitions. Two reasons it lives as a
/// JSON-string instead of a native `[[mcp.servers]]` table list:
///
/// 1. The `set_config_field` security boundary is per-`(section, key)`, and
///    threading dynamic-list mutation through that surface would mean adding
///    a parallel write path. A single string field reuses the existing one.
/// 2. The Settings UI already deals in JSON for editable structured fields;
///    a textarea + parse-on-save matches the prompt-system editor pattern.
///
/// `servers` holds the parsed view computed by the loader. Bad JSON, an
/// over-cap blob, or per-server validation failures degrade silently to an
/// empty list (with a stderr warning) so a typo in the JSON never bricks
/// chat — the model just sees no MCP tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct McpSection {
    /// User-editable JSON array of `McpServerConfig` records. Empty (the
    /// default) means "no MCP servers configured": no child processes are
    /// spawned and no `mcp__*` tools appear in the catalog.
    pub servers_json: String,
    /// Parsed `servers_json`. Computed at load time and never serialized
    /// back to disk so the on-disk file remains the single source of truth.
    /// Empty when `servers_json` is empty, malformed, over the byte cap,
    /// or when every server in the array fails per-server validation.
    #[serde(skip)]
    pub servers: Vec<McpServerConfig>,
}

impl Default for McpSection {
    fn default() -> Self {
        Self {
            servers_json: DEFAULT_MCP_SERVERS_JSON.to_string(),
            servers: Vec::new(),
        }
    }
}

/// Single MCP server definition as parsed out of `[mcp].servers_json`.
///
/// `name` becomes part of the tool name surfaced to Ollama and must be
/// safe to embed in `mcp__<name>__<tool>`: alphanumeric plus `-_`, no
/// whitespace, no separators that would collide with our delimiter.
/// `command` is the absolute path or PATH-resolvable executable launched
/// as the server child. `args` and `env` follow `tokio::process::Command`
/// semantics directly. `BTreeMap` for `env` so the serialized JSON has a
/// deterministic key order — easier to diff and review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// Top-level application configuration. Managed Tauri state; every subsystem
/// reads from `State<RwLock<AppConfig>>` and nowhere else. The loader resolves all
/// empty strings and out-of-bounds numerics to compiled defaults before the
/// `AppConfig` is installed, so every field here holds a usable value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct AppConfig {
    pub inference: InferenceSection,
    pub prompt: PromptSection,
    pub window: WindowSection,
    pub quote: QuoteSection,
    pub search: SearchSection,
    pub voice: VoiceSection,
    pub mcp: McpSection,
}
