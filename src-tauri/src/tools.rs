/*!
 * Wren tool catalog (Phase 1: read-only)
 *
 * Defines the JSON schemas for tool definitions sent to Ollama and a
 * `dispatch` entrypoint that executes each tool by name. All tools in this
 * phase are read-only; nothing here writes to disk, the clipboard, or the
 * network. Phase 2 will add destructive tools behind a confirmation modal.
 *
 * Output budgets are deliberately small. Tool results re-enter the model
 * context on every turn, so an unbounded directory listing or file read can
 * blow the context window in two calls. Each tool truncates its own output
 * with a clear `[truncated: ...]` marker so the model knows there is more.
 */

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

/// Maximum bytes of text returned by `read_file` before truncation.
const READ_FILE_MAX_BYTES: usize = 50_000;
/// Maximum directory entries returned by `list_dir`.
const LIST_DIR_MAX_ENTRIES: usize = 500;
/// Maximum paths returned by `glob`.
const GLOB_MAX_RESULTS: usize = 200;
/// Maximum matches returned by `grep_content`.
const GREP_MAX_MATCHES: usize = 100;
/// Maximum windows returned by `list_windows`.
const LIST_WINDOWS_MAX: usize = 100;

/// Returns true for tools that mutate the user's machine in any visible
/// way OR that reach out across the network to an arbitrary host. The
/// tool loop pauses on these and asks for explicit user approval before
/// dispatching. Read-only local tools dispatch without prompting.
///
/// `fetch_url` is gated here even though it does not write to disk: a
/// drive-by request to an attacker-controlled URL is the SSRF risk we
/// want the user to see and acknowledge. The approval card shows the
/// full URL so the user can spot a suspicious host before it is hit.
///
/// Every `mcp__<server>__<tool>` is destructive by default for the same
/// "I do not know what this server does" reason — secret managers,
/// finance backends, file servers, custom shells are all in scope. The
/// approval card surfacing server + tool + args is the user's gate.
pub fn is_destructive(name: &str) -> bool {
    if crate::mcp::is_mcp_tool_name(name) {
        return true;
    }
    matches!(
        name,
        "write_file"
            | "delete_file"
            | "run_shell"
            | "write_clipboard"
            | "open_url"
            | "launch_app"
            | "fetch_url"
    )
}

/// JSON-Schema-style tool definitions for the Ollama `/api/chat` `tools`
/// field. Names and descriptions are written for the model, not the user;
/// keep them precise.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool_def(
            "read_file",
            "Read the UTF-8 text content of a file. Returns the file's contents (truncated if very large). Use absolute paths.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the file." }
                },
                "required": ["path"]
            }),
        ),
        tool_def(
            "list_dir",
            "List the entries (files and subdirectories) of a directory. Returns one entry per line: '[D] name' for directories, '[F  size]  name' for files.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the directory." }
                },
                "required": ["path"]
            }),
        ),
        tool_def(
            "glob",
            "Find files matching a glob pattern. Supports * and ** wildcards. Pattern may be absolute (e.g. 'D:/Work/**/*.rs') or relative when 'root' is given.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern, e.g. '**/*.toml' or 'D:/Work/**/*.rs'." },
                    "root": { "type": "string", "description": "Optional base directory for a relative pattern." }
                },
                "required": ["pattern"]
            }),
        ),
        tool_def(
            "grep_content",
            "Search for a substring inside files matching a glob pattern. Returns matching lines with file path and line number.",
            json!({
                "type": "object",
                "properties": {
                    "needle": { "type": "string", "description": "Substring to search for (case-insensitive)." },
                    "pattern": { "type": "string", "description": "Glob pattern to limit the search, e.g. '**/*.rs'." },
                    "root": { "type": "string", "description": "Optional base directory for a relative pattern." }
                },
                "required": ["needle", "pattern"]
            }),
        ),
        tool_def(
            "active_window",
            "Get the title and process name of the foreground (currently focused) window on the user's desktop.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool_def(
            "list_windows",
            "List all visible top-level windows on the user's desktop, with title and process name.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool_def(
            "monitor_info",
            "List all connected monitors with their resolution, position, and primary flag.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool_def(
            "read_clipboard",
            "Read the current text contents of the system clipboard.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool_def(
            "write_file",
            "Create or overwrite a UTF-8 text file at an absolute path. Requires user approval before running. Use sparingly.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path." },
                    "content": { "type": "string", "description": "Full file contents to write." }
                },
                "required": ["path", "content"]
            }),
        ),
        tool_def(
            "delete_file",
            "Delete a single file at an absolute path. Requires user approval. Will not delete directories.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the file." }
                },
                "required": ["path"]
            }),
        ),
        tool_def(
            "run_shell",
            "Run a shell command and return its stdout, stderr, and exit code. Requires user approval before running. On Windows the command is executed via cmd /C.",
            json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Full command line to run." }
                },
                "required": ["command"]
            }),
        ),
        tool_def(
            "write_clipboard",
            "Replace the current contents of the system clipboard with the given text. Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Text to put on the clipboard." }
                },
                "required": ["text"]
            }),
        ),
        tool_def(
            "open_url",
            "Open an http or https URL in the user's default browser. Requires user approval. Other URL schemes are rejected.",
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Full http or https URL." }
                },
                "required": ["url"]
            }),
        ),
        tool_def(
            "launch_app",
            "Launch an executable by absolute path or by name (e.g. 'notepad', 'code'). Requires user approval. Returns immediately; output is not captured.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path or executable name on PATH." },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional arguments to pass to the executable."
                    }
                },
                "required": ["path"]
            }),
        ),
        tool_def(
            "fetch_url",
            "Fetch an http(s) URL and return the readable article text (Mozilla Readability) as markdown. Use this to read documentation, articles, or any web page the user references. Requires user approval. Only text/html responses are supported; private/loopback hosts are rejected.",
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Full http or https URL." },
                    "format": {
                        "type": "string",
                        "enum": ["markdown", "text"],
                        "description": "Output format. Defaults to 'markdown'."
                    }
                },
                "required": ["url"]
            }),
        ),
    ]
}

fn tool_def(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}

/// Dispatch a tool call from the model. Returns the textual result to feed
/// back as a `tool` role message. All errors are returned as `Ok(String)`
/// with an error description so the model can react and retry rather than
/// the whole loop aborting.
///
/// Names matching `mcp__<server>__<tool>` are routed to the MCP registry
/// before the built-in match. `dispatch_mcp_tool` already returns its
/// failures as `Error: ...` strings so the chat-loop's `ok` heuristic
/// (`!result.starts_with("Error:")`) keeps working uniformly.
pub async fn dispatch(name: &str, args: &Value) -> String {
    if crate::mcp::is_mcp_tool_name(name) {
        return crate::mcp::dispatch_mcp_tool(name, args).await;
    }
    let result: Result<String, String> = match name {
        "read_file" => read_file(args),
        "list_dir" => list_dir(args),
        "glob" => glob_tool(args),
        "grep_content" => grep_content(args),
        "active_window" => active_window(),
        "list_windows" => list_windows(),
        "monitor_info" => monitor_info(),
        "read_clipboard" => read_clipboard(),
        "write_file" => write_file(args),
        "delete_file" => delete_file(args),
        "run_shell" => run_shell(args).await,
        "write_clipboard" => write_clipboard(args),
        "open_url" => open_url_tool(args),
        "launch_app" => launch_app(args),
        "fetch_url" => fetch_url(args).await,
        other => Err(format!("Unknown tool: {other}")),
    };
    match result {
        Ok(s) => s,
        Err(e) => format!("Error: {e}"),
    }
}

#[derive(Deserialize)]
struct PathArg {
    path: String,
}

#[derive(Deserialize)]
struct GlobArg {
    pattern: String,
    #[serde(default)]
    root: Option<String>,
}

#[derive(Deserialize)]
struct GrepArg {
    needle: String,
    pattern: String,
    #[serde(default)]
    root: Option<String>,
}

fn parse<T: for<'de> Deserialize<'de>>(args: &Value) -> Result<T, String> {
    serde_json::from_value(args.clone()).map_err(|e| format!("invalid arguments: {e}"))
}

fn read_file(args: &Value) -> Result<String, String> {
    let a: PathArg = parse(args)?;
    let bytes = std::fs::read(&a.path).map_err(|e| format!("read {}: {e}", a.path))?;
    let total = bytes.len();
    let take = bytes.len().min(READ_FILE_MAX_BYTES);
    let text = String::from_utf8_lossy(&bytes[..take]).to_string();
    if total > READ_FILE_MAX_BYTES {
        Ok(format!(
            "{text}\n\n[truncated: showing first {READ_FILE_MAX_BYTES} of {total} bytes]"
        ))
    } else {
        Ok(text)
    }
}

fn list_dir(args: &Value) -> Result<String, String> {
    let a: PathArg = parse(args)?;
    let mut entries: Vec<(bool, String, u64)> = Vec::new();
    let read = std::fs::read_dir(&a.path).map_err(|e| format!("read_dir {}: {e}", a.path))?;
    for entry in read {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let (is_dir, size) = match entry.metadata() {
            Ok(m) => (m.is_dir(), m.len()),
            Err(_) => (false, 0),
        };
        entries.push((is_dir, name, size));
    }
    // Dirs first, then files; alpha within each.
    entries.sort_by(|a, b| match (a.0, b.0) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.1.to_lowercase().cmp(&b.1.to_lowercase()),
    });
    let total = entries.len();
    let truncated = total > LIST_DIR_MAX_ENTRIES;
    entries.truncate(LIST_DIR_MAX_ENTRIES);
    let mut out = String::new();
    for (is_dir, name, size) in entries {
        if is_dir {
            out.push_str(&format!("[D]                     {name}\n"));
        } else {
            out.push_str(&format!("[F  {size:>15}]  {name}\n"));
        }
    }
    if truncated {
        out.push_str(&format!(
            "\n[truncated: showing first {LIST_DIR_MAX_ENTRIES} of {total} entries]\n"
        ));
    }
    if out.is_empty() {
        out = "(empty directory)".to_string();
    }
    Ok(out)
}

fn resolve_pattern(pattern: &str, root: Option<&str>) -> String {
    match root {
        Some(r) if !r.is_empty() => {
            let sep = if r.ends_with('/') || r.ends_with('\\') {
                ""
            } else {
                "/"
            };
            format!("{r}{sep}{pattern}")
        }
        _ => pattern.to_string(),
    }
}

fn glob_tool(args: &Value) -> Result<String, String> {
    let a: GlobArg = parse(args)?;
    let full = resolve_pattern(&a.pattern, a.root.as_deref());
    let iter = glob::glob(&full).map_err(|e| format!("invalid glob '{full}': {e}"))?;
    let mut found: Vec<String> = Vec::new();
    let mut total = 0usize;
    for path in iter.flatten() {
        total += 1;
        if found.len() < GLOB_MAX_RESULTS {
            found.push(path.display().to_string());
        }
    }
    if found.is_empty() {
        return Ok(format!("(no matches for '{full}')"));
    }
    let mut out = found.join("\n");
    if total > GLOB_MAX_RESULTS {
        out.push_str(&format!(
            "\n\n[truncated: showing first {GLOB_MAX_RESULTS} of {total} matches]"
        ));
    }
    Ok(out)
}

fn grep_content(args: &Value) -> Result<String, String> {
    let a: GrepArg = parse(args)?;
    if a.needle.is_empty() {
        return Err("needle must be non-empty".to_string());
    }
    let needle_lower = a.needle.to_lowercase();
    let full = resolve_pattern(&a.pattern, a.root.as_deref());
    let iter = glob::glob(&full).map_err(|e| format!("invalid glob '{full}': {e}"))?;
    let mut matches: Vec<String> = Vec::new();
    let mut total = 0usize;
    for entry in iter {
        let Ok(path) = entry else { continue };
        if !path.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (lineno, line) in content.lines().enumerate() {
            if line.to_lowercase().contains(&needle_lower) {
                total += 1;
                if matches.len() < GREP_MAX_MATCHES {
                    let trimmed = if line.len() > 300 {
                        format!("{}…", &line[..300])
                    } else {
                        line.to_string()
                    };
                    matches.push(format!("{}:{}: {}", path.display(), lineno + 1, trimmed));
                }
            }
        }
    }
    if matches.is_empty() {
        return Ok(format!("(no matches for '{}' in '{full}')", a.needle));
    }
    let mut out = matches.join("\n");
    if total > GREP_MAX_MATCHES {
        out.push_str(&format!(
            "\n\n[truncated: showing first {GREP_MAX_MATCHES} of {total} matches]"
        ));
    }
    Ok(out)
}

// ─── Windows-only desktop introspection ───────────────────────────────────
//
// `active_window`, `list_windows`, and `monitor_info` use Win32 APIs. On
// macOS these tools are stubbed so the catalog stays uniform across
// platforms; the model still receives the same definitions and gets a
// "not supported on this platform" string back if it tries to call them.

#[cfg(windows)]
fn active_window() -> Result<String, String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Ok("(no foreground window)".to_string());
        }
        let title = window_title(hwnd);
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = process_name_for_pid(pid).unwrap_or_else(|| "<unknown>".to_string());
        Ok(format!("title: {title}\nprocess: {process}\npid: {pid}"))
    }
}

#[cfg(not(windows))]
fn active_window() -> Result<String, String> {
    Err("active_window is only implemented on Windows".to_string())
}

#[cfg(windows)]
fn list_windows() -> Result<String, String> {
    use std::sync::Mutex;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };

    static COLLECTOR: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());

    unsafe extern "system" fn enum_proc(hwnd: HWND, _l: LPARAM) -> BOOL {
        if !IsWindowVisible(hwnd).as_bool() {
            return TRUE;
        }
        let title = window_title(hwnd);
        if title.trim().is_empty() {
            return TRUE;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if let Ok(mut g) = COLLECTOR.lock() {
            g.push((title, pid));
        }
        TRUE
    }

    {
        let mut g = COLLECTOR.lock().map_err(|e| e.to_string())?;
        g.clear();
    }
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(0));
    }
    let collected: Vec<(String, u32)> = {
        let mut g = COLLECTOR.lock().map_err(|e| e.to_string())?;
        std::mem::take(&mut *g)
    };
    let total = collected.len();
    let truncated = total > LIST_WINDOWS_MAX;
    let mut out = String::new();
    for (title, pid) in collected.into_iter().take(LIST_WINDOWS_MAX) {
        let process = process_name_for_pid(pid).unwrap_or_else(|| "<unknown>".to_string());
        out.push_str(&format!("[{pid:>6}] {process} — {title}\n"));
    }
    if truncated {
        out.push_str(&format!(
            "\n[truncated: showing first {LIST_WINDOWS_MAX} of {total} windows]\n"
        ));
    }
    if out.is_empty() {
        out = "(no visible windows)".to_string();
    }
    Ok(out)
}

#[cfg(not(windows))]
fn list_windows() -> Result<String, String> {
    Err("list_windows is only implemented on Windows".to_string())
}

#[cfg(windows)]
fn monitor_info() -> Result<String, String> {
    use std::sync::Mutex;
    use windows::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };
    // `MONITORINFOF_PRIMARY` isn't re-exported from the Gdi module in this
    // version of the `windows` crate; it's just a bitflag value (0x1).
    const MONITORINFOF_PRIMARY: u32 = 0x00000001;

    static COLLECTOR: Mutex<Vec<String>> = Mutex::new(Vec::new());

    unsafe extern "system" fn enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _r: *mut RECT,
        _l: LPARAM,
    ) -> BOOL {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut info).as_bool() {
            let m = info.rcMonitor;
            let primary = (info.dwFlags & MONITORINFOF_PRIMARY) != 0;
            let line = format!(
                "{}{}x{} @ ({}, {}){}",
                if primary { "* " } else { "  " },
                m.right - m.left,
                m.bottom - m.top,
                m.left,
                m.top,
                if primary { "  [primary]" } else { "" },
            );
            if let Ok(mut g) = COLLECTOR.lock() {
                g.push(line);
            }
        }
        TRUE
    }

    {
        let mut g = COLLECTOR.lock().map_err(|e| e.to_string())?;
        g.clear();
    }
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(enum_proc), LPARAM(0));
    }
    let collected: Vec<String> = {
        let mut g = COLLECTOR.lock().map_err(|e| e.to_string())?;
        std::mem::take(&mut *g)
    };
    if collected.is_empty() {
        return Ok("(no monitors found)".to_string());
    }
    Ok(collected.join("\n"))
}

#[cfg(not(windows))]
fn monitor_info() -> Result<String, String> {
    Err("monitor_info is only implemented on Windows".to_string())
}

fn read_clipboard() -> Result<String, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("clipboard init: {e}"))?;
    match cb.get_text() {
        Ok(s) if s.is_empty() => Ok("(clipboard is empty)".to_string()),
        Ok(s) => Ok(s),
        Err(e) => Err(format!("clipboard read: {e}")),
    }
}

// ─── Destructive tools (Phase 2) ─────────────────────────────────────────
//
// Every function below mutates the user's machine. The tool loop in
// `commands.rs` blocks on user approval before reaching `dispatch` for
// these names; the implementations themselves do not re-prompt.

#[derive(Deserialize)]
struct WriteFileArg {
    path: String,
    content: String,
}

fn write_file(args: &Value) -> Result<String, String> {
    let a: WriteFileArg = parse(args)?;
    if let Some(parent) = std::path::Path::new(&a.path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir -p {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(&a.path, a.content.as_bytes()).map_err(|e| format!("write {}: {e}", a.path))?;
    Ok(format!("Wrote {} bytes to {}", a.content.len(), a.path))
}

fn delete_file(args: &Value) -> Result<String, String> {
    let a: PathArg = parse(args)?;
    let meta = std::fs::metadata(&a.path).map_err(|e| format!("stat {}: {e}", a.path))?;
    if meta.is_dir() {
        return Err(format!(
            "{} is a directory; refusing to delete (use a shell command if you really mean it)",
            a.path
        ));
    }
    std::fs::remove_file(&a.path).map_err(|e| format!("delete {}: {e}", a.path))?;
    Ok(format!("Deleted {}", a.path))
}

#[derive(Deserialize)]
struct ShellArg {
    command: String,
}

/// Maximum bytes captured from the command's stdout + stderr combined.
const SHELL_OUTPUT_MAX_BYTES: usize = 10_000;

/// Hard timeout on a single `run_shell` invocation. The child process is
/// killed on expiry. 30 seconds is a compromise between "long enough for
/// a real command" and "short enough that the user does not feel the
/// app has hung." If the model needs longer, it can break the work into
/// multiple calls.
const SHELL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

async fn run_shell(args: &Value) -> Result<String, String> {
    let a: ShellArg = parse(args)?;
    if a.command.trim().is_empty() {
        return Err("empty command".to_string());
    }
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", &a.command]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", &a.command]);
        c
    };
    cmd.kill_on_drop(true);

    let child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    let timeout_fut = tokio::time::sleep(SHELL_TIMEOUT);
    tokio::pin!(timeout_fut);

    let output = tokio::select! {
        out = child.wait_with_output() => out.map_err(|e| format!("wait: {e}"))?,
        _ = &mut timeout_fut => {
            // Child gets killed when `cmd` is dropped (kill_on_drop). Note
            // that we already moved into child + child.wait_with_output()
            // consumed it; the kill happens via the dropped future. To be
            // explicit, we return early with a clear message.
            return Err(format!(
                "Command exceeded {}s timeout and was killed.",
                SHELL_TIMEOUT.as_secs()
            ));
        }
    };

    let stdout = trim_to_limit(
        &String::from_utf8_lossy(&output.stdout),
        SHELL_OUTPUT_MAX_BYTES,
    );
    let stderr = trim_to_limit(
        &String::from_utf8_lossy(&output.stderr),
        SHELL_OUTPUT_MAX_BYTES,
    );
    let code = output
        .status
        .code()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".to_string());

    let mut out = format!("exit: {code}\n");
    if !stdout.is_empty() {
        out.push_str("--- stdout ---\n");
        out.push_str(&stdout);
        if !stdout.ends_with('\n') {
            out.push('\n');
        }
    }
    if !stderr.is_empty() {
        out.push_str("--- stderr ---\n");
        out.push_str(&stderr);
    }
    if stdout.is_empty() && stderr.is_empty() {
        out.push_str("(no output)\n");
    }
    Ok(out)
}

fn trim_to_limit(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!(
            "{}\n[truncated: showing first {max} of {} bytes]",
            &s[..max],
            s.len()
        )
    }
}

#[derive(Deserialize)]
struct ClipboardWriteArg {
    text: String,
}

fn write_clipboard(args: &Value) -> Result<String, String> {
    let a: ClipboardWriteArg = parse(args)?;
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("clipboard init: {e}"))?;
    cb.set_text(a.text.clone())
        .map_err(|e| format!("clipboard write: {e}"))?;
    Ok(format!(
        "Wrote {} characters to the clipboard.",
        a.text.chars().count()
    ))
}

#[derive(Deserialize)]
struct UrlArg {
    url: String,
}

fn open_url_tool(args: &Value) -> Result<String, String> {
    let a: UrlArg = parse(args)?;
    if !(a.url.starts_with("http://") || a.url.starts_with("https://")) {
        return Err("only http/https URLs are allowed".to_string());
    }
    let result = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &a.url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&a.url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&a.url).spawn()
    };
    result.map_err(|e| format!("spawn browser: {e}"))?;
    Ok(format!("Opened {}", a.url))
}

#[derive(Deserialize)]
struct LaunchArg {
    path: String,
    #[serde(default)]
    args: Option<Vec<String>>,
}

fn launch_app(args: &Value) -> Result<String, String> {
    let a: LaunchArg = parse(args)?;
    let mut cmd = std::process::Command::new(&a.path);
    if let Some(extra) = a.args.as_ref() {
        cmd.args(extra);
    }
    cmd.spawn().map_err(|e| format!("launch {}: {e}", a.path))?;
    Ok(format!(
        "Launched {}{}",
        a.path,
        a.args
            .as_ref()
            .filter(|v| !v.is_empty())
            .map(|v| format!(" {}", v.join(" ")))
            .unwrap_or_default()
    ))
}

// ─── Web fetch (Phase 2: webclaw) ────────────────────────────────────────
//
// `fetch_url` reaches out to an arbitrary URL, streams the body up to a
// hard byte cap, and runs Mozilla Readability over the HTML to return the
// article body as markdown. The model uses this to read documentation,
// articles, or any page the user references.
//
// Security model:
// 1. Marked `is_destructive` so every call goes through the inline
//    approval card. The user sees the URL before any byte hits the wire.
// 2. Scheme allowlist (http/https only).
// 3. Host blocklist (loopback, link-local, RFC1918, IPv6 ULA/link-local,
//    `.local`/`.internal`). Defense-in-depth against an LLM that asks for
//    `http://localhost:11434` or similar after the user clicks Allow on a
//    public URL — the IP check still blocks it.
// 4. Streamed body with a hard byte cap, total timeout, and per-chunk
//    no-progress timeout. Borrows the structural pattern from
//    `model_pull.rs` (no Channel — tool dispatch is one-shot).

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

/// Hard cap on raw response body bytes. Anything beyond is dropped and
/// the request is aborted.
const FETCH_MAX_BYTES: usize = 5 * 1024 * 1024;
/// Hard cap on the extracted markdown that we return to the model. Tool
/// results re-enter context on every turn — keep this on the same order
/// as `READ_FILE_MAX_BYTES`.
const FETCH_RESULT_MAX_BYTES: usize = 50_000;
/// Top-level timeout on a single fetch (connect + read).
const FETCH_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Per-chunk no-progress timeout. If the server accepts the connection
/// but goes silent we surface a clear error rather than blocking the
/// tool loop for the full total timeout.
const FETCH_CHUNK_TIMEOUT: Duration = Duration::from_secs(10);
/// User-Agent string. Identifies Wren's web tool to operators who want
/// to filter or rate-limit it.
const FETCH_USER_AGENT: &str = concat!(
    "Wren/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/basezero-projects/wren)"
);

#[derive(Deserialize)]
struct FetchUrlArg {
    url: String,
    #[serde(default)]
    format: Option<String>,
}

/// Lazily-built `reqwest::Client` shared across `fetch_url` calls. The
/// existing tool dispatch signature (`String -> String`) does not have a
/// `State<reqwest::Client>` plumbed through, and threading one through
/// would touch every existing tool for no benefit. A `OnceLock` keeps
/// the client cheap and reusable without changing the dispatch contract.
fn fetch_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(FETCH_USER_AGENT)
            .timeout(FETCH_TOTAL_TIMEOUT)
            // Cap redirect count — a redirect storm should fail fast.
            .redirect(reqwest::redirect::Policy::limited(8))
            .build()
            .expect("reqwest client built with valid options")
    })
}

/// Returns an error string if `host` is on the blocklist. Hosts that
/// parse as an IP go through `is_blocked_ip`; everything else goes
/// through a name-based blocklist (loopback, mDNS, common internal
/// suffixes).
fn validate_fetch_host(host: &str) -> Result<(), String> {
    // `Url::host_str` returns IPv6 addresses inside `[...]` brackets;
    // strip them before attempting to parse as `IpAddr`.
    let stripped = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    let host_lc = stripped.trim_end_matches('.').to_ascii_lowercase();
    if host_lc.is_empty() {
        return Err("URL has no host".into());
    }
    if let Ok(ip) = host_lc.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(format!(
                "host {host_lc} resolves to a private or loopback address"
            ));
        }
        return Ok(());
    }
    // Name-based block: loopback aliases and common internal suffixes.
    // The IP-based check above catches the canonical loopback addresses;
    // these names cover OS hosts files and split-horizon DNS.
    if host_lc == "localhost"
        || host_lc.ends_with(".localhost")
        || host_lc.ends_with(".local")
        || host_lc.ends_with(".internal")
    {
        return Err(format!("host {host_lc} is on the internal-name blocklist"));
    }
    Ok(())
}

/// True if `ip` is loopback, link-local, multicast, broadcast, an RFC1918
/// private range, an IPv6 ULA (`fc00::/7`), or unspecified. We
/// deliberately keep the list strict: any LLM-driven request to one of
/// these is almost certainly an SSRF attempt or a confused fetch of a
/// local dev service.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() || v4.is_unspecified() || v4.is_link_local() {
                return true;
            }
            if v4.is_private() || v4.is_multicast() || v4.is_broadcast() {
                return true;
            }
            // 100.64.0.0/10 — Carrier-Grade NAT. Almost always internal.
            let o = v4.octets();
            if o[0] == 100 && (64..=127).contains(&o[1]) {
                return true;
            }
            // 169.254.0.0/16 link-local already caught by `is_link_local`.
            false
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return true;
            }
            let segments = v6.segments();
            // fc00::/7 — Unique Local Addresses (RFC 4193).
            if (segments[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // fe80::/10 — link-local.
            if (segments[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            // Map IPv4-mapped IPv6 (::ffff:0:0/96) back to v4 rules.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip(IpAddr::V4(v4));
            }
            false
        }
    }
}

/// Validates and normalises a URL string. Returns `(parsed_url, host)`.
/// Defense-in-depth: even after the user approves the call, we re-check
/// scheme + host so a URL crafted to bypass the approval card text
/// (control chars, unicode lookalikes) cannot reach the network.
fn parse_fetch_url(raw: &str) -> Result<(reqwest::Url, String), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("URL is empty".into());
    }
    let url = reqwest::Url::parse(trimmed).map_err(|e| format!("invalid URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "unsupported URL scheme '{other}'; only http/https are allowed"
            ))
        }
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_string();
    validate_fetch_host(&host)?;
    Ok((url, host))
}

async fn fetch_url(args: &Value) -> Result<String, String> {
    let a: FetchUrlArg = parse(args)?;
    let format = match a.format.as_deref().unwrap_or("markdown") {
        "markdown" => dom_smoothie::TextMode::Markdown,
        "text" => dom_smoothie::TextMode::Formatted,
        other => {
            return Err(format!(
                "unsupported format '{other}'; use 'markdown' or 'text'"
            ))
        }
    };
    let (url, _host) = parse_fetch_url(&a.url)?;

    let response = fetch_client().get(url.clone()).send().await.map_err(|e| {
        if e.is_timeout() {
            "fetch timed out".to_string()
        } else if e.is_connect() {
            format!("could not connect to {url}")
        } else {
            format!("network error: {e}")
        }
    })?;

    let final_url = response.url().clone();
    // Re-validate the final host after redirects: an open-redirect could
    // bounce us into a private IP otherwise.
    if let Some(final_host) = final_url.host_str() {
        validate_fetch_host(final_host).map_err(|e| format!("redirect blocked: {e}"))?;
    }

    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !status.is_success() {
        return Err(format!("server returned HTTP {status}"));
    }

    if !content_type_is_html(&content_type) {
        return Err(format!(
            "unsupported content-type '{content_type}'; only text/html is supported"
        ));
    }

    // Stream the body, capping at FETCH_MAX_BYTES. Per-chunk timeout
    // protects against half-open connections going silent mid-body.
    let mut stream = response.bytes_stream();
    let mut body: Vec<u8> = Vec::new();
    let mut truncated = false;
    loop {
        let next = tokio::time::timeout(FETCH_CHUNK_TIMEOUT, stream.next()).await;
        match next {
            Err(_) => return Err("fetch stalled: no progress".into()),
            Ok(None) => break,
            Ok(Some(Err(e))) => return Err(format!("read error: {e}")),
            Ok(Some(Ok(bytes))) => {
                if body.len() + bytes.len() > FETCH_MAX_BYTES {
                    let take = FETCH_MAX_BYTES - body.len();
                    body.extend_from_slice(&bytes[..take]);
                    truncated = true;
                    break;
                }
                body.extend_from_slice(&bytes);
            }
        }
    }

    let html = String::from_utf8_lossy(&body).into_owned();
    extract_article(&html, &final_url, format, truncated)
}

/// True if `content_type` looks like an HTML document. Tolerates
/// charset/quality parameters (`text/html; charset=utf-8`) and the
/// XHTML variant.
fn content_type_is_html(content_type: &str) -> bool {
    let primary = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(primary.as_str(), "text/html" | "application/xhtml+xml")
}

/// Runs Mozilla Readability via dom_smoothie and formats the output for
/// the model. Truncates to `FETCH_RESULT_MAX_BYTES` with the standard
/// `[truncated: ...]` footer. Pure function — extracted so the unit
/// tests can exercise it without spinning up an HTTP server.
fn extract_article(
    html: &str,
    final_url: &reqwest::Url,
    text_mode: dom_smoothie::TextMode,
    body_truncated: bool,
) -> Result<String, String> {
    let cfg = dom_smoothie::Config {
        text_mode,
        ..Default::default()
    };
    let mut readability = dom_smoothie::Readability::new(html, Some(final_url.as_str()), Some(cfg))
        .map_err(|e| format!("readability init failed: {e}"))?;
    let article = readability
        .parse()
        .map_err(|e| format!("could not extract article: {e}"))?;

    let title_line = if article.title.trim().is_empty() {
        String::new()
    } else {
        format!("# {}\n", article.title)
    };
    let source_line = format!("Source: {final_url}\n\n");
    let body = article.text_content.to_string();

    let mut out = String::with_capacity(title_line.len() + source_line.len() + body.len());
    out.push_str(&title_line);
    out.push_str(&source_line);
    out.push_str(&body);

    let total = out.len();
    if total > FETCH_RESULT_MAX_BYTES {
        // Truncate at a UTF-8 char boundary — `String::truncate` panics
        // mid-codepoint, so walk back to the previous boundary first.
        let mut cut = FETCH_RESULT_MAX_BYTES;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str(&format!(
            "\n\n[truncated: showing first {cut} of {total} bytes of extracted text]"
        ));
    } else if body_truncated {
        out.push_str(&format!(
            "\n\n[truncated: response body exceeded {FETCH_MAX_BYTES} bytes; readability ran on the partial body]"
        ));
    }
    Ok(out)
}

// ─── Win32 helpers ────────────────────────────────────────────────────────

#[cfg(windows)]
unsafe fn window_title(hwnd: windows::Win32::Foundation::HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let mut buf = [0u16; 512];
    let len = GetWindowTextW(hwnd, &mut buf);
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

#[cfg(windows)]
fn process_name_for_pid(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len: u32 = buf.len() as u32;
        let res = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        if res.is_err() {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        // Just the file name, not the full path.
        Some(path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    // ── is_destructive ──────────────────────────────────────────────────

    #[test]
    fn fetch_url_is_destructive() {
        assert!(is_destructive("fetch_url"));
    }

    #[test]
    fn read_only_tools_are_not_destructive() {
        assert!(!is_destructive("read_file"));
        assert!(!is_destructive("active_window"));
        assert!(!is_destructive("unknown_tool"));
    }

    // ── tool_definitions ────────────────────────────────────────────────

    #[test]
    fn tool_definitions_includes_fetch_url() {
        let defs = tool_definitions();
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|d| d.get("function")?.get("name")?.as_str())
            .collect();
        assert!(names.contains(&"fetch_url"));
    }

    // ── is_blocked_ip ───────────────────────────────────────────────────

    #[test]
    fn ipv4_loopback_is_blocked() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 1, 2, 3))));
    }

    #[test]
    fn ipv4_rfc1918_is_blocked() {
        for ip in [
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(172, 16, 0, 1),
            Ipv4Addr::new(172, 31, 255, 254),
            Ipv4Addr::new(192, 168, 1, 1),
        ] {
            assert!(is_blocked_ip(IpAddr::V4(ip)), "{ip} should be blocked");
        }
    }

    #[test]
    fn ipv4_link_local_is_blocked() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
    }

    #[test]
    fn ipv4_multicast_and_broadcast_are_blocked() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255))));
    }

    #[test]
    fn ipv4_unspecified_is_blocked() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
    }

    #[test]
    fn ipv4_carrier_grade_nat_is_blocked() {
        // 100.64.0.0/10 — RFC 6598 CGNAT range.
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 127, 255, 254))));
    }

    #[test]
    fn ipv4_public_is_allowed() {
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 63, 255, 255))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1))));
    }

    #[test]
    fn ipv6_loopback_and_unspecified_are_blocked() {
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
    }

    #[test]
    fn ipv6_unique_local_is_blocked() {
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0xfc00, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0xfd12, 0x3456, 0x789a, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn ipv6_link_local_is_blocked() {
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn ipv6_multicast_is_blocked() {
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0xff02, 0, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn ipv6_mapped_v4_loopback_is_blocked() {
        // ::ffff:127.0.0.1
        let mapped = Ipv4Addr::new(127, 0, 0, 1).to_ipv6_mapped();
        assert!(is_blocked_ip(IpAddr::V6(mapped)));
    }

    #[test]
    fn ipv6_public_is_allowed() {
        // 2001:4860:4860::8888 — Google public DNS.
        assert!(!is_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888
        ))));
    }

    // ── validate_fetch_host ─────────────────────────────────────────────

    #[test]
    fn host_validation_allows_public_names() {
        assert!(validate_fetch_host("example.com").is_ok());
        assert!(validate_fetch_host("docs.rs").is_ok());
        // Trailing dot is FQDN-style; should normalise.
        assert!(validate_fetch_host("example.com.").is_ok());
    }

    #[test]
    fn host_validation_blocks_localhost_aliases() {
        assert!(validate_fetch_host("localhost").is_err());
        assert!(validate_fetch_host("LOCALHOST").is_err());
        assert!(validate_fetch_host("foo.localhost").is_err());
        assert!(validate_fetch_host("router.local").is_err());
        assert!(validate_fetch_host("svc.internal").is_err());
    }

    #[test]
    fn host_validation_blocks_private_ips() {
        assert!(validate_fetch_host("127.0.0.1").is_err());
        assert!(validate_fetch_host("10.0.0.5").is_err());
        assert!(validate_fetch_host("192.168.1.1").is_err());
        assert!(validate_fetch_host("::1").is_err());
        assert!(validate_fetch_host("fc00::1").is_err());
    }

    #[test]
    fn host_validation_rejects_empty() {
        assert!(validate_fetch_host("").is_err());
        assert!(validate_fetch_host(".").is_err());
    }

    // ── parse_fetch_url ─────────────────────────────────────────────────

    #[test]
    fn parse_url_accepts_http_and_https() {
        assert!(parse_fetch_url("http://example.com/").is_ok());
        assert!(parse_fetch_url("https://example.com/foo?q=1").is_ok());
        // Surrounding whitespace is trimmed.
        assert!(parse_fetch_url("  https://example.com  ").is_ok());
    }

    #[test]
    fn parse_url_rejects_other_schemes() {
        assert!(parse_fetch_url("ftp://example.com/").is_err());
        assert!(parse_fetch_url("file:///etc/passwd").is_err());
        assert!(parse_fetch_url("javascript:alert(1)").is_err());
        assert!(parse_fetch_url("data:text/html,<h1>hi</h1>").is_err());
    }

    #[test]
    fn parse_url_rejects_empty_and_malformed() {
        assert!(parse_fetch_url("").is_err());
        assert!(parse_fetch_url("   ").is_err());
        assert!(parse_fetch_url("not a url").is_err());
    }

    #[test]
    fn parse_url_rejects_blocked_hosts() {
        assert!(parse_fetch_url("http://localhost/").is_err());
        assert!(parse_fetch_url("http://127.0.0.1:11434/").is_err());
        assert!(parse_fetch_url("http://192.168.1.1/").is_err());
        assert!(parse_fetch_url("http://[::1]/").is_err());
    }

    // ── content_type_is_html ────────────────────────────────────────────

    #[test]
    fn content_type_html_variants_are_accepted() {
        assert!(content_type_is_html("text/html"));
        assert!(content_type_is_html("text/html; charset=utf-8"));
        assert!(content_type_is_html("Text/HTML"));
        assert!(content_type_is_html("application/xhtml+xml"));
    }

    #[test]
    fn content_type_non_html_is_rejected() {
        assert!(!content_type_is_html(""));
        assert!(!content_type_is_html("application/json"));
        assert!(!content_type_is_html("application/pdf"));
        assert!(!content_type_is_html("text/plain"));
        assert!(!content_type_is_html("image/png"));
    }

    // ── extract_article ─────────────────────────────────────────────────

    fn sample_article_html() -> String {
        // Long enough body so Readability does not bail with GrabFailed.
        let para = "This is a paragraph of substantive article text used to give Readability enough signal to score the candidate node above the heuristic threshold. ".repeat(8);
        format!(
            "<!doctype html><html><head><title>Test Article</title></head>\
             <body>\
               <header><nav>nav junk</nav></header>\
               <article>\
                 <h1>Test Article</h1>\
                 <p>{para}</p>\
                 <p>{para}</p>\
                 <p>{para}</p>\
               </article>\
               <footer>footer junk</footer>\
             </body></html>"
        )
    }

    #[test]
    fn extract_article_returns_title_and_body() {
        let html = sample_article_html();
        let url = reqwest::Url::parse("https://example.com/article").unwrap();
        let out = extract_article(&html, &url, dom_smoothie::TextMode::Markdown, false)
            .expect("extract should succeed");
        assert!(out.contains("Test Article"));
        assert!(out.contains("Source: https://example.com/article"));
        assert!(out.contains("substantive article text"));
        assert!(!out.contains("nav junk"));
    }

    #[test]
    fn extract_article_truncates_long_output() {
        // Build a body whose extracted text comfortably exceeds the cap
        // (FETCH_RESULT_MAX_BYTES = 50_000). Many short paragraphs each
        // wrapping the same long sentence drives Readability to a single
        // article candidate with a body well above the cap.
        let sentence = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega ".repeat(8);
        let mut paragraphs = String::new();
        for _ in 0..100 {
            paragraphs.push_str(&format!("<p>{sentence}</p>"));
        }
        let big_html = format!(
            "<!doctype html><html><head><title>Big</title></head>\
             <body><article><h1>Big</h1>{paragraphs}</article></body></html>"
        );
        let url = reqwest::Url::parse("https://example.com/big").unwrap();
        let out = extract_article(&big_html, &url, dom_smoothie::TextMode::Markdown, false)
            .expect("extract should succeed");
        assert!(
            out.contains("[truncated:"),
            "expected truncation marker, got body of {} bytes",
            out.len()
        );
        assert!(out.len() <= FETCH_RESULT_MAX_BYTES + 200);
    }

    #[test]
    fn extract_article_appends_body_truncated_note() {
        let html = sample_article_html();
        let url = reqwest::Url::parse("https://example.com/").unwrap();
        let out = extract_article(&html, &url, dom_smoothie::TextMode::Markdown, true)
            .expect("extract should succeed");
        assert!(out.contains("response body exceeded"));
    }

    #[test]
    fn extract_article_handles_garbage_html() {
        // Truly unparseable / non-readable input -> Err. dom_smoothie
        // returns GrabFailed on documents with no article-shaped content.
        let url = reqwest::Url::parse("https://example.com/").unwrap();
        let out = extract_article(
            "not html at all",
            &url,
            dom_smoothie::TextMode::Markdown,
            false,
        );
        // Either the parse fails (Err) or it returns an effectively empty
        // article — both are acceptable; the former is what we expect.
        if let Ok(s) = out {
            // If somehow parse succeeded, the Source line should still be
            // present and there's no panic on missing article body.
            assert!(s.contains("Source: https://example.com"));
        }
    }

    // ── fetch_url dispatch surface ──────────────────────────────────────

    #[tokio::test]
    async fn fetch_url_rejects_invalid_url() {
        let args = json!({ "url": "not a url" });
        let res = fetch_url(&args).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("invalid URL"));
    }

    #[tokio::test]
    async fn fetch_url_rejects_unsupported_scheme() {
        let args = json!({ "url": "file:///etc/passwd" });
        let res = fetch_url(&args).await;
        assert!(res.unwrap_err().contains("unsupported URL scheme"));
    }

    #[tokio::test]
    async fn fetch_url_rejects_private_host() {
        let args = json!({ "url": "http://127.0.0.1:11434/api/tags" });
        let res = fetch_url(&args).await;
        assert!(res.unwrap_err().to_lowercase().contains("private"));
    }

    #[tokio::test]
    async fn fetch_url_rejects_unknown_format() {
        let args = json!({ "url": "https://example.com/", "format": "xml" });
        let res = fetch_url(&args).await;
        assert!(res.unwrap_err().contains("unsupported format"));
    }

    #[tokio::test]
    async fn fetch_url_rejects_empty_args() {
        let args = json!({ "url": "" });
        let res = fetch_url(&args).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error() {
        let res = dispatch("definitely_not_a_real_tool", &json!({})).await;
        assert!(res.starts_with("Error: Unknown tool"));
    }

    #[tokio::test]
    async fn dispatch_fetch_url_routes_through_validation() {
        let res = dispatch("fetch_url", &json!({ "url": "ftp://example.com" })).await;
        assert!(res.starts_with("Error:"));
        assert!(res.contains("unsupported URL scheme"));
    }

    #[test]
    fn is_destructive_marks_every_built_in_correctly() {
        // Read-only locals must auto-approve.
        for name in [
            "read_file",
            "list_dir",
            "glob",
            "grep_content",
            "active_window",
            "list_windows",
            "monitor_info",
            "read_clipboard",
        ] {
            assert!(!is_destructive(name), "{name} should not be destructive");
        }
        // Mutating / network tools must require approval.
        for name in [
            "write_file",
            "delete_file",
            "run_shell",
            "write_clipboard",
            "open_url",
            "launch_app",
            "fetch_url",
        ] {
            assert!(is_destructive(name), "{name} should be destructive");
        }
    }

    #[test]
    fn is_destructive_treats_every_mcp_tool_as_destructive() {
        // Any well-formed `mcp__<server>__<tool>` name is gated.
        assert!(is_destructive("mcp__syvault__get_secret"));
        assert!(is_destructive("mcp__ghostface__deploy"));
        // A name that just shares the prefix without the separator does
        // not count as MCP — falls through to the built-in match and is
        // therefore not destructive (and would surface "Unknown tool" at
        // dispatch time).
        assert!(!is_destructive("mcp__"));
        assert!(!is_destructive("mcp_built_in_lookalike"));
    }

    #[tokio::test]
    async fn dispatch_routes_mcp_prefixed_names_to_registry() {
        // No server is registered under this random name, so dispatch
        // should surface the registry's "not connected" error verbatim.
        let unique = uuid::Uuid::new_v4().to_string();
        let tool_name = format!("mcp__nope-{unique}__whatever");
        let res = dispatch(&tool_name, &json!({})).await;
        assert!(res.starts_with("Error:"));
        assert!(res.contains("not connected"));
    }
}
