# Changelog

Wren's release notes. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and [SemVer](https://semver.org/spec/v2.0.0.html).

Wren is a Windows port of [`quiet-node/thuki`](https://github.com/quiet-node/thuki) (Apache-2.0). Upstream history is not reproduced here, see that repo for the pre-fork lineage. Wren's own log starts at `0.1.0`.

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
