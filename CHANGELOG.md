# Changelog

All notable changes to **Wren** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Wren is a Windows port of [`quiet-node/thuki`](https://github.com/quiet-node/thuki) by Logan Nguyen (Apache-2.0). The pre-fork history of the upstream project is not reproduced here — see the upstream repository for that lineage. Wren's own changelog starts at `0.1.0`.

## [0.1.0] - 2026-04-29

First Wren release. The codebase was forked from `quiet-node/thuki@HEAD`, ported to Windows, rebranded, re-themed, and given a Phase-1 tool-calling layer in a single push. After this point each release captures incremental work on top.

### Added

- **Tool calling, Phase 1 (read-only).** Wren now routes between two local Ollama models on every turn. A chat model (user-selected from the in-app picker) handles conversation; a tool-capable model (`qwen3:8b`, hard-coded) handles requests that need real data. Routing is rule-based: action verbs at the start of the message (`read`, `list`, `find`, `grep`, `show`, `open`, `ls`, `cat`), path-shaped strings (`C:\`, `D:\`, `./`, `~`, `/`), or desktop keywords (`active window`, `clipboard`, `monitor info`, `list windows`) route to the tool model; everything else routes to the chat model. Slash overrides `/tool <ask>` and `/chat <ask>` win against the heuristic. Image attachments always route to chat. The catalog has 8 tools: `read_file`, `list_dir`, `glob`, `grep_content`, `active_window`, `list_windows`, `monitor_info`, `read_clipboard`. Each tool truncates its own output with a `[truncated: ...]` marker so the model knows there's more. Tool-call rounds are capped at 10 per turn for safety. Each tool invocation surfaces in the UI as a `[tool] name(args)` thinking line so the user can see what's happening during the cold-load and the loop.
- **Wren on Windows.** The app now runs on Windows 11 as a Tauri 2 desktop overlay. Floats above all apps via `alwaysOnTop`, decorations off, transparent window. **Hotkeys:** `Alt+Space` toggles the overlay (chat persists), `Ctrl+Space` summons with a fresh chat. The overlay appears at the cursor, accepts text input, animates entrance/exit, and morphs between a compact ask bar and an expanded chat as you type.
- **Win11 DWM polish.** On startup Wren calls `DwmSetWindowAttribute(DWMWA_BORDER_COLOR=NONE, DWMWA_WINDOW_CORNER_PREFERENCE=DONOTROUND)` against the main and settings windows so the default 1px Windows border and rounded-corner clip don't paint over an otherwise transparent overlay. No visible artifact around the chat bar.
- **SYVR brand styling.** Theme tokens in `src/App.css` are monochrome dark surfaces (`#0c0c0d`, `rgba(14,14,16,0.98)`) with a gold (`#d4af37`) accent. The halo gradient on the morphing container is gold; user message bubbles are a gold gradient with dark text; assistant bubbles are dark with a subtle gold border-top. The Mac traffic-light buttons in the title bar are replaced with a Windows-style red-on-hover close button on the right; the toolbar (model picker · save · new · history) sits on the left.
- **Manual screen capture, no overlay-hide.** `/screen` and the screenshot button capture all displays via the `screenshots` crate, downscaled to a maximum width of 1280px so vision models don't drown in tokens. Wren stays visible during the capture and accepts being in the screenshot itself.
- **Configurable two-model routing in the Ollama integration.** The active chat model is selected from the in-app picker; the tool model is hard-coded to `qwen3:8b` for the next release. `OLLAMA_KEEP_ALIVE=5m` and `OLLAMA_MAX_LOADED_MODELS=1` are recommended on the user side to avoid VRAM thrash.
- **Cancel-then-unload generation.** The cancel button (and any new generation request) cancels the local `CancellationToken` immediately AND fires a best-effort `POST /api/generate` with `keep_alive: 0` for the active model. The Ollama runner unloads from VRAM right away — fans stop spinning. Next prompt cold-loads (~30s for a 7B Q4 model); the tradeoff is intentional.

### Changed

- **Project rename.** Identifier is `com.syvr.wren`, product name is `Wren`. The on-disk folder is still `backseat/` from a mid-fork rebrand pass; will be renamed in a follow-up release once running processes can be killed cleanly.
- **Package manager.** `pnpm` replaces `bun` end-to-end. `bun.lock` removed; `pnpm-lock.yaml` is the lockfile.
- **macOS-only commands gated.** Every macOS-specific Tauri invoke (NSPanel manipulation, accessibility / screen-recording permission flows, CGEventTap activator, AX context capture) is now behind `cfg(target_os = "macos")`. The Windows build never registers them; the macOS build is unaffected.
- **README + docs.** Replaced the upstream README with a Wren-specific document that explains the port, attributes Logan Nguyen, lists Windows-specific install steps, and documents the slash commands and hotkeys actually shipped. The Vietnamese-etymology copy is gone — Wren is named after the bird.

### Removed

- The Thuki bear logo (`public/thuki-logo.png`). Replaced with `public/wren-logo.png`.
- The `tauri-nspanel` dependency from non-macOS builds. macOS builds still use it for the floating-panel implementation.
- The auto-route-to-vision-model path in `commands.rs::ask_ollama`. It was loading `qwen2.5vl:7b` underneath the user's selected chat model on image-bearing requests, which combined with the previous KEEP_ALIVE policy thrashed the GPU and crashed other apps on a 4090 already serving other workloads. Image attachments are now stripped by the existing capability filter when the active chat model isn't vision-capable; the user is prompted to switch via the picker chip.

### Notes

- This release **forks from `quiet-node/thuki@HEAD`**. Upstream commit `791c4ad` in this repo is the unmodified import; everything in `0.1.0` is on top of that.
- Phase 2 (destructive tools — `write_file`, `delete_file`, `run_shell`, `write_clipboard`, `open_url`, `launch_app` — gated behind a per-call confirmation modal) is the next major release.
