# Changelog

Wren's release notes. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and [SemVer](https://semver.org/spec/v2.0.0.html).

Wren is a Windows port of [`quiet-node/thuki`](https://github.com/quiet-node/thuki) (Apache-2.0). Upstream history is not reproduced here, see that repo for the pre-fork lineage. Wren's own log starts at `0.1.0`.

## [0.7.0] — 2026-04-29

### Added

- **Web fetch tool (`fetch_url`).** Wren can now read web pages on the user's behalf when chatting. The model gets a new tool, `fetch_url`, that takes an `url` (required, http/https) and an optional `format` ("markdown" | "text", default markdown). On dispatch the existing destructive-tool approval card surfaces in chat with the URL and an Allow / Deny pair — no byte hits the network until the user clicks Allow. After approval the tool fetches the page, runs Mozilla Readability over the HTML via the [`dom_smoothie`](https://crates.io/crates/dom_smoothie) crate, and returns the article body to the model formatted as:

  ```
  # <Article title>
  Source: <final URL after redirects>

  <markdown body>
  ```

  This is the same pattern as `read_file`'s output: title + provenance line + body, with a `[truncated: showing first N of M bytes]` footer when the extracted text exceeds 50,000 bytes. The model uses it the same way — fetch → quote relevant pieces back into the conversation.

  **Defense-in-depth on top of user approval:**
  - Scheme allowlist — only `http` and `https` (no `file://`, `ftp://`, `javascript:`, `data:`).
  - Host blocklist — rejects `localhost`, any `*.localhost`, `*.local`, `*.internal`, all RFC1918 ranges (`10/8`, `172.16/12`, `192.168/16`), link-local (`169.254/16`, `fe80::/10`), loopback (`127/8`, `::1`), IPv6 ULA (`fc00::/7`), CGNAT (`100.64/10`), multicast, broadcast, unspecified, and IPv4-mapped IPv6 of any of the above. Re-validates after redirects so an open-redirect cannot bounce the fetch into a private IP.
  - Body cap — streamed read aborts at 5 MB. Per-chunk no-progress timeout (10 s) and total timeout (30 s) so a slow-loris server cannot wedge the tool loop.
  - Content-type gate — only `text/html` and `application/xhtml+xml` are accepted; PDFs, JSON, images, and plain text are rejected with a clear error so the model can adapt.
  - User-Agent — identifies as `Wren/0.7.0 (+https://github.com/basezero-projects/wren)` so site operators can filter or rate-limit.

  No UI surface beyond the existing tool-approval card and tool-result chip — the user sees the URL on the approval card, clicks Allow, and the article body lands as a tool result the model can quote from. Listed in the tool catalog returned by `tool_definitions()`; gated by `is_destructive("fetch_url") -> true` so the chat tool loop pauses for user approval before dispatch.

  Implementation surface: new `fetch_url` async function in `src-tauri/src/tools.rs`, plus helpers `parse_fetch_url`, `validate_fetch_host`, `is_blocked_ip`, `content_type_is_html`, `extract_article`, and a process-wide `OnceLock<reqwest::Client>` so we don't rebuild a TLS stack per fetch. The helper layer is fully unit-tested (32 new tests covering scheme/host/IP validation, content-type matching, extraction, truncation, and dispatch routing). New Cargo dep: `dom_smoothie = "0.17"` (active Mozilla Readability port, MIT, no native deps; emits markdown directly via `TextMode::Markdown`).

## [0.6.0] — 2026-04-29

### Added

- **Push-to-talk voice input.** Settings gains a new "Voice" tab (microphone icon, between Web and Display). Hold <kbd>Ctrl+Shift+Space</kbd> while the Wren overlay is open to dictate; release to transcribe. The transcript appends to whatever you have already typed in the input box (with a leading space when the input has prior text), so you can mix typing and speaking in the same prompt without one clobbering the other. Audio never leaves the machine — Wren runs whisper.cpp locally with whichever ggml model you picked. The hotkey is overlay-only by design (a no-op while Wren is hidden) so it never fights with dictation in unrelated apps.

  Voice tab layout:
  - **Push-to-talk section** — explainer line plus an "Enable voice input" checkbox row. The checkbox shows "On — hotkey active" or "Off"; off by default so a fresh install never starts capturing audio without the user opting in.
  - **Active model section** — only renders when at least one ggml model is on disk. A dropdown row labeled "Whisper model" lists every installed model with its filename and disk size (e.g. `ggml-base.en-q5_1.bin (60.5 MB)`); the first option is "— No model selected —" so the user can deliberately blank it.
  - **Install a model section** — a curated four-option dropdown (Tiny / Base / Small / Medium English variants, all q5_1 quantized) plus an "Install" button. Selecting a size shows a one-line description of the speed/accuracy trade-off ("Instant transcription, more mistakes on tricky words…", "Catches unusual words and accents. ~2-3s pause on release.", etc.). The Install button streams from the public `ggerganov/whisper.cpp` HuggingFace repo and shows the same gold progress bar + byte-counter UI as the Ollama pull. A Cancel button drops the download cleanly. Dismiss button on the Done / Error / Cancelled callouts. Auto-installs become the active model on first install if no model was set.
  - **Installed models section** — appears once anything is installed. Each row shows the filename in monospace, the file size, and a small inline trash icon. Clicking the trash inline-flips into a Delete / Cancel two-button confirm. The active model gets a gold left-border and an `ACTIVE` chip. Deleting the active model also blanks the config field so Wren never holds a dangling reference.

  Hotkey behavior:
  - Press: backend emits `wren://voice-hotkey` with `phase: "press"` → `useVoice` calls `voice_record` over a typed Tauri Channel. The Channel receives a sequence of `Listening` → `Transcribing` → `Final(text)` events, with `Cancelled` and `Error(msg)` for the failure paths.
  - Release: backend emits `phase: "release"` → frontend calls `voice_finalize`, the recording loop drops the mic stream and runs whisper on the buffered audio.
  - Hard cap: 5 minutes of recording per session — above that the loop forces a finalize so a stuck Ctrl+Shift never burns memory indefinitely.
  - Re-press while a session is still in flight: the backend cancels the stale session before installing the new one, so a flaky hotkey never stacks captures.

  Implementation surface: 
  - Cargo deps: `whisper-rs = "0.16"` (statically links whisper.cpp), `cpal = "0.17"` (cross-platform mic capture; uses WASAPI on Windows).
  - New `voice` module exposes `voice_record`, `voice_finalize`, `voice_cancel`, `download_whisper_model`, `cancel_whisper_download`, `list_whisper_models`, `delete_whisper_model` Tauri commands. The recording loop uses cpal on a dedicated OS thread (cpal's Stream is `!Send`), funnels f32 PCM into a `tokio::mpsc` channel, downmixes to mono, linearly resamples to 16 kHz, and runs whisper inference inside `spawn_blocking`. Sessions live in a new `VoiceState` Tauri-managed struct that holds the active oneshot finish-sender and a download cancel token.
  - `download_whisper_model` mirrors the `model_pull` shape: same per-chunk timeout (60 s), same total cap (30 min), same Channel-based event stream (`Status` / `Progress { total, completed }` / `Done` / `Cancelled` / `Error`). Files land at `<app_data_dir>/whisper-models/<filename>` with a `.part` write-then-rename so a cancel mid-download leaves no broken file behind.
  - Filename validation rejects path traversal and non-`ggml-*.bin` shapes everywhere — `voice_record`, `download_whisper_model`, `delete_whisper_model`, and `list_whisper_models` all run the same `validate_model_filename` gate before touching the filesystem.
  - Voice config: new `[voice]` section in `config.toml` with two fields, `enabled` (bool) and `model` (filename). Both routed through the existing `set_config_field` allowlist; both have ?-tooltip helper copy in Settings explaining the trade-offs.

  Build dependencies: whisper-rs needs LLVM (`libclang.dll` for bindgen) and CMake (to build whisper.cpp from source). New `src-tauri/.cargo/config.toml` pins the standard Windows install paths via `LIBCLANG_PATH` and `CMAKE` env vars so a clean clone builds without manual env setup. Both override-friendly (`force = false`) for contributors with non-default install locations.

  Streaming partial transcripts and a manual mic button on the AskBar are deferred — push-to-talk on a curated small-model lineup gets the feature usable today; the streaming UX lands in a future release once a smaller streaming-friendly whisper model is wired in.

## [0.5.0] — 2026-04-29

### Added

- **Manage installed models from inside Wren.** Settings → AI gains an "Installed models" section right under the Pull field. Lists every model in your local Ollama with file size, sorted by most-recently-modified so a freshly-pulled model jumps to the top. A header row shows total count and aggregate disk usage (`3 models installed (12.74 GB)`). Each row has a small red trash icon on the right; clicking it inline-flips into a Delete / Cancel two-button confirm — no scary modal dialog, just enough friction that you do not nuke a 30 GB pull by accident. Refresh button forces a re-fetch.

  Backend: two new Tauri commands wrap Ollama's `/api/tags` (rich variant — name, size, modified_at) and `/api/delete`. `list_installed_models` enriches the existing slug-only fetcher used by the active-model picker without disturbing it. `delete_model` runs the slug through the existing shape validator before any network call. Both surface clean string errors when Ollama is unreachable, returns a non-2xx, or rejects a delete (e.g. "model not found").

  Combined with the 0.4.x install field, Settings → AI now covers the full lifecycle: pull a model from Ollama or HuggingFace, see what's on disk, free up space when you no longer need something. No terminal trip required for any of it.

## [0.4.1] — 2026-04-29

### Changed

- **Install-a-model field documents HuggingFace GGUF support.** The 0.4.0 helper text framed the field as "Ollama library only," which was wrong — Ollama natively resolves `hf.co/<owner>/<repo>:<quant>` slugs and pulls the GGUF directly. Wren's pull command just forwards to `/api/pull`, so HuggingFace models worked from day one of the feature; the docs were the only thing missing. Settings now lists both formats with examples for each. The placeholder string in the input also shows both shapes (`qwen3:8b  or  hf.co/owner/repo:Q4_K_M`).

  Real "not yet supported" cases for clarity: non-GGUF HF repos (raw `safetensors` / `pytorch_model.bin`), custom Modelfile params (context-window override, custom stop tokens), and private HF repos with token auth. Those still need terminal work via `ollama create -f Modelfile` after the initial download.

## [0.4.0] — 2026-04-29

### Added

- **Install models from inside Wren — no terminal trip.** Settings → AI now has an "Install a model" section above the existing fields. Type any Ollama-library slug (`qwen3:8b`, `gemma3:12b`, `qwen2.5vl:7b`, …) and click Pull. Wren streams the download from Ollama's `/api/pull` endpoint and shows live progress: a status line, a gold progress bar, and a `1.2 GB / 4.7 GB (26%)` running counter. Cancel button drops the connection mid-download. Done / Error / Cancelled states each get their own coloured callout with a Dismiss control.

  Backend: new `model_pull` module wraps `POST /api/pull` and streams typed `PullEvent` chunks over a Tauri Channel — `Status`, `Progress {digest, total, completed}`, `Done`, `Cancelled`, `Error`. The pull cancellation token piggybacks on `GenerationState`, so the existing `cancel_generation` command also stops a running pull. A 60-second per-chunk no-progress timeout surfaces a clear error if Ollama or the network goes dark mid-download instead of leaving the user staring at a frozen bar; a 4-hour total cap protects against a connection that hangs forever (a 70B-class model on a slow line can legitimately take that long).

  Aggregation across multi-layer pulls happens in the frontend: each digest reports its own total/completed, the UI keeps the latest values per digest and sums them so the bar reflects the whole download even when Ollama emits interleaved progress lines for different layers. `arbitrary URL imports` (HuggingFace GGUFs, custom Modelfiles) are not in this release; v1 is library slugs only.

## [0.3.1] — 2026-04-29

### Added

- **Settings gear in the overlay toolbar.** Settings was previously only reachable through the Windows system-tray icon — fine if you knew about it, easy to miss otherwise. The collapsed ask bar and the chat-mode toolbar now both show a small gear next to the History clock; clicking it opens the Settings window directly. New Tauri command `open_settings` exposes the existing `show_settings_window` to the frontend.

## [0.3.0] — 2026-04-29

### Added

- **Tool model is now configurable from Settings.** Settings → AI now has a "Tool model" field next to the Ollama URL. Anyone running Wren can point the tool route at whichever model they want without forking the repo. Three behaviours fall out of one knob:

  1. **Leave it empty** (default): Wren uses your active chat model for tool calls. Works great if your chat model already supports tool calling — one model loaded, no VRAM thrash, full personality on every turn.
  2. **Leave it empty AND your chat model cannot tool-call**: Wren falls back to a built-in `qwen3:8b` default. First-run still works without any setup; the user just needs `ollama pull qwen3:8b` once.
  3. **Set it to a specific slug**: Wren uses exactly that model for tool calls regardless of which chat model is active. Lets you keep a tiny chat model for casual replies and a beefy tool-capable model for actions.

  Setting the tool model to the same slug as the active chat model puts Wren in **single-model mode**: there is no second model loaded into VRAM and the tool route uses your full chat-mode system prompt (personality, communication style, everything) plus a short tool-usage suffix. Anyone running a recent multi-capability model — qwen2.5-vl variants, qwen3 variants, llama3.3 with tools, multimodal Mistral — gets the most efficient possible setup with one config field.

### Changed

- **Tool-route system prompt adapts to which model is running it.** When the tool model is the same as the chat model (single-model mode) Wren now replays the user's full chat persona prompt and only adds a short tool-usage suffix. The slim, tool-focused prompt only kicks in when a separate tool model is configured. Same conversation, consistent voice — no more "different models talking" feeling on tool turns.
- **`TOOL_MODEL` constant renamed to `FALLBACK_TOOL_MODEL`** to make its role explicit. New `resolve_tool_model` function holds the full resolution order (config override → active chat model when capable → fallback) and is unit-testable.

## [0.2.7] — 2026-04-29

### Changed

- **Approval card resolves into a result-first layout.** While pending, the card still leads with the JSON arguments — that is what the user is consenting to. Once the user clicks Allow or Deny, the card pivots: the result line moves to the top with a green checkmark or red cross, the bubble border tints to match (green for success, red for tool error, grey for terminal-not-run), and the JSON arguments tuck behind a `▸ Show arguments` disclosure. The bubble still has full provenance for after-the-fact inspection but no longer dominates the chat with a wall of JSON after the work is done.

### Fixed

- **Removed the misleading "Reasoning is hidden from gemma3-heretic" banner.** With the tool router selecting qwen3 (a thinking model) regardless of the active chat-model pick, thinking content in conversation history can come from either model. The legacy warning fired any time the active chat model lacked thinking support, regardless of which model produced the thinking, and incorrectly suggested the user switch chat models. The check is removed entirely; the other capability warnings (vision, image cap) still fire when relevant.

## [0.2.6] — 2026-04-29

### Added

- **Tool-loop heartbeat.** A `[tool model] starting…` thinking line now fires the moment the loop opens, before the first HTTP request to Ollama. Confirms the IPC channel is alive during the cold-load window so the user sees something happen instead of staring at empty space.
- **Per-chunk diagnostic log.** Every stream chunk now logs to the dev-tools console with its type and arrival timestamp. Lets us tell whether a "late-appearing" card is a delivery problem or a rendering problem.

### Fixed

- **Cancelled approval cards no longer hang as "Awaiting approval."** 0.2.5 added Cancelled-chunk handling to flip pending cards to "Cancelled — not run", but the chunk arrived after `activeGenerationRef` was already nulled, so `onmessage` dropped it and the cards stayed pending. The cleanup now happens synchronously inside `abortActiveGeneration` — at the same instant the ref is detached, every still-pending card on the assistant message is flipped to "Cancelled — not run".

## [0.2.5] — 2026-04-29

### Fixed

- **Approval cards no longer lie about what they did.** Clicking Allow on a card whose backing oneshot had already been cleaned up (typically because the user cancelled the generation, or the card sat unanswered past the 5-minute timeout) used to flip the card to "Allowed" and call it a day — even though no tool ran. The frontend now checks the boolean return of `approve_tool_call`. When the backend reports the entry was already gone, the card flips to a grey "Expired — not run" badge instead. Same behaviour for `Tauri invoke` errors; if we cannot prove the dispatch happened, we do not claim it did.
- **Cancelling a generation cancels every pending approval card too.** Previously a cancelled generation could leave an "Awaiting approval" card sitting in the bubble forever — clicking it would go straight into the "Expired" path now exposed above. The Cancelled chunk now flips every still-pending card on the assistant message to "Cancelled — not run" and removes the buttons, matching the truth that no tool will run.

### Added

- **Tool result line inline on the approval card.** After a destructive tool dispatches, the backend emits a new `ToolResult` chunk with the tool name, ok/error flag, and a one-line summary. The matching card grows a green-bordered "Result: Wrote 12 bytes to D:/tmp/wren-test.txt" footer (or red-bordered "Error: ..." on failure). Users can finally tell whether their tool call actually did the thing without checking the file system to confirm.

## [0.2.4] — 2026-04-29

### Fixed

- **Tool calls now happen in a second or two, not thirty.** qwen3 is a thinking model and was emitting thousands of reasoning tokens before every tool call by default — Ollama's prompt-eval and the thinking pass together routinely ran 30 to 60 seconds for "write hello world to a file." The tool-loop request payload now sets `"think": false`, which tells Ollama to skip the reasoning step entirely. The model jumps straight to the `tool_calls` array. A simple `write_file` test that previously needed 60 seconds completes in under two. Tool-call accuracy was unchanged in spot checks; the reasoning was making decisions the prompt already constrained.
- **Watchdog message no longer lies about its own timer.** 0.2.3 raised the frontend watchdog from 90 to 180 seconds but left the human-facing string hardcoded. Users saw "no response for 90 seconds" after waiting three minutes, which was confusing and made debugging harder. The string now interpolates the actual constant so it always matches reality.

## [0.2.3] — 2026-04-29

### Fixed

- **Tool-route prompts no longer time out on the first turn.** The tool route was replaying Wren's full chat-mode system prompt (~17,800 characters of personality, communication style, and value framing) on top of the 14-tool catalog and the user message. On a fresh model load, prompt evaluation alone took 30+ seconds and qwen3's thinking pass pushed the total past the 90-second frontend watchdog, so the user saw a "stopped hearing back" error after a long stare at the loading dots. The tool route now uses a slim system prompt focused on tool usage; the personality essay still wraps the chat route. First-turn latency drops dramatically.
- **Frontend watchdog raised to 180 seconds.** Even with the slim prompt, a cold-load of an 8B Q4 model on a busy GPU plus a long thinking-mode generation can run past 90s. The server-side per-chunk and request timeouts (60s, 120s) still fire first when Ollama actually misbehaves; this watchdog only catches the case where the IPC channel itself is dead.

## [0.2.2] — 2026-04-29

### Fixed

- **Prompts no longer cancel themselves on submit.** 0.2.1 added a `generation.cancel()` call inside `notify_frontend_ready` to recover from orphaned generations after a backend restart. Under React's `StrictMode` (and during Vite HMR), the frontend invokes `notify_frontend_ready` twice on every mount, which fired the cancel during a legitimate in-flight prompt. The send button immediately reset, the assistant bubble was silently removed, and the user saw nothing happen. The cancel hook is reverted; the 90-second frontend watchdog and the server-side request and per-chunk timeouts already cover the orphaned-generation case the cancel hook was trying to address.

## [0.2.1] — 2026-04-29

### Added

- **Guardrails against silent hangs.** Wren now treats every long-running operation as something that can fail and surfaces a clear error rather than spinning the loading dots forever.

  **Backend.** The non-streaming tool-loop POST to Ollama gets a hard 120-second request timeout. The streaming chat path adds a 60-second per-chunk timeout — if Ollama goes silent mid-stream (runner crash, daemon restart) Wren emits a "Stalled" error instead of waiting on a dead socket. The destructive-tool approval card auto-denies after 5 minutes so the user is never stuck looking at "Awaiting approval." `run_shell` runs through `tokio::process::Command` with a 30-second timeout; the child is killed on expiry via `kill_on_drop`. Tool dispatch errors surface as a `[tool] name -> Error: ...` thinking line so the user can see when a tool call failed inside the loop.

  **Frontend reload recovery.** `notify_frontend_ready` now calls `generation.cancel()` whenever the frontend (re)mounts. Hot-reloading the dev server, killing and reopening the overlay, or any other event that orphans an in-flight generation now cleans up the Rust state cleanly. The Ollama runner unloads, every pending tool-approval sender drops, and the next prompt starts fresh.

  **Frontend watchdog.** `useOllama.ts` arms a 90-second no-progress timer at the start of every turn and resets it on every chunk. If the IPC channel itself dies — the case that prompted this release, where a `tauri.conf.json` change hot-reloaded the backend mid-prompt — the watchdog fires, replaces the assistant bubble with a clear error, and resets `isGenerating` so the user can retry without manually cancelling.

## [0.2.0] — 2026-04-29

### Added

- **Phase 2 destructive tools, with inline approval.** The tool catalog adds six write-class tools: `write_file`, `delete_file`, `run_shell`, `write_clipboard`, `open_url`, `launch_app`. When the model emits a tool call for any of these names, the Rust tool loop pauses and emits a new `ToolApprovalRequest` chunk. The frontend renders an inline card inside the assistant bubble showing the tool name in a gold pill, the JSON arguments verbatim in a scrollable code block, and two buttons: `Allow` and `Deny`. The card carries an "Awaiting approval" badge while pending; after a click the badge flips to `Allowed` (green) or `Denied` (red) and the buttons disappear. Behind the scenes a `oneshot::Sender<bool>` is registered against a UUID in `GenerationState`, the tool loop awaits the receiver, and the new `approve_tool_call(id, allowed)` Tauri command resolves it. Cancelling the generation while a card is up drops every pending sender — every awaiting `select!` in the loop sees `Cancelled` and returns. A denied call returns `Error: User denied permission to run \`<name>\`. Do not retry...` to the model so it can adapt instead of looping. The `[tool] name(args)` thinking line still fires after approval so the existing trace is preserved. Read-only tools dispatch without prompting; the gate is purely on `is_destructive(name)`.

  **Tool implementations.** `write_file` calls `fs::create_dir_all` on the parent before writing. `delete_file` refuses directories. `run_shell` runs through `cmd /C` on Windows and `sh -c` elsewhere, captures stdout and stderr (capped at 10 KB combined with a `[truncated]` marker), and returns exit code plus both streams. `write_clipboard` and `open_url` route through `arboard` and `cmd /C start`, respectively. `launch_app` spawns the executable detached and returns immediately. All six are gated through the same approval mechanism — there is no allowlist or auto-approve for "safe" commands in this release.

## [0.1.1] — 2026-04-29

### Added

- **Selection capture on Windows.** Highlight text in any app, hit `Alt+Space` (or `Ctrl+Space`), and Wren reads the selection. The user message arrives at the model wrapped as `[Highlighted Text]`, with the typed prompt as `[Request]` underneath, the same shape macOS already produced.

  The capture path: snapshot the existing clipboard, defensively release `VK_MENU`, send `Ctrl+C` via `SendInput`, sleep 80ms for the foreground app to populate the clipboard, read the new contents, restore the snapshot. If the new clipboard text matches the snapshot or is empty, no selection was made and `selected_text` stays `None`. The 80ms settle window was tuned against Word, VS Code, and Chromium-based apps; shorter values miss captures, longer values feel laggy.

  **Trade-offs.** If the clipboard held a non-text payload (image, file list) at capture time, that payload is lost — `arboard` only round-trips text and a richer restore would need raw Win32 clipboard-format plumbing. The hotkey handler skips capture when Wren is already foreground so the synthetic `Ctrl+C` cannot land in Wren's own input.

## [0.1.0] — 2026-04-29

First release. The fork, the Windows port, the rebrand, the theme, and a Phase-1 tool-calling layer all land in one push. Future releases will be incremental.

### Added

#### Tool calling, Phase 1 (read-only)

Wren routes between two local Ollama models per turn. The chat model (your pick from the in-app picker) handles conversation; the tool model (`qwen3:8b`, hard-coded for now) handles requests that need real data. Routing is rule-based:

- Action verbs at the start: `read`, `list`, `find`, `grep`, `show`, `open`, `ls`, `cat`.
- Path-shaped strings: `C:\`, `D:\`, `./`, `~`, `/`.
- Desktop keywords: `active window`, `clipboard`, `monitor info`, `list windows`.

`/tool <ask>` and `/chat <ask>` override the heuristic. Image attachments always go to chat.

Eight tools in the catalog: `read_file`, `list_dir`, `glob`, `grep_content`, `active_window`, `list_windows`, `monitor_info`, `read_clipboard`. Each one caps its own output and appends a `[truncated: ...]` marker so the model knows there is more. The loop is capped at 10 rounds per turn. Every invocation surfaces in the UI as a `[tool] name(args)` thinking line, so you can see what is happening during the cold-load.

#### Wren on Windows

Tauri 2 desktop overlay. Floats over everything via `alwaysOnTop`, decorations off, transparent window. `Alt+Space` toggles the overlay (chat persists). `Ctrl+Space` summons with a fresh chat. The window appears at the cursor, accepts text input, animates in and out, and morphs between a compact ask bar and an expanded chat as you type.

#### Win11 DWM polish

On startup Wren calls `DwmSetWindowAttribute(DWMWA_BORDER_COLOR=NONE, DWMWA_WINDOW_CORNER_PREFERENCE=DONOTROUND)` on the main and settings windows. Without this, Win11 paints a 1px border and rounds the corners on every top-level window, which shows up as a visible rectangle around an otherwise transparent overlay.

#### SYVR theme

Monochrome dark surfaces (`#0c0c0d`, `rgba(14,14,16,0.98)`) with a gold accent (`#d4af37`). The halo gradient on the morphing container is gold. User bubbles are a gold gradient with dark text; assistant bubbles are dark with a subtle gold border on top. Mac traffic-light buttons replaced with a Windows-style red-on-hover close on the right. The toolbar (model picker, save, new, history) sits on the left.

#### Screen capture

`/screen` and the screenshot button capture every display via the `screenshots` crate, downscaled to a max width of 1280px so vision models do not drown in tokens. Wren stays visible during the capture and accepts being in the screenshot itself.

#### Cancel-then-unload

Cancel does two things. The local `CancellationToken` fires immediately so the UI updates fast. In the background, a best-effort POST to `/api/generate` with `keep_alive: 0` tells Ollama to unload the runner from VRAM. Fans stop spinning. The next prompt cold-loads, around 30 seconds for a 7B Q4 model. Worth it.

### Changed

- **Project rename.** Identifier is `com.syvr.wren`, product name is `Wren`. The on-disk folder is still `backseat/` from a mid-fork rebrand pass. Will rename when no processes are touching it.
- **Package manager.** `pnpm` replaces `bun`. `bun.lock` is gone.
- **macOS-only commands gated.** Every macOS-specific Tauri invoke (NSPanel, the AX and screen-recording permission flow, the CGEventTap activator, AX context capture) is behind `cfg(target_os = "macos")`. The macOS build still works.
- **README and docs.** New Wren-specific README with Windows install steps, slash commands, and Logan Nguyen attribution. The Vietnamese-etymology copy is gone. Wren is named after the bird.

### Removed

- The Thuki bear logo. Replaced with `public/wren-logo.png`.
- `tauri-nspanel` from non-macOS builds.
- The silent auto-route to `qwen2.5vl:7b` on image-bearing requests. It was loading the vision model under the user's feet, which thrashed VRAM and crashed other apps on a 4090 already busy with other workloads. The capability filter strips images now and points the user at the picker.

### Notes

- Forks from `quiet-node/thuki@HEAD`.
- Phase 2 (write, delete, shell, launch tools behind per-call confirmation) is next.

