<h1 align="center">Wren</h1>

<p align="center">
  <img src="public/wren-logo.png" alt="Wren logo" width="240" />
</p>

<p align="center">
  A floating, local-first AI overlay for Windows.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License" /></a>
  <img src="https://img.shields.io/badge/platform-Windows-0078D6?logo=windows&logoColor=white" alt="Platform: Windows" />
  <img src="https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri&logoColor=white" alt="Tauri v2" />
  <img src="https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black" alt="React 19" />
  <img src="https://img.shields.io/badge/Rust-stable-CE422B?logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/Ollama-local-black" alt="Ollama" />
</p>

---

Wren is a floating AI overlay that talks to a locally running [Ollama](https://ollama.com) instance on your machine. Press a hotkey, ask a question, get an answer, dismiss. Nothing leaves your computer.

This project is a **Windows port** of [`quiet-node/thuki`](https://github.com/quiet-node/thuki) — a macOS-only floating AI secretary by Logan Nguyen. The macOS-specific bits (NSPanel, Core Graphics event taps, Accessibility / Screen Recording permission dance) have been swapped for Windows-equivalents (DWM border suppression, `tauri-plugin-global-shortcut`, the `screenshots` crate, Win32 desktop introspection). Wren also adds tool-calling support via a second local model so the assistant can read your filesystem, list windows, and check the clipboard when relevant.

Wren is licensed under Apache-2.0, the same license as the upstream project.

## What it does

- **Always available.** A global hotkey summons the overlay from any app, including fullscreen apps. The window floats above everything.
- **Local-only.** All inference runs through Ollama on `localhost`. No API keys, no accounts, no telemetry, no cloud.
- **Two-model setup.** A chat model handles conversation; a tool-capable model (`qwen3:8b`) handles requests that need real data — file listings, foreground-window introspection, clipboard reads, etc. Wren routes between them automatically.
- **Highlighted-text and screenshot context.** Highlight text anywhere or capture your screen, and Wren sends it as part of your question.
- **Persistent conversations.** Local SQLite history. Open the overlay, your last conversation is still there.
- **SYVR brand styling.** Monochrome dark surfaces with a gold accent (`#d4af37`).

## Hotkeys

| Hotkey | Action |
|--------|--------|
| `Alt+Space` | Toggle overlay visibility — chat persists across opens |
| `Ctrl+Space` | Show overlay with a fresh chat (clears state) |

The hotkeys are registered globally via [`tauri-plugin-global-shortcut`](https://crates.io/crates/tauri-plugin-global-shortcut). They will fire from any app.

## Slash commands

Type these at the start of your message in the ask bar:

| Command | Effect |
|---------|--------|
| `/think <question>` | Ask the model to reason through it before answering |
| `/screen <question>` | Capture your full screen and attach it as context |
| `/search <query>` | Live web search via the optional search sandbox |
| `/translate`, `/rewrite`, `/tldr`, `/refine`, `/bullets`, `/todos` | Prompt shortcuts on highlighted or quoted text |
| `/tool <ask>` | Force-route to the tool-capable model |
| `/chat <ask>` | Force-route to the chat model |

Without a slash prefix, Wren picks between the chat and tool model based on simple intent heuristics (action verbs, file paths, desktop keywords).

## Tools (Phase 1, read-only)

When the request looks like it needs real data, Wren forwards it to a tool-capable model with this catalog:

- `read_file(path)` — read a UTF-8 text file
- `list_dir(path)` — list directory entries
- `glob(pattern, root?)` — find files by glob pattern
- `grep_content(needle, pattern, root?)` — substring search inside files
- `active_window()` — title + process of the foreground window
- `list_windows()` — all visible top-level windows
- `monitor_info()` — connected monitors with resolution + position
- `read_clipboard()` — current clipboard text

All tools are read-only in this phase. Destructive tools (write, delete, shell, launch) will land behind a per-call confirmation modal in a future release.

## Getting started

### 1. Install Ollama and pull two models

[Install Ollama](https://ollama.com/download), then pull a chat model and the tool model:

```bash
ollama pull gemma3:12b      # or any chat model you like
ollama pull qwen3:8b        # tool-capable, used for the tool route
```

Wren will let you pick the chat model from its in-app picker. The tool model is currently hard-coded to `qwen3:8b`.

Recommended environment variables (set permanently if you use Ollama for other things too):

```
OLLAMA_KEEP_ALIVE=5m            # don't pin models in VRAM after idle
OLLAMA_MAX_LOADED_MODELS=1      # one model resident at a time
```

### 2. Build and run Wren

Wren is a [Tauri 2](https://v2.tauri.app/) app — Rust backend, React 19 / TypeScript / Tailwind 4 frontend.

```bash
git clone https://github.com/basezero-projects/wren.git
cd wren
pnpm install

# Dev (Vite HMR + Tauri dev window):
pnpm tauri dev

# Production build:
pnpm tauri build
```

Requires:

- [Rust toolchain](https://rustup.rs/) (stable)
- [pnpm](https://pnpm.io/installation)
- [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for Windows (WebView2 is bundled with Windows 11; Visual Studio Build Tools are required for the Rust side)

### 3. Use it

After the first launch, pick your chat model from the picker chip on the left of the toolbar. Then:

- Press **Alt+Space** anywhere to summon the overlay.
- Type a question. Press Enter.
- Press **Alt+Space** again to dismiss; your chat persists.
- Press **Ctrl+Space** to summon with a fresh chat.

## Architecture

- **Frontend** (`src/`) — React 19 / TypeScript / Tailwind 4 / Framer Motion. The UI morphs between a compact ask-bar and an expanded chat. Streaming uses Tauri's Channel API.
- **Backend** (`src-tauri/src/`) — Tauri 2 app bootstrap, Ollama HTTP streaming, Win32 DWM polish (suppresses Win11's default border + rounded corners on transparent overlays), global hotkey, screen capture (downscaled to 1280px for vision-friendly token counts), tool-call loop.
- **Tooling layer** (`src-tauri/src/tools.rs`) — JSON-Schema definitions sent to Ollama plus a single `dispatch(name, args)` entrypoint that runs each tool in-process.
- **Routing** (`src-tauri/src/commands.rs::route_message`) — rule-based: action verbs, path-shaped strings, desktop keywords → tool model; everything else → chat model. Slash overrides (`/tool`, `/chat`) win.
- **History** — SQLite via `rusqlite`, stored under the Tauri app data directory.

See [`docs/configurations.md`](docs/configurations.md) for the user-tunable config schema.

## Status

Wren is early. Expect rough edges. The chat path is solid; tool calling is in active development (Phase 1 read-only is the current state, Phase 2 destructive-with-confirmation is next).

## Acknowledgements

Wren is forked from [`quiet-node/thuki`](https://github.com/quiet-node/thuki) by Logan Nguyen. The macOS implementation, the streaming Ollama transport, the configuration system, the agentic search sandbox, and most of the UX shape are his work. Wren ports it to Windows, reskins it, and adds tool calling.

This project is licensed under the [Apache License 2.0](LICENSE), same as upstream.
