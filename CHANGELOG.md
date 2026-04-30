# Changelog

Wren's release notes. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and [SemVer](https://semver.org/spec/v2.0.0.html).

Wren is a Windows port of [`quiet-node/thuki`](https://github.com/quiet-node/thuki) (Apache-2.0). Upstream history is not reproduced here, see that repo for the pre-fork lineage. Wren's own log starts at `0.1.0`.

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

