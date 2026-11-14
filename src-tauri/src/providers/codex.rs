//! Codex: rate limits from the local rollout logs and from the ChatGPT usage
//! endpoint, whichever is newer. Read-only; tokens are never refreshed here.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ruzstd::decoding::StreamingDecoder;
use serde_json::Value;

use super::{activity_from, parse_time, to_utc, ProviderContext};
use crate::engine::http::{self, HttpResponse};
use crate::engine::model::{Activity, ProviderId, QuotaWindow};
use crate::engine::paths::Tool;
use crate::engine::process::ToolKind;
use crate::engine::provider::{ActivityInfo, Provider, ProviderError, QuotaInfo};
use crate::engine::tail;

const DEFAULT_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// The local logs are re-read on this cadence...
const POLL_INTERVAL: Duration = Duration::from_secs(60);
/// ...the network only on this one.
const LIVE_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Compressed rollouts above this size are not decoded (too slow for a tray app).
const MAX_ZSTD_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
const ROLLOUT_SUFFIXES: [&str; 2] = [".jsonl", ".jsonl.zst"];

const HINT_LOGIN: &str = "Sign in to Codex (`codex login`) to see usage.";
const HINT_REJECTED: &str = "ChatGPT rejected the saved Codex login. Open Codex to refresh it.";
const NOTE_API_KEY: &str = "API key mode: usage is billed per token, there are no quota windows.";

#[derive(Debug, Clone, PartialEq)]
pub struct RateWindow {
    pub used_percent: f64,
    pub window_seconds: Option<i64>,
    pub resets_at: Option<DateTime<Utc>>,
}

/// Rate limits as reported by one source at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitsSnapshot {
    pub at: DateTime<Utc>,
    pub plan: Option<String>,
    pub primary: Option<RateWindow>,
    pub secondary: Option<RateWindow>,
}

fn rollout_window(value: &Value, at: DateTime<Utc>) -> Option<RateWindow> {
    let used_percent = value.get("used_percent")?.as_f64()?;
    let resets_at = value.get("resets_at").and_then(parse_time).or_else(|| {
        // Older rollouts carry a relative value instead.
        value
            .get("resets_in_seconds")
            .and_then(Value::as_i64)
            .map(|seconds| at + chrono::Duration::seconds(seconds))
    });
    Some(RateWindow {
        used_percent,
        window_seconds: value
            .get("window_minutes")
            .and_then(Value::as_i64)
            .map(|minutes| minutes * 60),
        resets_at,
    })
}

/// Parses a rollout line if it is a `token_count` event that carries rate limits.
pub fn parse_token_count_line(line: &str) -> Option<LimitsSnapshot> {
    if !line.contains("\"token_count\"") {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "event_msg" {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("type")?.as_str()? != "token_count" {
        return None;
    }
    let limits = payload
        .get("rate_limits")
        .filter(|limits| limits.is_object())?;
    let at = value.get("timestamp").and_then(parse_time)?;
    let primary = limits.get("primary").and_then(|w| rollout_window(w, at));
    let secondary = limits.get("secondary").and_then(|w| rollout_window(w, at));
    if primary.is_none() && secondary.is_none() {
        return None;
    }
    Some(LimitsSnapshot {
        at,
        plan: limits
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        primary,
        secondary,
    })
}

/// Parses a rollout line if it is a `turn_context` and returns its model.
pub fn parse_turn_context_line(line: &str) -> Option<String> {
    if !line.contains("\"turn_context\"") {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "turn_context" {
        return None;
    }
    let model = value.get("payload")?.get("model")?.as_str()?;
    (!model.is_empty()).then(|| model.to_string())
}

fn live_window(value: &Value, now: DateTime<Utc>) -> Option<RateWindow> {
    let used_percent = value.get("used_percent")?.as_f64()?;
    let resets_at = value.get("reset_at").and_then(parse_time).or_else(|| {
        value
            .get("reset_after_seconds")
            .and_then(Value::as_i64)
            .map(|seconds| now + chrono::Duration::seconds(seconds))
    });
    Some(RateWindow {
        used_percent,
        window_seconds: value.get("limit_window_seconds").and_then(Value::as_i64),
        resets_at,
    })
}

/// Parses the body of the ChatGPT usage endpoint.
pub fn parse_live_usage(body: &str, now: DateTime<Utc>) -> Result<LimitsSnapshot, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::Other("usage response is not valid JSON".to_string()))?;
    if !value.is_object() {
        return Err(ProviderError::Other(
            "usage response has an unexpected shape".to_string(),
        ));
    }
    let limit = value.get("rate_limit");
    let window = |name: &str| {
        limit
            .and_then(|limit| limit.get(name))
            .and_then(|window| live_window(window, now))
    };
    Ok(LimitsSnapshot {
        at: now,
        plan: value
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        primary: window("primary_window"),
        secondary: window("secondary_window"),
    })
}

pub fn interpret_live_response(
    response: &HttpResponse,
    now: DateTime<Utc>,
) -> Result<LimitsSnapshot, ProviderError> {
    http::check_status(response, HINT_REJECTED)?;
    parse_live_usage(&response.body, now)
}

/// Merge rule: the source with the newer timestamp wins.
pub fn merge(
    local: Option<LimitsSnapshot>,
    live: Option<LimitsSnapshot>,
) -> Option<LimitsSnapshot> {
    match (local, live) {
        (Some(local), Some(live)) => Some(if local.at > live.at { local } else { live }),
        (local, live) => local.or(live),
    }
}

fn window_identity(seconds: Option<i64>, primary: bool) -> (String, String) {
    match seconds {
        Some(18_000) => ("five_hour".to_string(), "5 hours".to_string()),
        Some(604_800) => ("weekly".to_string(), "Weekly".to_string()),
        Some(seconds) if seconds > 0 => {
            let id = if primary { "primary" } else { "secondary" };
            let label = if seconds % 86_400 == 0 {
                format!("{} days", seconds / 86_400)
            } else if seconds % 3_600 == 0 {
                format!("{} hours", seconds / 3_600)
            } else {
                format!("{} minutes", seconds / 60)
            };
            (id.to_string(), label)
        }
        _ if primary => ("five_hour".to_string(), "5 hours".to_string()),
        _ => ("weekly".to_string(), "Weekly".to_string()),
    }
}

/// Converts to UI windows. A window whose reset time has passed without newer
/// data is shown as 0 % and flagged as an estimate.
pub fn to_windows(snapshot: &LimitsSnapshot, now: DateTime<Utc>) -> Vec<QuotaWindow> {
    let mut windows = Vec::new();
    for (window, primary) in [(&snapshot.primary, true), (&snapshot.secondary, false)] {
        let Some(window) = window else {
            continue;
        };
        let (id, label) = window_identity(window.window_seconds, primary);
        let mut quota = QuotaWindow::percent(&id, &label, window.used_percent as f32);
        match window.resets_at {
            Some(resets_at) if resets_at <= now => {
                quota.used_percent = Some(0.0);
                quota.estimated = true;
            }
            resets_at => quota.resets_at = resets_at,
        }
        windows.push(quota);
    }
    windows
}

/// Deliberately not `Debug`: the ChatGPT variant holds a secret.
pub enum AuthMode {
    ChatGpt {
        access_token: String,
        account_id: Option<String>,
    },
    ApiKey,
    Missing,
}

pub fn parse_auth(json: &str) -> AuthMode {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return AuthMode::Missing;
    };
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let tokens = value.get("tokens");
    if let Some(access_token) = text(tokens.and_then(|t| t.get("access_token"))) {
        return AuthMode::ChatGpt {
            access_token,
            account_id: text(tokens.and_then(|t| t.get("account_id"))),
        };
    }
    if text(value.get("OPENAI_API_KEY")).is_some() {
        AuthMode::ApiKey
    } else {
        AuthMode::Missing
    }
}

/// Value of a top-level `key = "value"` entry of a TOML file (before any `[table]`).
pub fn toml_top_level_string(toml: &str, key: &str) -> Option<String> {
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        let value = value.trim();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let rest = &value[1..];
        return rest.find(quote).map(|end| rest[..end].to_string());
    }
    None
}

/// Usage URL for the configured `chatgpt_base_url`. `None` means the configured
/// host is not an OpenAI one, so the token must not be sent there.
pub fn usage_url(config_toml: Option<&str>) -> Option<String> {
    let Some(base) = config_toml.and_then(|toml| toml_top_level_string(toml, "chatgpt_base_url"))
    else {
        return Some(DEFAULT_USAGE_URL.to_string());
    };
    let base = base.trim_end_matches('/');
    let host = base
        .strip_prefix("https://")?
        .split(['/', ':', '?', '#'])
        .next()?
        .to_ascii_lowercase();
    let trusted = ["chatgpt.com", "openai.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    if !trusted {
        return None;
    }
    if base.to_ascii_lowercase().ends_with("/backend-api") {
        Some(format!("{base}/wham/usage"))
    } else {
        Some(format!("{base}/api/codex/usage"))
    }
}

/// Calls `on_line` for every line of a (possibly multi-frame) zstd file.
pub fn for_each_zstd_line(path: &Path, mut on_line: impl FnMut(&str)) -> io::Result<()> {
    let mut source = BufReader::new(File::open(path)?);
    let mut carry: Vec<u8> = Vec::new();
    let mut skipping = false;
    let mut total: u64 = 0;
    let mut chunk = vec![0u8; 64 * 1024];

    while !source.fill_buf()?.is_empty() {
        let mut decoder =
            StreamingDecoder::new(source).map_err(|error| io::Error::other(error.to_string()))?;
        loop {
            let read = decoder.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            total += read as u64;
            if total > MAX_DECODED_BYTES {
                return Ok(());
            }
            for piece in chunk[..read].split_inclusive(|byte| *byte == b'\n') {
                let complete = piece.ends_with(b"\n");
                if !skipping {
                    carry.extend_from_slice(piece);
                }
                if complete {
                    if !skipping {
                        on_line(String::from_utf8_lossy(&carry).trim_end());
                    }
                    carry.clear();
                    skipping = false;
                } else if carry.len() > MAX_LINE_BYTES {
                    // Absurdly long line: drop it and resume at the next newline.
                    carry.clear();
                    skipping = true;
                }
            }
        }
        source = decoder.into_inner();
    }
    if !skipping && !carry.is_empty() {
        on_line(String::from_utf8_lossy(&carry).trim_end());
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RolloutData {
    pub limits: Option<LimitsSnapshot>,
    pub model: Option<String>,
}

/// Reads the last rate limits and model from a rollout file (plain or zstd).
pub fn read_rollout(path: &Path) -> RolloutData {
    let is_zstd = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zst"));
    if !is_zstd {
        return RolloutData {
            limits: tail::find_last(path, parse_token_count_line),
            model: tail::find_last(path, parse_turn_context_line),
        };
    }
    let mut data = RolloutData::default();
    let small_enough = std::fs::metadata(path).is_ok_and(|meta| meta.len() <= MAX_ZSTD_BYTES);
    if small_enough {
        // A truncated file still yields everything decoded before the error.
        let _ = for_each_zstd_line(path, |line| {
            if let Some(limits) = parse_token_count_line(line) {
                data.limits = Some(limits);
            } else if let Some(model) = parse_turn_context_line(line) {
                data.model = Some(model);
            }
        });
    }
    data
}

struct RolloutCache {
    path: PathBuf,
    modified: SystemTime,
    data: RolloutData,
}

#[derive(Default)]
struct State {
    rollout: Option<RolloutCache>,
    live: Option<LimitsSnapshot>,
    next_live_at: Option<Instant>,
}

struct LocalRead {
    auth: AuthMode,
    usage_url: Option<String>,
    config_model: Option<String>,
    rollout: RolloutData,
}

pub struct CodexProvider {
    ctx: ProviderContext,
    state: Mutex<State>,
}

impl CodexProvider {
    pub fn new(ctx: ProviderContext) -> Self {
        Self {
            ctx,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn newest_rollout(dir: &Path) -> Option<(PathBuf, SystemTime)> {
        tail::newest_file(&dir.join("sessions"), &ROLLOUT_SUFFIXES, 4)
    }

    /// Rollout data, re-read only when the newest file changed.
    fn rollout_data(&self, dir: &Path) -> RolloutData {
        let Some((path, modified)) = Self::newest_rollout(dir) else {
            return RolloutData::default();
        };
        if let Some(cache) = &self.state().rollout {
            if cache.path == path && cache.modified == modified {
                return cache.data.clone();
            }
        }
        let data = read_rollout(&path);
        self.state().rollout = Some(RolloutCache {
            path,
            modified,
            data: data.clone(),
        });
        data
    }

    fn read_local(&self) -> Result<LocalRead, ProviderError> {
        let resolved = self
            .ctx
            .paths
            .resolve(Tool::Codex)
            .ok_or(ProviderError::NotInstalled)?;
        let dir = resolved.path;
        let config = std::fs::read_to_string(dir.join("config.toml")).ok();
        Ok(LocalRead {
            auth: std::fs::read_to_string(dir.join("auth.json"))
                .map(|json| parse_auth(&json))
                .unwrap_or(AuthMode::Missing),
            usage_url: usage_url(config.as_deref()),
            config_model: config
                .as_deref()
                .and_then(|toml| toml_top_level_string(toml, "model")),
            rollout: self.rollout_data(&dir),
        })
    }

    fn detect_blocking(&self) -> ActivityInfo {
        let Some(resolved) = self.ctx.paths.resolve(Tool::Codex) else {
            return ActivityInfo::new(Activity::NotRunning);
        };
        let last_write = Self::newest_rollout(&resolved.path).map(|(_, at)| to_utc(at));
        let running = self.ctx.processes.is_running(ToolKind::Codex);
        ActivityInfo {
            activity: activity_from(running, resolved.is_wsl(), last_write, Utc::now()),
            current_model: self
                .state()
                .rollout
                .as_ref()
                .and_then(|cache| cache.data.model.clone()),
            last_activity_at: last_write,
        }
    }

    async fn fetch_live(
        &self,
        url: &str,
        access_token: &str,
        account_id: Option<&str>,
    ) -> Result<LimitsSnapshot, ProviderError> {
        let mut request = self
            .ctx
            .http
            .get(url)
            .bearer_auth(access_token)
            .header("Accept", "application/json");
        if let Some(account_id) = account_id {
            request = request.header("ChatGPT-Account-Id", account_id);
        }
        let response = http::send(request).await?;
        interpret_live_response(&response, Utc::now())
    }
}

/// Runs a blocking closure that borrows the provider, off the async threads.
async fn blocking<T: Send + 'static>(
    provider: &CodexProvider,
    work: impl FnOnce(&CodexProvider) -> T + Send,
) -> T {
    // `block_in_place` needs the multi-thread runtime, which Tauri uses.
    tokio::task::block_in_place(|| work(provider))
}

#[async_trait]
impl Provider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn poll_interval(&self) -> Duration {
        POLL_INTERVAL
    }

    async fn detect_activity(&self) -> ActivityInfo {
        blocking(self, |provider| provider.detect_blocking()).await
    }

    async fn fetch_quota(&self) -> Result<QuotaInfo, ProviderError> {
        let local = blocking(self, |provider| provider.read_local()).await?;
        let current_model = local.rollout.model.clone().or(local.config_model.clone());

        let mut live_error = None;
        let mut note = None;
        match &local.auth {
            AuthMode::ApiKey => {
                return Ok(QuotaInfo {
                    current_model,
                    note: Some(NOTE_API_KEY.to_string()),
                    ..QuotaInfo::default()
                });
            }
            AuthMode::Missing => {
                live_error = Some(ProviderError::AuthRequired(HINT_LOGIN.to_string()));
            }
            AuthMode::ChatGpt {
                access_token,
                account_id,
            } => {
                let due = self
                    .state()
                    .next_live_at
                    .is_none_or(|at| Instant::now() >= at);
                match (&local.usage_url, due) {
                    (Some(url), true) => {
                        let result = self
                            .fetch_live(url, access_token, account_id.as_deref())
                            .await;
                        let mut state = self.state();
                        state.next_live_at = Some(Instant::now() + LIVE_INTERVAL);
                        match result {
                            Ok(live) => state.live = Some(live),
                            Err(error) => live_error = Some(error),
                        }
                    }
                    (None, _) => {
                        note = Some(
                            "Custom chatgpt_base_url: live usage is skipped, showing local logs only."
                                .to_string(),
                        );
                    }
                    (Some(_), false) => {}
                }
            }
        }

        let live = self.state().live.clone();
        let Some(merged) = merge(local.rollout.limits.clone(), live) else {
            return Err(live_error.unwrap_or_else(|| {
                ProviderError::Other("no Codex usage data found yet".to_string())
            }));
        };
        if let Some(error) = &live_error {
            note = Some(format!(
                "Live usage unavailable ({error}); showing the last local data."
            ));
        }
        Ok(QuotaInfo {
            plan: merged.plan.clone(),
            windows: to_windows(&merged, Utc::now()),
            current_model,
            note,
            ..QuotaInfo::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use ruzstd::encoding::{compress_to_vec, CompressionLevel};

    const ROLLOUT: &str = include_str!("../../../fixtures/codex/rollout.jsonl");
    const LIVE: &str = include_str!("../../../fixtures/codex/wham-usage.json");

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn snapshot(seconds: i64, used: f64) -> LimitsSnapshot {
        LimitsSnapshot {
            at: at(seconds),
            plan: Some("plus".into()),
            primary: Some(RateWindow {
                used_percent: used,
                window_seconds: Some(18_000),
                resets_at: Some(at(seconds + 3_600)),
            }),
            secondary: None,
        }
    }

    #[test]
    fn rollout_fixture_limits_and_model() {
        let limits = ROLLOUT
            .lines()
            .rev()
            .find_map(parse_token_count_line)
            .unwrap();
        assert_eq!(limits.plan.as_deref(), Some("plus"));
        let primary = limits.primary.unwrap();
        assert_eq!(primary.used_percent, 7.0);
        assert_eq!(primary.window_seconds, Some(18_000));
        assert_eq!(primary.resets_at, Some(at(1_793_042_592)));
        let secondary = limits.secondary.unwrap();
        assert_eq!(secondary.used_percent, 3.0);
        assert_eq!(secondary.window_seconds, Some(604_800));

        let model = ROLLOUT.lines().rev().find_map(parse_turn_context_line);
        assert_eq!(model.as_deref(), Some("gpt-6.1-sol"));
    }

    #[test]
    fn token_count_edge_cases() {
        // No rate limits (API key sessions).
        assert!(parse_token_count_line(
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":null}}"#
        )
        .is_none());
        // Older relative format.
        let old = parse_token_count_line(
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":12.5,"window_minutes":300,"resets_in_seconds":600}}}}"#,
        )
        .unwrap();
        let primary = old.primary.unwrap();
        assert_eq!(primary.used_percent, 12.5);
        assert_eq!(
            primary.resets_at,
            Some(old.at + chrono::Duration::seconds(600))
        );
        assert!(old.secondary.is_none());
        // Half-written line.
        assert!(parse_token_count_line(
            r#"{"type":"event_msg","payload":{"type":"token_count","rate"#
        )
        .is_none());
        assert!(parse_turn_context_line(r#"{"type":"turn_context","payload":{}}"#).is_none());
    }

    #[test]
    fn live_fixture() {
        let now = at(1_794_674_800);
        let live = parse_live_usage(LIVE, now).unwrap();
        assert_eq!(live.at, now);
        assert_eq!(live.plan.as_deref(), Some("plus"));
        let primary = live.primary.clone().unwrap();
        assert_eq!(primary.used_percent, 0.0);
        assert_eq!(primary.window_seconds, Some(18_000));
        assert_eq!(primary.resets_at, Some(at(1_794_692_793)));
        assert_eq!(live.secondary.clone().unwrap().used_percent, 1.0);

        let windows = to_windows(&live, now);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].id, "five_hour");
        assert_eq!(windows[1].id, "weekly");
        assert_eq!(windows[1].used_percent, Some(1.0));
        assert!(!windows[1].estimated);
    }

    #[test]
    fn live_tolerates_missing_parts_and_maps_errors() {
        let now = at(1_800_000_000);
        let sparse = parse_live_usage(
            r#"{"plan_type":"pro","rate_limit":null,"new_field":1}"#,
            now,
        )
        .unwrap();
        assert!(sparse.primary.is_none() && sparse.secondary.is_none());
        assert!(to_windows(&sparse, now).is_empty());
        assert!(parse_live_usage("[]", now).is_err());

        let response = |status: u16| HttpResponse {
            status,
            retry_after: None,
            body: LIVE.to_string(),
        };
        assert!(interpret_live_response(&response(200), now).is_ok());
        assert!(matches!(
            interpret_live_response(&response(401), now),
            Err(ProviderError::AuthRequired(_))
        ));
        assert!(matches!(
            interpret_live_response(&response(429), now),
            Err(ProviderError::RateLimited { .. })
        ));
    }

    #[test]
    fn merge_prefers_newer_source() {
        let older = snapshot(1_000, 10.0);
        let newer = snapshot(2_000, 20.0);
        assert_eq!(
            merge(Some(older.clone()), Some(newer.clone())),
            Some(newer.clone())
        );
        assert_eq!(
            merge(Some(newer.clone()), Some(older.clone())),
            Some(newer.clone())
        );
        assert_eq!(merge(None, Some(older.clone())), Some(older.clone()));
        assert_eq!(merge(Some(older.clone()), None), Some(older));
        assert_eq!(merge(None, None), None);
    }

    #[test]
    fn passed_reset_becomes_estimated_zero() {
        let stale = snapshot(1_000, 80.0);
        let before = to_windows(&stale, at(2_000));
        assert_eq!(before[0].used_percent, Some(80.0));
        assert_eq!(before[0].resets_at, Some(at(4_600)));
        assert!(!before[0].estimated);

        let after = to_windows(&stale, at(5_000));
        assert_eq!(after[0].used_percent, Some(0.0));
        assert_eq!(after[0].resets_at, None);
        assert!(after[0].estimated);
    }

    #[test]
    fn window_labels() {
        assert_eq!(window_identity(Some(18_000), false).0, "five_hour");
        assert_eq!(window_identity(Some(604_800), true).0, "weekly");
        assert_eq!(
            window_identity(Some(86_400 * 30), false),
            ("secondary".to_string(), "30 days".to_string())
        );
        assert_eq!(window_identity(Some(7_200), true).1, "2 hours");
        assert_eq!(window_identity(None, true).0, "five_hour");
        assert_eq!(window_identity(None, false).0, "weekly");
    }

    #[test]
    fn auth_modes() {
        assert!(matches!(
            parse_auth(r#"{"OPENAI_API_KEY":null,"tokens":{"access_token":"t","account_id":"a"}}"#),
            AuthMode::ChatGpt {
                account_id: Some(_),
                ..
            }
        ));
        assert!(matches!(
            parse_auth(r#"{"OPENAI_API_KEY":"sk-test","tokens":null}"#),
            AuthMode::ApiKey
        ));
        assert!(matches!(
            parse_auth(r#"{"OPENAI_API_KEY":null}"#),
            AuthMode::Missing
        ));
        assert!(matches!(parse_auth("garbage"), AuthMode::Missing));
    }

    #[test]
    fn config_parsing_and_usage_url() {
        let toml = "model = \"gpt-x\"\n# comment\nchatgpt_base_url = 'https://chatgpt.com/backend-api/'\n[profiles.a]\nmodel = \"other\"\n";
        assert_eq!(
            toml_top_level_string(toml, "model").as_deref(),
            Some("gpt-x")
        );
        assert_eq!(toml_top_level_string(toml, "missing"), None);
        assert_eq!(toml_top_level_string("[t]\nmodel = \"x\"", "model"), None);

        assert_eq!(usage_url(None).as_deref(), Some(DEFAULT_USAGE_URL));
        assert_eq!(
            usage_url(Some("model = \"x\"")).as_deref(),
            Some(DEFAULT_USAGE_URL)
        );
        assert_eq!(usage_url(Some(toml)).as_deref(), Some(DEFAULT_USAGE_URL));
        assert_eq!(
            usage_url(Some("chatgpt_base_url = \"https://proxy.openai.com\"")).as_deref(),
            Some("https://proxy.openai.com/api/codex/usage")
        );
        // The token must never go to a foreign or non-TLS host.
        assert_eq!(
            usage_url(Some(
                "chatgpt_base_url = \"https://evil.example/backend-api\""
            )),
            None
        );
        assert_eq!(
            usage_url(Some("chatgpt_base_url = \"https://notopenai.com\"")),
            None
        );
        assert_eq!(
            usage_url(Some("chatgpt_base_url = \"http://chatgpt.com\"")),
            None
        );
    }

    #[test]
    fn reads_plain_and_zstd_rollouts() {
        let dir = tempfile::tempdir().unwrap();

        let plain = dir.path().join("rollout-a.jsonl");
        std::fs::write(&plain, ROLLOUT).unwrap();
        let from_plain = read_rollout(&plain);
        assert_eq!(from_plain.model.as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(
            from_plain
                .limits
                .as_ref()
                .unwrap()
                .secondary
                .as_ref()
                .unwrap()
                .used_percent,
            3.0
        );

        // Two concatenated frames, the second one ending without a newline.
        let (first, second) = ROLLOUT.split_at(ROLLOUT.find('\n').unwrap() + 1);
        let mut bytes = compress_to_vec(first.as_bytes(), CompressionLevel::Fastest);
        bytes.extend(compress_to_vec(
            second.trim_end().as_bytes(),
            CompressionLevel::Fastest,
        ));
        let compressed = dir.path().join("rollout-b.jsonl.zst");
        std::fs::write(&compressed, bytes).unwrap();
        assert_eq!(read_rollout(&compressed), from_plain);

        let garbage = dir.path().join("rollout-c.jsonl.zst");
        std::fs::write(&garbage, b"not zstd at all").unwrap();
        assert_eq!(read_rollout(&garbage), RolloutData::default());
    }
}
