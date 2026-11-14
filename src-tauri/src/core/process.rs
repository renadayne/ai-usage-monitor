//! Finds the processes of the monitored tools and the TCP ports they listen on.
//!
//! Never requires admin rights: anything that cannot be read is simply missing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use super::cmd::run_hidden;

const SCAN_CACHE_TTL: Duration = Duration::from_secs(5);
const CHILD_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    ClaudeCode,
    /// The Claude desktop (Electron) app. It hosts Claude Code but is not the CLI itself.
    ClaudeDesktop,
    Codex,
    Antigravity,
    AntigravityLanguageServer,
    /// An editor that can host GitHub Copilot.
    CopilotEditor,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    /// May hold secrets (CSRF tokens). Never log or send it to the UI.
    pub cmdline: Vec<String>,
    pub exe: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedProcess {
    pub kind: ToolKind,
    pub info: ProcessInfo,
}

const COPILOT_EDITORS: &[&str] = &[
    "code.exe",
    "code - insiders.exe",
    "vscodium.exe",
    "devenv.exe",
    "idea64.exe",
    "pycharm64.exe",
    "webstorm64.exe",
    "phpstorm64.exe",
    "rider64.exe",
    "goland64.exe",
    "clion64.exe",
    "rubymine64.exe",
    "datagrip64.exe",
    "rustrover64.exe",
    "studio64.exe",
];

const JS_RUNTIMES: &[&str] = &["node.exe", "node", "bun.exe", "bun"];

/// Cheap pre-filter on the (lower-cased) process name, before command lines are read.
pub fn is_candidate_name(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "claude.exe" | "claude" | "codex.exe" | "codex" | "antigravity.exe"
    ) || name_lower.starts_with("language_server")
        || JS_RUNTIMES.contains(&name_lower)
        || COPILOT_EDITORS.contains(&name_lower)
}

/// Maps a process to the tool it belongs to. Name matching is case-insensitive.
pub fn classify(info: &ProcessInfo) -> Option<ToolKind> {
    let name = info.name.to_ascii_lowercase();
    let exe = info
        .exe
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace('/', "\\");
    let cmd = info
        .cmdline
        .join(" ")
        .to_ascii_lowercase()
        .replace('/', "\\");

    match name.as_str() {
        "claude.exe" | "claude" => {
            let unknown = exe.is_empty() && cmd.is_empty();
            let desktop = exe.contains("\\windowsapps\\claude_")
                || exe.contains("\\anthropicclaude\\")
                || exe.ends_with("\\app\\claude.exe")
                || cmd.contains("--type=");
            if unknown || desktop {
                Some(ToolKind::ClaudeDesktop)
            } else {
                Some(ToolKind::ClaudeCode)
            }
        }
        "codex.exe" | "codex" => Some(ToolKind::Codex),
        "antigravity.exe" => Some(ToolKind::Antigravity),
        _ if name.starts_with("language_server") => (exe.contains("antigravity")
            || cmd.contains("antigravity"))
        .then_some(ToolKind::AntigravityLanguageServer),
        _ if JS_RUNTIMES.contains(&name.as_str()) => {
            if cmd.contains("claude-code") || cmd.contains("@anthropic-ai") {
                Some(ToolKind::ClaudeCode)
            } else if cmd.contains("@openai\\codex") || cmd.contains("\\codex\\bin\\codex.js") {
                Some(ToolKind::Codex)
            } else {
                None
            }
        }
        _ if COPILOT_EDITORS.contains(&name.as_str()) => Some(ToolKind::CopilotEditor),
        _ => None,
    }
}

/// Splits a raw Windows command line into arguments (quotes only, good enough for flags).
pub fn split_command_line(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    for ch in raw.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

/// Parses `Get-CimInstance Win32_Process | Select ProcessId,CommandLine | ConvertTo-Json`.
pub fn parse_cim_json(json: &str) -> HashMap<u32, String> {
    let mut result = HashMap::new();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json.trim()) else {
        return result;
    };
    let items = match value {
        serde_json::Value::Array(items) => items,
        other => vec![other],
    };
    for item in items {
        let pid = item.get("ProcessId").and_then(|v| v.as_u64());
        let cmd = item.get("CommandLine").and_then(|v| v.as_str());
        if let (Some(pid), Some(cmd)) = (pid, cmd) {
            if let Ok(pid) = u32::try_from(pid) {
                result.insert(pid, cmd.to_string());
            }
        }
    }
    result
}

/// Fallback for processes whose command line `sysinfo` could not read.
fn command_lines_via_cim(pids: &[u32]) -> HashMap<u32, String> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let filter = pids
        .iter()
        .map(|pid| format!("ProcessId={pid}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let script = format!(
        "Get-CimInstance Win32_Process -Filter '{filter}' | \
         Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress"
    );
    run_hidden(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
        CHILD_TIMEOUT,
    )
    .map(|output| parse_cim_json(&String::from_utf8_lossy(&output)))
    .unwrap_or_default()
}

#[derive(Default)]
struct ScannerState {
    system: Option<System>,
    cached: Option<(Instant, Arc<Vec<DetectedProcess>>)>,
    /// CIM results per pid, so PowerShell is not started on every scan.
    cim_cache: HashMap<u32, Option<Vec<String>>>,
}

/// Shared, cached process scanner. `scan` blocks; call it from a blocking context.
#[derive(Default)]
pub struct ProcessScanner {
    state: Mutex<ScannerState>,
}

impl ProcessScanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn scan(&self) -> Arc<Vec<DetectedProcess>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, cached)) = &state.cached {
            if at.elapsed() < SCAN_CACHE_TTL {
                return Arc::clone(cached);
            }
        }
        let detected = Arc::new(scan_now(&mut state));
        state.cached = Some((Instant::now(), Arc::clone(&detected)));
        detected
    }

    pub fn find(&self, kind: ToolKind) -> Vec<ProcessInfo> {
        self.scan()
            .iter()
            .filter(|process| process.kind == kind)
            .map(|process| process.info.clone())
            .collect()
    }

    pub fn is_running(&self, kind: ToolKind) -> bool {
        self.scan().iter().any(|process| process.kind == kind)
    }
}

fn scan_now(state: &mut ScannerState) -> Vec<DetectedProcess> {
    if state.system.is_none() {
        state.system = System::new().ok();
    }
    let Some(system) = state.system.as_mut() else {
        return Vec::new();
    };

    // Pass 1: names only (cheap). Pass 2: command line + exe for candidates only.
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let candidates: Vec<sysinfo::Pid> = system
        .processes()
        .iter()
        .filter(|(_, process)| {
            is_candidate_name(&process.name().to_string_lossy().to_ascii_lowercase())
        })
        .map(|(pid, _)| *pid)
        .collect();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&candidates),
        false,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::OnlyIfNotSet),
    );

    let mut infos: Vec<ProcessInfo> = candidates
        .iter()
        .filter_map(|pid| system.process(*pid))
        .map(|process| ProcessInfo {
            pid: process.pid().as_u32(),
            parent_pid: process.parent().map(|pid| pid.as_u32()),
            name: process.name().to_string_lossy().into_owned(),
            cmdline: process
                .cmd()
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            exe: process
                .exe()
                .map(|path| path.to_string_lossy().into_owned()),
            start_time: i64::try_from(process.start_time())
                .ok()
                .filter(|secs| *secs > 0)
                .and_then(|secs| Utc.timestamp_opt(secs, 0).single()),
        })
        .collect();

    // Fallback through CIM for command lines sysinfo could not read.
    state
        .cim_cache
        .retain(|pid, _| infos.iter().any(|info| info.pid == *pid));
    let missing: Vec<u32> = infos
        .iter()
        .filter(|info| info.cmdline.is_empty() && !state.cim_cache.contains_key(&info.pid))
        .map(|info| info.pid)
        .collect();
    if !missing.is_empty() {
        let found = command_lines_via_cim(&missing);
        for pid in missing {
            let args = found.get(&pid).map(|raw| split_command_line(raw));
            state.cim_cache.insert(pid, args);
        }
    }
    for info in &mut infos {
        if info.cmdline.is_empty() {
            if let Some(Some(args)) = state.cim_cache.get(&info.pid) {
                info.cmdline = args.clone();
            }
        }
    }

    let mut detected: Vec<DetectedProcess> = infos
        .into_iter()
        .filter_map(|info| classify(&info).map(|kind| DetectedProcess { kind, info }))
        .collect();
    detected.sort_by_key(|process| process.info.pid);
    detected
}

/// Parses `netstat -ano -p TCP` output into `(pid, port)` pairs of listening sockets.
pub fn parse_netstat(output: &str) -> Vec<(u32, u16)> {
    let mut result = Vec::new();
    for line in output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5
            || !fields[0].eq_ignore_ascii_case("TCP")
            || !fields[3].eq_ignore_ascii_case("LISTENING")
        {
            continue;
        }
        let port = fields[1]
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok());
        let pid = fields[4].parse::<u32>().ok();
        if let (Some(port), Some(pid)) = (port, pid) {
            result.push((pid, port));
        }
    }
    result
}

fn listening_sockets_netstat() -> Vec<(u32, u16)> {
    run_hidden("netstat.exe", &["-ano", "-p", "TCP"], CHILD_TIMEOUT)
        .map(|output| parse_netstat(&String::from_utf8_lossy(&output)))
        .unwrap_or_default()
}

#[cfg(windows)]
mod win32 {
    use std::mem::size_of;
    use std::ptr;

    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
        MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    /// The port is stored in network byte order in the low 16 bits.
    fn port_from_raw(raw: u32) -> u16 {
        u16::from_be((raw & 0xFFFF) as u16)
    }

    /// Returns the raw table as `u32` words (so it is aligned for the row structs).
    fn query_table(family: u32) -> Option<Vec<u32>> {
        let mut size: u32 = 0;
        // SAFETY: a null buffer with size 0 is the documented way to ask for the size.
        unsafe {
            GetExtendedTcpTable(
                ptr::null_mut(),
                &mut size,
                0,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            );
        }
        for _ in 0..4 {
            let words = (size as usize).div_ceil(4) + 64;
            let mut buffer = vec![0u32; words];
            let mut actual = u32::try_from(words * 4).ok()?;
            // SAFETY: `buffer` is writable for `actual` bytes and outlives the call.
            let status = unsafe {
                GetExtendedTcpTable(
                    buffer.as_mut_ptr().cast(),
                    &mut actual,
                    0,
                    family,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            if status == 0 {
                return Some(buffer);
            }
            if status != ERROR_INSUFFICIENT_BUFFER {
                return None;
            }
            size = actual;
        }
        None
    }

    /// Number of rows that are guaranteed to fit inside the buffer.
    fn row_count(buffer: &[u32], row_size: usize) -> usize {
        let Some(declared) = buffer.first() else {
            return 0;
        };
        let capacity = (buffer.len() * 4).saturating_sub(4) / row_size;
        (*declared as usize).min(capacity)
    }

    pub fn listening_sockets() -> Option<Vec<(u32, u16)>> {
        let mut result = Vec::new();

        let v4 = query_table(u32::from(AF_INET))?;
        let count = row_count(&v4, size_of::<MIB_TCPROW_OWNER_PID>());
        // SAFETY: the buffer is 4-byte aligned, starts with a MIB_TCPTABLE_OWNER_PID
        // and `count` rows were checked to lie inside it.
        unsafe {
            let table = v4.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
            let rows = ptr::addr_of!((*table).table).cast::<MIB_TCPROW_OWNER_PID>();
            for index in 0..count {
                let row = rows.add(index).read();
                result.push((row.dwOwningPid, port_from_raw(row.dwLocalPort)));
            }
        }

        if let Some(v6) = query_table(u32::from(AF_INET6)) {
            let count = row_count(&v6, size_of::<MIB_TCP6ROW_OWNER_PID>());
            // SAFETY: same layout argument as above for the IPv6 table.
            unsafe {
                let table = v6.as_ptr().cast::<MIB_TCP6TABLE_OWNER_PID>();
                let rows = ptr::addr_of!((*table).table).cast::<MIB_TCP6ROW_OWNER_PID>();
                for index in 0..count {
                    let row = rows.add(index).read();
                    result.push((row.dwOwningPid, port_from_raw(row.dwLocalPort)));
                }
            }
        }
        Some(result)
    }
}

/// All `(pid, port)` pairs of listening TCP sockets: Win32 API first, `netstat` as fallback.
pub fn listening_sockets() -> Vec<(u32, u16)> {
    #[cfg(windows)]
    {
        if let Some(sockets) = win32::listening_sockets() {
            return sockets;
        }
    }
    listening_sockets_netstat()
}

/// Sorted, de-duplicated TCP ports the given process listens on.
pub fn listening_ports(pid: u32) -> Vec<u16> {
    ports_of(&listening_sockets(), pid)
}

pub fn ports_of(sockets: &[(u32, u16)], pid: u32) -> Vec<u16> {
    let mut ports: Vec<u16> = sockets
        .iter()
        .filter(|(owner, _)| *owner == pid)
        .map(|(_, port)| *port)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(name: &str, exe: &str, cmd: &[&str]) -> ProcessInfo {
        ProcessInfo {
            pid: 1,
            name: name.to_string(),
            exe: (!exe.is_empty()).then(|| exe.to_string()),
            cmdline: cmd.iter().map(|arg| arg.to_string()).collect(),
            ..ProcessInfo::default()
        }
    }

    #[test]
    fn claude_code_native_binary() {
        let info = process(
            "claude.exe",
            r"C:\Users\u\AppData\Local\Claude-Data\x\claude-code\2.1.321\abc\claude.exe",
            &["claude.exe", "--output-format", "stream-json"],
        );
        assert_eq!(classify(&info), Some(ToolKind::ClaudeCode));
        let cli = process(
            "CLAUDE.EXE",
            r"C:\Users\u\.local\bin\claude.exe",
            &["claude"],
        );
        assert_eq!(classify(&cli), Some(ToolKind::ClaudeCode));
    }

    #[test]
    fn claude_desktop_is_not_claude_code() {
        let main = process(
            "Claude.exe",
            r"C:\Program Files\WindowsApps\Claude_5.1.2.0_x64__abc\app\Claude.exe",
            &[r"C:\Program Files\WindowsApps\Claude_5.1.2.0_x64__abc\app\Claude.exe"],
        );
        assert_eq!(classify(&main), Some(ToolKind::ClaudeDesktop));
        let renderer = process("claude.exe", "", &["Claude.exe", "--type=renderer"]);
        assert_eq!(classify(&renderer), Some(ToolKind::ClaudeDesktop));
        let unreadable = process("claude.exe", "", &[]);
        assert_eq!(classify(&unreadable), Some(ToolKind::ClaudeDesktop));
    }

    #[test]
    fn node_hosted_tools() {
        let claude = process(
            "node.exe",
            r"C:\Program Files\nodejs\node.exe",
            &[
                "node",
                r"C:\Users\u\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\cli.js",
            ],
        );
        assert_eq!(classify(&claude), Some(ToolKind::ClaudeCode));
        let codex = process(
            "node.exe",
            "",
            &[
                "node",
                "C:/Users/u/AppData/Roaming/npm/node_modules/@openai/codex/bin/codex.js",
            ],
        );
        assert_eq!(classify(&codex), Some(ToolKind::Codex));
        let other = process("node.exe", "", &["node", "server.js"]);
        assert_eq!(classify(&other), None);
    }

    #[test]
    fn codex_antigravity_and_editors() {
        assert_eq!(
            classify(&process("Codex.exe", "", &[])),
            Some(ToolKind::Codex)
        );
        assert_eq!(
            classify(&process("codex-windows-sandbox-service.exe", "", &[])),
            None
        );
        assert_eq!(
            classify(&process("Antigravity.exe", "", &[])),
            Some(ToolKind::Antigravity)
        );
        let server = process(
            "language_server_windows_x64.exe",
            r"C:\Users\u\AppData\Local\Programs\Antigravity\resources\app\extensions\antigravity\bin\language_server_windows_x64.exe",
            &["language_server_windows_x64.exe", "--csrf_token", "x"],
        );
        assert_eq!(classify(&server), Some(ToolKind::AntigravityLanguageServer));
        let other_server = process(
            "language_server_windows_x64.exe",
            r"C:\Users\u\.codeium\windsurf\language_server_windows_x64.exe",
            &["language_server_windows_x64.exe"],
        );
        assert_eq!(classify(&other_server), None);
        assert_eq!(
            classify(&process("Code - Insiders.exe", "", &[])),
            Some(ToolKind::CopilotEditor)
        );
        assert_eq!(
            classify(&process("idea64.exe", "", &[])),
            Some(ToolKind::CopilotEditor)
        );
        assert_eq!(classify(&process("notepad.exe", "", &[])), None);
    }

    #[test]
    fn candidate_names_cover_every_classified_name() {
        for name in [
            "claude.exe",
            "codex.exe",
            "antigravity.exe",
            "language_server_windows_x64.exe",
            "node.exe",
            "code.exe",
        ] {
            assert!(is_candidate_name(name), "{name}");
        }
        assert!(!is_candidate_name("explorer.exe"));
    }

    #[test]
    fn splits_quoted_command_lines() {
        assert_eq!(
            split_command_line(r#""C:\Program Files\App\app.exe" --flag  "a b" --x=1 """#),
            vec![
                r"C:\Program Files\App\app.exe",
                "--flag",
                "a b",
                "--x=1",
                ""
            ]
        );
        assert!(split_command_line("   ").is_empty());
    }

    #[test]
    fn parses_cim_json_object_and_array() {
        let single = parse_cim_json(r#"{"ProcessId":42,"CommandLine":"a.exe --x"}"#);
        assert_eq!(single.get(&42).map(String::as_str), Some("a.exe --x"));
        let many = parse_cim_json(
            r#"[{"ProcessId":1,"CommandLine":"one"},{"ProcessId":2,"CommandLine":null}]"#,
        );
        assert_eq!(many.len(), 1);
        assert!(parse_cim_json("not json").is_empty());
    }

    #[test]
    fn parses_netstat_listeners() {
        let output = "\r\nActive Connections\r\n\r\n  Proto  Local Address          Foreign Address        State           PID\r\n  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1234\r\n  TCP    127.0.0.1:53421        0.0.0.0:0              LISTENING       777\r\n  TCP    127.0.0.1:53999        127.0.0.1:53421        ESTABLISHED     777\r\n  TCP    [::1]:42100            [::]:0                 LISTENING       777\r\n";
        let sockets = parse_netstat(output);
        assert_eq!(sockets, vec![(1234, 135), (777, 53421), (777, 42100)]);
        assert_eq!(ports_of(&sockets, 777), vec![42100, 53421]);
        assert!(ports_of(&sockets, 1).is_empty());
    }

    #[test]
    fn real_scan_does_not_panic() {
        let scanner = ProcessScanner::new();
        let first = scanner.scan();
        // Second call is served from the cache.
        assert!(Arc::ptr_eq(&first, &scanner.scan()));
        let _ = listening_sockets();
    }
}
