//! Antigravity: quota from the language server the IDE runs on localhost.
//!
//! Only works while Antigravity is open. When it is closed the last snapshot
//! stays on screen, marked stale.
//!
//! Security: the CSRF token is read from the process command line, kept in
//! memory and sent to `127.0.0.1` only. Certificate checks are disabled on a
//! dedicated client that can talk to nothing but loopback.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{parse_time, ProviderContext};
use crate::engine::http::{self, HttpResponse};
use crate::engine::model::{Activity, ProviderId, QuotaWindow};
use crate::engine::process::{listening_ports, ProcessInfo, ToolKind};
use crate::engine::provider::{ActivityInfo, Provider, ProviderError, QuotaInfo};

const LOOPBACK: &str = "127.0.0.1";
const SERVICE: &str = "exa.language_server_pb.LanguageServerService";
const METHOD_SUMMARY: &str = "RetrieveUserQuotaSummary";
const METHOD_STATUS: &str = "GetUserStatus";
const METHOD_CONFIGS: &str = "GetCommandModelConfigs";
const POLL_INTERVAL: Duration = Duration::from_secs(60);
const ACTIVE_WINDOW_SECS: i64 = 5 * 60;
const NOTE_CLOSED: &str = "Antigravity is not running. Showing the last known numbers.";
const REQUEST_BODY: &str =
    r#"{"metadata":{"ideName":"antigravity","extensionName":"antigravity","locale":"en"}}"#;

/// Usual install locations, used to tell "closed" from "not installed".
fn install_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for (variable, tail) in [
        ("LOCALAPPDATA", "Programs/Antigravity/Antigravity.exe"),
        ("ProgramFiles", "Antigravity/Antigravity.exe"),
        ("USERPROFILE", ".gemini/antigravity"),
    ] {
        if let Some(base) = std::env::var_os(variable) {
            candidates.push(PathBuf::from(base).join(tail));
        }
    }
    candidates
}

/// True when any of the given paths exists.
pub fn any_exists(paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| path.exists())
}

/// Value of a command line flag; accepts `--flag VALUE` and `--flag=VALUE`.
pub fn flag_value(args: &[String], name: &str) -> Option<String> {
    let exact = format!("--{name}");
    let prefix = format!("--{name}=");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if *arg == exact {
            return iter
                .next()
                .filter(|value| !value.starts_with("--"))
                .cloned();
        }
        if let Some(value) = arg.strip_prefix(&prefix) {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// What is needed to talk to one language server.
/// Deliberately not `Debug`: it holds the CSRF token.
pub struct ServerTarget {
    pub pid: u32,
    pub csrf_token: String,
    /// Ports named on the command line, tried before the listening ports.
    pub hinted_ports: Vec<u16>,
}

pub fn server_target(process: &ProcessInfo) -> Option<ServerTarget> {
    let csrf_token = flag_value(&process.cmdline, "csrf_token").filter(|t| !t.is_empty())?;
    let hinted_ports = ["https_server_port", "http_server_port", "server_port"]
        .iter()
        .filter_map(|name| flag_value(&process.cmdline, name))
        .filter_map(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .collect();
    Some(ServerTarget {
        pid: process.pid,
        csrf_token,
        hinted_ports,
    })
}

/// Hinted ports first, then everything the process listens on, without duplicates.
pub fn candidate_ports(hinted: &[u16], listening: &[u16]) -> Vec<u16> {
    let mut ports = Vec::new();
    for port in hinted.iter().chain(listening) {
        if !ports.contains(port) {
            ports.push(*port);
        }
    }
    ports
}

/// The only place URLs are built: the host is always loopback.
pub fn loopback_url(scheme: &str, port: u16, method: &str) -> String {
    format!("{scheme}://{LOOPBACK}:{port}/{SERVICE}/{method}")
}

/// Guard used right before sending: true only for `http(s)://127.0.0.1:<port>/...`.
pub fn is_loopback_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    match authority.split_once(':') {
        Some((host, port)) => host == LOOPBACK && port.parse::<u16>().is_ok(),
        None => false,
    }
}

/// Client for the local language server only (self-signed certificate).
fn loopback_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(http::USER_AGENT)
        .danger_accept_invalid_certs(true)
        .no_proxy()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(6))
        .build()
        .unwrap_or_default()
}

/// `remainingFraction` is omitted when it is 0 (protobuf default), so a quota
/// object that has a reset time but no fraction means "nothing left".
fn used_percent(quota: &Value) -> Option<f32> {
    let fraction = match quota.get("remainingFraction").and_then(Value::as_f64) {
        Some(fraction) => fraction,
        None if quota.get("resetTime").is_some() => 0.0,
        None => return None,
    };
    Some((((1.0 - fraction) * 100.0) as f32).clamp(0.0, 100.0))
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

fn window_label(window: Option<&str>, fallback: Option<&str>) -> String {
    match window {
        Some("5h") => "5 hours".to_string(),
        Some("weekly") => "Weekly".to_string(),
        Some("daily") => "Daily".to_string(),
        Some("monthly") => "Monthly".to_string(),
        other => fallback.or(other).unwrap_or("Quota").to_string(),
    }
}

/// `RetrieveUserQuotaSummary`: quota pools shared by groups of models.
pub fn parse_quota_summary(body: &str) -> Option<Vec<QuotaWindow>> {
    let value: Value = serde_json::from_str(body).ok()?;
    let groups = value
        .get("response")
        .unwrap_or(&value)
        .get("groups")?
        .as_array()?;
    let mut windows = Vec::new();
    for group in groups {
        let group_name = group
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or("Models");
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };
        for bucket in buckets {
            let Some(used) = used_percent(bucket) else {
                continue;
            };
            let window = bucket.get("window").and_then(Value::as_str);
            let label = window_label(window, bucket.get("displayName").and_then(Value::as_str));
            let id = format!(
                "pool:{}:{}",
                slug(group_name),
                window.map(slug).unwrap_or_else(|| slug(&label))
            );
            windows.push(
                QuotaWindow::percent(&id, &format!("{group_name} · {label}"), used)
                    .with_reset(bucket.get("resetTime").and_then(parse_time)),
            );
        }
    }
    Some(windows)
}

/// "Gemini 4 Argon (High)" -> "Gemini 4 Argon"
fn base_model_name(label: &str) -> &str {
    match label.rfind(" (") {
        Some(index) if label.ends_with(')') => &label[..index],
        _ => label,
    }
}

/// Per-model quota list (`clientModelConfigs`). Effort variants of one model
/// that share the same quota collapse into a single window.
fn windows_from_model_configs(configs: &[Value]) -> Vec<QuotaWindow> {
    let mut windows: Vec<QuotaWindow> = Vec::new();
    for config in configs {
        let Some(label) = config.get("label").and_then(Value::as_str) else {
            continue;
        };
        let Some(quota) = config.get("quotaInfo") else {
            continue;
        };
        let Some(used) = used_percent(quota) else {
            continue;
        };
        let resets_at = quota.get("resetTime").and_then(parse_time);
        let base = base_model_name(label);
        let duplicate = windows.iter().any(|window| {
            window.label == base
                && window.used_percent == Some(used)
                && window.resets_at == resets_at
        });
        if duplicate {
            continue;
        }
        let name = if windows.iter().any(|window| window.label == base) {
            label
        } else {
            base
        };
        windows.push(
            QuotaWindow::percent(&format!("model:{}", slug(name)), name, used)
                .with_reset(resets_at),
        );
    }
    windows
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct UserStatus {
    pub plan: Option<String>,
    pub windows: Vec<QuotaWindow>,
}

/// `GetUserStatus`: plan plus per-model quota.
pub fn parse_user_status(body: &str) -> Option<UserStatus> {
    let value: Value = serde_json::from_str(body).ok()?;
    let status = value.get("userStatus")?;
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty() && *text != "[redacted]")
            .map(str::to_string)
    };
    let plan = text(
        status
            .get("userTier")
            .and_then(|tier| tier.get("description")),
    )
    .or_else(|| {
        text(
            status
                .get("planStatus")
                .and_then(|plan| plan.get("planInfo"))
                .and_then(|info| info.get("planName")),
        )
    });
    let windows = status
        .get("cascadeModelConfigData")
        .and_then(|data| data.get("clientModelConfigs"))
        .and_then(Value::as_array)
        .map(|configs| windows_from_model_configs(configs))
        .unwrap_or_default();
    Some(UserStatus { plan, windows })
}

/// `GetCommandModelConfigs`: same per-model list, at the top level.
pub fn parse_command_model_configs(body: &str) -> Option<Vec<QuotaWindow>> {
    let value: Value = serde_json::from_str(body).ok()?;
    let configs = value
        .get("clientModelConfigs")
        .or_else(|| value.get("commandModelConfigs"))
        .or_else(|| value.get("modelConfigs"))?
        .as_array()?;
    Some(windows_from_model_configs(configs))
}

fn signature(windows: &[QuotaWindow]) -> String {
    windows
        .iter()
        .map(|window| format!("{}:{:?}", window.id, window.used_percent))
        .collect::<Vec<_>>()
        .join("|")
}

#[derive(Default)]
struct State {
    /// `(pid, base scheme, port)` of the last endpoint that answered.
    endpoint: Option<(u32, &'static str, u16)>,
    signature: Option<String>,
    last_change_at: Option<DateTime<Utc>>,
}

impl State {
    fn observe(&mut self, signature: String, now: DateTime<Utc>) {
        if self.signature.as_ref().is_some_and(|old| *old != signature) {
            self.last_change_at = Some(now);
        }
        self.signature = Some(signature);
    }
}

pub struct AntigravityProvider {
    ctx: ProviderContext,
    client: reqwest::Client,
    state: Mutex<State>,
}

impl AntigravityProvider {
    pub fn new(ctx: ProviderContext) -> Self {
        Self {
            ctx,
            client: loopback_client(),
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn call(
        &self,
        scheme: &str,
        port: u16,
        method: &str,
        csrf_token: &str,
    ) -> Result<HttpResponse, ProviderError> {
        let url = loopback_url(scheme, port, method);
        if !is_loopback_url(&url) {
            return Err(ProviderError::Other(
                "refusing to call a non-loopback address".to_string(),
            ));
        }
        let request = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Connect-Protocol-Version", "1")
            .header("X-Codeium-Csrf-Token", csrf_token)
            .body(REQUEST_BODY);
        http::send(request).await
    }

    /// Finds the Connect endpoint of one server: the cached one, else a probe
    /// over the candidate ports (HTTPS first, then plain HTTP).
    async fn find_endpoint(&self, target: &ServerTarget) -> Option<(&'static str, u16)> {
        let cached = self.state().endpoint;
        if let Some((pid, scheme, port)) = cached {
            if pid == target.pid {
                return Some((scheme, port));
            }
        }
        let pid = target.pid;
        let listening = tokio::task::block_in_place(|| listening_ports(pid));
        for port in candidate_ports(&target.hinted_ports, &listening) {
            for scheme in ["https", "http"] {
                let answered = self
                    .call(scheme, port, METHOD_SUMMARY, &target.csrf_token)
                    .await
                    // Any HTTP answer from the service (even "unimplemented")
                    // identifies the Connect port; other ports fail to connect
                    // or answer 404.
                    .is_ok_and(|response| response.is_success() || response.status == 501);
                if answered {
                    self.state().endpoint = Some((pid, scheme, port));
                    return Some((scheme, port));
                }
            }
        }
        None
    }

    /// Tries the three methods in order and merges what they give.
    async fn query(&self, target: &ServerTarget) -> Result<QuotaInfo, ProviderError> {
        let Some((scheme, port)) = self.find_endpoint(target).await else {
            return Err(ProviderError::Other(
                "Antigravity's local server did not answer".to_string(),
            ));
        };
        let token = &target.csrf_token;

        let summary = match self.call(scheme, port, METHOD_SUMMARY, token).await {
            Ok(response) if response.is_success() => parse_quota_summary(&response.body),
            Ok(_) => None,
            Err(error) => {
                // The server went away: forget the endpoint and report it.
                self.state().endpoint = None;
                return Err(error);
            }
        }
        .filter(|windows| !windows.is_empty());

        let status = match self.call(scheme, port, METHOD_STATUS, token).await {
            Ok(response) if response.is_success() => parse_user_status(&response.body),
            Ok(response) if matches!(response.status, 401 | 403) => {
                return Err(ProviderError::AuthRequired(
                    "Sign in to Antigravity to see usage.".to_string(),
                ));
            }
            _ => None,
        }
        .unwrap_or_default();

        let mut windows = summary.unwrap_or(status.windows);
        if windows.is_empty() {
            windows = match self.call(scheme, port, METHOD_CONFIGS, token).await {
                Ok(response) if response.is_success() => {
                    parse_command_model_configs(&response.body).unwrap_or_default()
                }
                _ => Vec::new(),
            };
        }
        if windows.is_empty() && status.plan.is_none() {
            return Err(ProviderError::Other(
                "Antigravity returned no quota information".to_string(),
            ));
        }
        Ok(QuotaInfo {
            plan: status.plan,
            windows,
            ..QuotaInfo::default()
        })
    }
}

#[async_trait]
impl Provider for AntigravityProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Antigravity
    }

    fn poll_interval(&self) -> Duration {
        POLL_INTERVAL
    }

    async fn detect_activity(&self) -> ActivityInfo {
        let running = tokio::task::block_in_place(|| {
            self.ctx.processes.is_running(ToolKind::Antigravity)
                || self
                    .ctx
                    .processes
                    .is_running(ToolKind::AntigravityLanguageServer)
        });
        let state = self.state();
        let recent = state
            .last_change_at
            .is_some_and(|at| (Utc::now() - at).num_seconds() <= ACTIVE_WINDOW_SECS);
        let activity = match (running, recent) {
            (false, _) => Activity::NotRunning,
            (true, true) => Activity::Active,
            (true, false) => Activity::Idle,
        };
        ActivityInfo {
            activity,
            // The server does not say which model is in use.
            current_model: None,
            last_activity_at: state.last_change_at,
        }
    }

    async fn fetch_quota(&self) -> Result<QuotaInfo, ProviderError> {
        let servers = tokio::task::block_in_place(|| {
            self.ctx.processes.find(ToolKind::AntigravityLanguageServer)
        });
        let targets: Vec<ServerTarget> = servers.iter().filter_map(server_target).collect();
        if targets.is_empty() {
            self.state().endpoint = None;
            let app_running = tokio::task::block_in_place(|| {
                self.ctx.processes.is_running(ToolKind::Antigravity)
            });
            if !app_running && !any_exists(&install_candidates()) {
                return Err(ProviderError::NotInstalled);
            }
            return Ok(QuotaInfo {
                stale: true,
                note: Some(NOTE_CLOSED.to_string()),
                ..QuotaInfo::default()
            });
        }

        // Several windows mean several servers; the first that answers wins.
        let mut last_error = None;
        for target in &targets {
            match self.query(target).await {
                Ok(info) => {
                    self.state().observe(signature(&info.windows), Utc::now());
                    return Ok(info);
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            ProviderError::Other("Antigravity's local server did not answer".to_string())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const SUMMARY: &str =
        include_str!("../../../fixtures/antigravity/RetrieveUserQuotaSummary.json");
    const STATUS: &str = include_str!("../../../fixtures/antigravity/GetUserStatus.json");

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn flags_in_both_forms() {
        let spaced = args(&[
            "server.exe",
            "--csrf_token",
            "abc",
            "--https_server_port",
            "56589",
        ]);
        assert_eq!(flag_value(&spaced, "csrf_token").as_deref(), Some("abc"));
        assert_eq!(
            flag_value(&spaced, "https_server_port").as_deref(),
            Some("56589")
        );

        let equals = args(&[
            "server.exe",
            "--csrf_token=abc",
            "--extension_server_port=\"4242\"",
        ]);
        assert_eq!(flag_value(&equals, "csrf_token").as_deref(), Some("abc"));
        assert_eq!(
            flag_value(&equals, "extension_server_port").as_deref(),
            Some("4242")
        );

        // A flag without a value, a missing flag and a look-alike prefix.
        let odd = args(&["--csrf_token", "--standalone", "--csrf_token_extra=zzz"]);
        assert_eq!(flag_value(&odd, "csrf_token"), None);
        assert_eq!(flag_value(&odd, "missing"), None);
    }

    #[test]
    fn target_from_process() {
        let process = ProcessInfo {
            pid: 77,
            cmdline: args(&[
                "language_server.exe",
                "--standalone",
                "--https_server_port=56589",
                "--csrf_token",
                "secret",
            ]),
            ..ProcessInfo::default()
        };
        let target = server_target(&process).unwrap();
        assert_eq!(target.pid, 77);
        assert_eq!(target.csrf_token, "secret");
        assert_eq!(target.hinted_ports, vec![56589]);

        let without_token = ProcessInfo {
            cmdline: args(&["language_server.exe", "--standalone"]),
            ..ProcessInfo::default()
        };
        assert!(server_target(&without_token).is_none());
    }

    #[test]
    fn port_candidates_keep_hints_first() {
        assert_eq!(
            candidate_ports(&[56589], &[56590, 56589, 56591]),
            vec![56589, 56590, 56591]
        );
        assert!(candidate_ports(&[], &[]).is_empty());
    }

    #[test]
    fn urls_are_always_loopback() {
        let url = loopback_url("https", 56589, METHOD_SUMMARY);
        assert_eq!(
            url,
            "https://127.0.0.1:56589/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary"
        );
        assert!(is_loopback_url(&url));
        assert!(is_loopback_url("http://127.0.0.1:1/x"));
        for bad in [
            "https://example.com:443/x",
            "https://127.0.0.1.evil.example:443/x",
            "https://127.0.0.1@evil.example:443/x",
            "https://localhost:443/x",
            "https://127.0.0.1/x",
            "ftp://127.0.0.1:21/x",
            "https://10.0.0.5:443/x",
        ] {
            assert!(!is_loopback_url(bad), "{bad}");
        }
    }

    #[test]
    fn quota_summary_fixture() {
        let windows = parse_quota_summary(SUMMARY).unwrap();
        let ids: Vec<&str> = windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "pool:gemini-models:weekly",
                "pool:gemini-models:5h",
                "pool:claude-and-gpt-models:weekly",
                "pool:claude-and-gpt-models:5h",
                "pool:gemini-4-argon:weekly",
            ]
        );
        assert_eq!(windows[0].label, "Gemini Models · Weekly");
        assert_eq!(windows[1].label, "Gemini Models · 5 hours");
        assert_eq!(windows[0].used_percent, Some(0.0));
        assert_eq!(
            windows[1].resets_at,
            Some(Utc.with_ymd_and_hms(2026, 11, 14, 21, 54, 50).unwrap())
        );
    }

    #[test]
    fn missing_fraction_means_exhausted() {
        let windows = parse_quota_summary(
            r#"{"response":{"groups":[{"displayName":"G","buckets":[
                 {"window":"5h","resetTime":"2026-11-14T21:54:50Z"},
                 {"window":"weekly","remainingFraction":0.25},
                 {"displayName":"Odd Bucket","remainingFraction":2},
                 {}]},{"displayName":"Empty"}]}}"#,
        )
        .unwrap();
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].used_percent, Some(100.0));
        assert_eq!(windows[1].used_percent, Some(75.0));
        assert_eq!(windows[2].id, "pool:g:odd-bucket");
        assert_eq!(windows[2].used_percent, Some(0.0));

        assert!(parse_quota_summary("{}").is_none());
        assert!(parse_quota_summary("nope").is_none());
    }

    #[test]
    fn user_status_fixture() {
        let status = parse_user_status(STATUS).unwrap();
        assert_eq!(status.plan.as_deref(), Some("Google AI Pro"));
        let labels: Vec<&str> = status.windows.iter().map(|w| w.label.as_str()).collect();
        // Effort variants collapse; the model without quota info is skipped.
        assert_eq!(
            labels,
            vec![
                "Gemini 4 Argon",
                "Gemini 4 Hydrogen",
                "Gemini 3.8 Flash",
                "Claude Opus 5.5",
                "Claude Sonnet 5.5",
                "GPT-OSS 120B",
            ]
        );
        assert_eq!(status.windows[0].id, "model:gemini-4-argon");
        assert_eq!(status.windows[0].used_percent, Some(0.0));
        assert!(status.windows[0].resets_at.is_some());
    }

    #[test]
    fn variants_with_different_quota_stay_separate() {
        let windows = parse_command_model_configs(
            r#"{"clientModelConfigs":[
                 {"label":"Model X (Low)","quotaInfo":{"remainingFraction":0.5}},
                 {"label":"Model X (High)","quotaInfo":{"remainingFraction":0.1}},
                 {"label":"Model X (Medium)","quotaInfo":{"remainingFraction":0.5}},
                 {"label":"No Quota","quotaInfo":{}},
                 {"quotaInfo":{"remainingFraction":1}}]}"#,
        )
        .unwrap();
        let labels: Vec<&str> = windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, vec!["Model X", "Model X (High)"]);
        assert_eq!(windows[1].used_percent, Some(90.0));
        assert!(parse_command_model_configs(r#"{"code":"unimplemented"}"#).is_none());
        assert!(parse_user_status(r#"{"other":1}"#).is_none());
    }

    #[test]
    fn install_detection() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.exe");
        assert!(!any_exists(std::slice::from_ref(&missing)));
        assert!(any_exists(&[missing, dir.path().to_path_buf()]));
        assert!(!any_exists(&[]));
    }

    #[test]
    fn quota_change_marks_activity() {
        let now = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let mut state = State::default();
        state.observe("a".into(), now);
        assert_eq!(state.last_change_at, None);
        state.observe("b".into(), now);
        assert_eq!(state.last_change_at, Some(now));
    }
}
