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
/// way. The tool loop pauses on these and asks for explicit user approval
/// before dispatching. Read-only tools dispatch without prompting.
pub fn is_destructive(name: &str) -> bool {
    matches!(
        name,
        "write_file"
            | "delete_file"
            | "run_shell"
            | "write_clipboard"
            | "open_url"
            | "launch_app"
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
pub fn dispatch(name: &str, args: &Value) -> String {
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
        "run_shell" => run_shell(args),
        "write_clipboard" => write_clipboard(args),
        "open_url" => open_url_tool(args),
        "launch_app" => launch_app(args),
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
    for entry in iter {
        if let Ok(path) = entry {
            total += 1;
            if found.len() < GLOB_MAX_RESULTS {
                found.push(path.display().to_string());
            }
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
        return Ok(format!(
            "(no matches for '{}' in '{full}')",
            a.needle
        ));
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

    unsafe extern "system" fn enum_proc(hmon: HMONITOR, _hdc: HDC, _r: *mut RECT, _l: LPARAM) -> BOOL {
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

fn run_shell(args: &Value) -> Result<String, String> {
    let a: ShellArg = parse(args)?;
    if a.command.trim().is_empty() {
        return Err("empty command".to_string());
    }
    let output = if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/C", &a.command])
            .output()
    } else {
        std::process::Command::new("sh")
            .args(["-c", &a.command])
            .output()
    }
    .map_err(|e| format!("spawn: {e}"))?;

    let stdout = trim_to_limit(&String::from_utf8_lossy(&output.stdout), SHELL_OUTPUT_MAX_BYTES);
    let stderr = trim_to_limit(&String::from_utf8_lossy(&output.stderr), SHELL_OUTPUT_MAX_BYTES);
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
        format!("{}\n[truncated: showing first {max} of {} bytes]", &s[..max], s.len())
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
    Ok(format!("Wrote {} characters to the clipboard.", a.text.chars().count()))
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
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len: u32 = buf.len() as u32;
        let res = QueryFullProcessImageNameW(handle, PROCESS_NAME_FORMAT(0), windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        if res.is_err() {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        // Just the file name, not the full path.
        Some(
            path.rsplit(['\\', '/'])
                .next()
                .unwrap_or(&path)
                .to_string(),
        )
    }
}
