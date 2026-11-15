//! Claude (Claude Code): quota from the OAuth usage endpoint, activity and
//! model from the local session logs.
//!
//! Read-only by design: the OAuth token is never refreshed here, because that
//! could invalidate Claude Code's own session.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{activity_from, humanize, parse_time, to_utc, ProviderContext};
use crate::engine::http::{self, HttpResponse};
use crate::engine::model::{Activity, ProviderId, QuotaWindow};
use crate::engine::paths::Tool;
use crate::engine::process::ToolKind;
use crate::engine::provider::{ActivityInfo, Provider, ProviderError, QuotaInfo};
use crate::engine::tail;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
/// The endpoint rate-limits aggressively; never poll faster than this.
pub const MIN_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// A statusline dump older than this is not worth showing.
const STATUSLINE_MAX_AGE_SECS: i64 = 6 * 3600;
pub const STATUSLINE_FILE: &str = "claude-statusline.json";

const HINT_LOGIN: &str =
    "Claude Code credentials not found. Sign in with the Claude Code CLI (`claude`, then /login).";
const HINT_EXPIRED: &str = "Claude Code login expired. Open Claude Code to refresh it.";
const HINT_REJECTED: &str = "Claude rejected the saved login. Open Claude Code and sign in again.";

/// Deliberately not `Debug`: it holds a secret.
pub struct Credentials {
    pub access_token: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub subscription: Option<String>,
}

impl Credentials {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|at| at <= now)
    }
}

pub fn parse_credentials(json: &str) -> Option<Credentials> {
    let value: Value = serde_json::from_str(json).ok()?;
    let oauth = value.get("claudeAiOauth")?;
    let access_token = oauth.get("accessToken")?.as_str()?.to_string();
    if access_token.is_empty() {
        return None;
    }
    Some(Credentials {
        access_token,
        expires_at: oauth.get("expiresAt").and_then(parse_time),
        subscription: oauth
            .get("subscriptionType")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn window_meta(key: &str) -> (String, String) {
    match key {
        "five_hour" => ("five_hour".to_string(), "5 hours".to_string()),
        "seven_day" => ("weekly".to_string(), "Weekly".to_string()),
        other => match other.strip_prefix("seven_day_") {
            Some(model) => (
                format!("weekly:{model}"),
                format!("Weekly · {}", humanize(model)),
            ),
            None => (other.to_string(), humanize(other)),
        },
    }
}

/// Parses the usage endpoint body. Unknown buckets are kept (anything with a
/// numeric `utilization`), unknown shapes are ignored.
pub fn parse_usage(body: &str) -> Result<Vec<QuotaWindow>, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::Other("usage response is not valid JSON".to_string()))?;
    let Some(object) = value.as_object() else {
        return Err(ProviderError::Other(
            "usage response has an unexpected shape".to_string(),
        ));
    };

    let mut keys: Vec<&String> = object.keys().filter(|key| *key != "extra_usage").collect();
    // Well-known windows first, the rest alphabetically.
    keys.sort_by_key(|key| match key.as_str() {
        "five_hour" => (0, String::new()),
        "seven_day" => (1, String::new()),
        other => (2, other.to_string()),
    });

    let mut windows = Vec::new();
    for key in keys {
        let bucket = &object[key];
        let Some(utilization) = bucket.get("utilization").and_then(Value::as_f64) else {
            continue;
        };
        let (id, label) = window_meta(key);
        windows.push(
            QuotaWindow::percent(&id, &label, utilization as f32)
                .with_reset(bucket.get("resets_at").and_then(parse_time)),
        );
    }

    if let Some(extra) = object.get("extra_usage") {
        let enabled = extra
            .get("is_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if enabled {
            let limit = extra.get("monthly_limit").and_then(Value::as_f64);
            let used = extra.get("used_credits").and_then(Value::as_f64);
            let percent = extra
                .get("utilization")
                .and_then(Value::as_f64)
                .or_else(|| match (used, limit) {
                    (Some(used), Some(limit)) if limit > 0.0 => Some(used / limit * 100.0),
                    _ => None,
                });
            windows.push(QuotaWindow {
                id: "extra_usage".to_string(),
                label: "Extra usage".to_string(),
                used_percent: percent.map(|p| (p as f32).clamp(0.0, 100.0)),
                remaining: match (used, limit) {
                    (Some(used), Some(limit)) => Some((limit - used).max(0.0)),
                    _ => None,
                },
                limit,
                unit: Some("credits".to_string()),
                resets_at: None,
                estimated: false,
            });
        }
    }
    Ok(windows)
}

/// Turns the HTTP response of the usage endpoint into windows or a typed error.
pub fn interpret_usage_response(
    response: &HttpResponse,
) -> Result<Vec<QuotaWindow>, ProviderError> {
    http::check_status(response, HINT_REJECTED)?;
    parse_usage(&response.body)
}

/// Parses the JSON Claude Code hands to a statusline script (or just its
/// `rate_limits` object). Returns `(windows, model)`.
pub fn parse_statusline(json: &str) -> Option<(Vec<QuotaWindow>, Option<String>)> {
    let value: Value = serde_json::from_str(json).ok()?;
    let limits = value.get("rate_limits").unwrap_or(&value);
    let mut windows = Vec::new();
    for (key, id, label) in [
        ("five_hour", "five_hour", "5 hours"),
        ("seven_day", "weekly", "Weekly"),
    ] {
        let Some(bucket) = limits.get(key) else {
            continue;
        };
        let Some(used) = bucket.get("used_percentage").and_then(Value::as_f64) else {
            continue;
        };
        windows.push(
            QuotaWindow::percent(id, label, used as f32)
                .with_reset(bucket.get("resets_at").and_then(parse_time)),
        );
    }
    if windows.is_empty() {
        return None;
    }
    let model = value
        .get("model")
        .and_then(|model| model.get("id").or_else(|| model.get("display_name")))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((windows, model))
}

/// Extracts `(model, timestamp)` from one session log line if it is an assistant turn.
pub fn parse_assistant_line(line: &str) -> Option<(String, Option<DateTime<Utc>>)> {
    // Cheap pre-check: most lines are something else and some are huge.
    if !line.contains("\"assistant\"") {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let model = value.get("message")?.get("model")?.as_str()?;
    if model.is_empty() || model.starts_with('<') {
        // "<synthetic>" marks locally generated messages.
        return None;
    }
    Some((
        model.to_string(),
        value.get("timestamp").and_then(parse_time),
    ))
}

type LastTurn = Option<(String, Option<DateTime<Utc>>)>;

/// Result of the last log scan, reused while the newest log file is unchanged
/// so an idle machine does not re-read (possibly huge) logs every few seconds.
struct TurnCache {
    path: PathBuf,
    modified: SystemTime,
    turn: LastTurn,
}

pub struct ClaudeProvider {
    ctx: ProviderContext,
    turn_cache: Arc<Mutex<Option<TurnCache>>>,
}

struct LocalState {
    dir: PathBuf,
    is_wsl: bool,
}

impl ClaudeProvider {
    pub fn new(ctx: ProviderContext) -> Self {
        Self {
            ctx,
            turn_cache: Arc::new(Mutex::new(None)),
        }
    }

    fn locate(ctx: &ProviderContext) -> Option<LocalState> {
        ctx.paths.resolve(Tool::Claude).map(|resolved| LocalState {
            is_wsl: resolved.is_wsl(),
            dir: resolved.path,
        })
    }

    fn last_turn(cache: &Mutex<Option<TurnCache>>, path: &Path, modified: SystemTime) -> LastTurn {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = cache.as_ref() {
            if cached.path == path && cached.modified == modified {
                return cached.turn.clone();
            }
        }
        let turn = tail::find_last(path, parse_assistant_line);
        *cache = Some(TurnCache {
            path: path.to_path_buf(),
            modified,
            turn: turn.clone(),
        });
        turn
    }

    fn detect_blocking(ctx: &ProviderContext, cache: &Mutex<Option<TurnCache>>) -> ActivityInfo {
        let Some(local) = Self::locate(ctx) else {
            return ActivityInfo::new(Activity::NotRunning);
        };
        let newest = tail::newest_file(&local.dir.join("projects"), &[".jsonl"], 4);
        let last_write = newest.as_ref().map(|(_, modified)| to_utc(*modified));
        let process_running = ctx.processes.is_running(ToolKind::ClaudeCode);
        let activity = activity_from(process_running, local.is_wsl, last_write, Utc::now());

        let last_turn = newest
            .as_ref()
            .and_then(|(path, modified)| Self::last_turn(cache, path, *modified));
        let (current_model, turn_time) = match last_turn {
            Some((model, time)) => (Some(model), time),
            None => (None, None),
        };
        ActivityInfo {
            activity,
            current_model,
            last_activity_at: turn_time.or(last_write),
        }
    }

    /// Secondary source: a dump written by an (optional) statusline hook.
    fn read_statusline(path: &Path, now: DateTime<Utc>) -> Option<QuotaInfo> {
        let modified = to_utc(std::fs::metadata(path).ok()?.modified().ok()?);
        if (now - modified).num_seconds() > STATUSLINE_MAX_AGE_SECS {
            return None;
        }
        let (windows, current_model) = parse_statusline(&std::fs::read_to_string(path).ok()?)?;
        Some(QuotaInfo {
            windows,
            current_model,
            note: Some(format!(
                "From the Claude Code statusline, {}",
                modified.format("%H:%M UTC")
            )),
            ..QuotaInfo::default()
        })
    }

    fn load_credentials(ctx: &ProviderContext) -> Result<Credentials, ProviderError> {
        let local = Self::locate(ctx).ok_or(ProviderError::NotInstalled)?;
        let raw = std::fs::read_to_string(local.dir.join(".credentials.json"))
            .map_err(|_| ProviderError::AuthRequired(HINT_LOGIN.to_string()))?;
        let credentials = parse_credentials(&raw)
            .ok_or_else(|| ProviderError::AuthRequired(HINT_LOGIN.to_string()))?;
        if credentials.is_expired(Utc::now()) {
            return Err(ProviderError::AuthRequired(HINT_EXPIRED.to_string()));
        }
        Ok(credentials)
    }

    async fn statusline_fallback(&self) -> Option<QuotaInfo> {
        let path = self.ctx.data_dir.join(STATUSLINE_FILE);
        tokio::task::spawn_blocking(move || Self::read_statusline(&path, Utc::now()))
            .await
            .ok()
            .flatten()
    }
}

#[async_trait]
impl Provider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn poll_interval(&self) -> Duration {
        MIN_POLL_INTERVAL
    }

    async fn detect_activity(&self) -> ActivityInfo {
        let ctx = self.ctx.clone();
        let cache = Arc::clone(&self.turn_cache);
        tokio::task::spawn_blocking(move || Self::detect_blocking(&ctx, &cache))
            .await
            .unwrap_or_else(|_| ActivityInfo::unknown())
    }

    async fn fetch_quota(&self) -> Result<QuotaInfo, ProviderError> {
        let ctx = self.ctx.clone();
        let credentials = tokio::task::spawn_blocking(move || Self::load_credentials(&ctx))
            .await
            .map_err(|_| ProviderError::Other("credential lookup failed".to_string()))?;

        let credentials = match credentials {
            Ok(credentials) => credentials,
            Err(ProviderError::NotInstalled) => return Err(ProviderError::NotInstalled),
            Err(error) => return self.statusline_fallback().await.ok_or(error),
        };

        let request = self
            .ctx
            .http
            .get(USAGE_URL)
            .bearer_auth(&credentials.access_token)
            .header("anthropic-beta", OAUTH_BETA);
        let result = match http::send(request).await {
            Ok(response) => interpret_usage_response(&response),
            Err(error) => Err(error),
        };

        match result {
            Ok(windows) => Ok(QuotaInfo {
                plan: credentials.subscription,
                windows,
                ..QuotaInfo::default()
            }),
            // Cached numbers beat no numbers while the endpoint refuses us.
            Err(error) => self.statusline_fallback().await.ok_or(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const USAGE: &str = include_str!("../../../fixtures/claude/usage.json");
    const SESSION: &str = include_str!("../../../fixtures/claude/session.jsonl");
    const STATUSLINE: &str = include_str!("../../../fixtures/claude/statusline.json");
    const CREDENTIALS: &str = include_str!("../../../fixtures/claude/credentials.json");

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            retry_after: (status == 429).then(|| Duration::from_secs(300)),
            body: body.to_string(),
        }
    }

    #[test]
    fn parses_usage_fixture() {
        let windows = parse_usage(USAGE).unwrap();
        let ids: Vec<&str> = windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "five_hour",
                "weekly",
                "weekly:opus",
                "weekly:sonnet",
                "extra_usage"
            ]
        );
        assert_eq!(windows[0].used_percent, Some(37.0));
        assert_eq!(
            windows[0].resets_at,
            Some(Utc.timestamp_opt(1_794_686_400, 0).unwrap())
        );
        assert_eq!(windows[1].label, "Weekly");
        assert_eq!(windows[2].label, "Weekly · Opus");
        assert_eq!(windows[3].resets_at, None);

        let extra = &windows[4];
        assert_eq!(extra.used_percent, Some(25.0));
        assert_eq!(extra.limit, Some(5000.0));
        assert_eq!(extra.remaining, Some(3750.0));
        assert_eq!(extra.unit.as_deref(), Some("credits"));
    }

    #[test]
    fn usage_tolerates_unknown_and_missing_fields() {
        let windows = parse_usage(
            r#"{"five_hour":{"utilization":140},"brand_new":{"utilization":5,"resets_at":"garbage"},
                "weird":[1,2],"extra_usage":{"is_enabled":false},"seven_day":null}"#,
        )
        .unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].used_percent, Some(100.0));
        assert_eq!(windows[1].id, "brand_new");
        assert_eq!(windows[1].label, "Brand new");
        assert_eq!(windows[1].resets_at, None);

        assert!(parse_usage("[]").is_err());
        assert!(parse_usage("<html>").is_err());
        assert_eq!(parse_usage("{}").unwrap(), Vec::new());
    }

    #[test]
    fn interprets_error_statuses() {
        assert!(interpret_usage_response(&response(200, USAGE)).is_ok());
        assert_eq!(
            interpret_usage_response(&response(401, "{}")),
            Err(ProviderError::AuthRequired(HINT_REJECTED.to_string()))
        );
        assert_eq!(
            interpret_usage_response(&response(429, "")),
            Err(ProviderError::RateLimited {
                retry_after: Some(Duration::from_secs(300))
            })
        );
        assert!(matches!(
            interpret_usage_response(&response(500, "")),
            Err(ProviderError::Other(_))
        ));
    }

    #[test]
    fn credentials_and_expiry() {
        let credentials = parse_credentials(CREDENTIALS).unwrap();
        assert_eq!(credentials.subscription.as_deref(), Some("max"));
        let expiry = Utc.timestamp_opt(1_794_700_000, 0).unwrap();
        assert_eq!(credentials.expires_at, Some(expiry));
        assert!(!credentials.is_expired(expiry - chrono::Duration::seconds(1)));
        assert!(credentials.is_expired(expiry));

        assert!(parse_credentials("{}").is_none());
        assert!(parse_credentials(r#"{"claudeAiOauth":{"accessToken":""}}"#).is_none());
        assert!(parse_credentials("nope").is_none());
    }

    #[test]
    fn statusline_fixture() {
        let (windows, model) = parse_statusline(STATUSLINE).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].id, "five_hour");
        assert_eq!(windows[0].used_percent, Some(23.5));
        assert_eq!(
            windows[0].resets_at,
            Some(Utc.timestamp_opt(1_794_690_000, 0).unwrap())
        );
        assert_eq!(windows[1].id, "weekly");
        assert_eq!(model.as_deref(), Some("claude-fable-5-2"));

        // Bare `rate_limits` object and plans without the field.
        assert!(parse_statusline(r#"{"five_hour":{"used_percentage":1}}"#).is_some());
        assert!(parse_statusline(r#"{"model":{"id":"x"}}"#).is_none());
    }

    #[test]
    fn last_assistant_turn_from_session_fixture() {
        let last = SESSION
            .lines()
            .rev()
            .find_map(parse_assistant_line)
            .expect("fixture has an assistant line");
        assert!(last.0.starts_with("claude-"));
        assert!(last.1.is_some());
    }

    #[test]
    fn assistant_line_edge_cases() {
        assert!(parse_assistant_line(r#"{"type":"user","message":{"role":"user"}}"#).is_none());
        assert!(
            parse_assistant_line(r#"{"type":"assistant","message":{"model":"<synthetic>"}}"#)
                .is_none()
        );
        // Line still being written.
        assert!(parse_assistant_line(r#"{"type":"assistant","message":{"model":"cla"#).is_none());
        let (model, time) =
            parse_assistant_line(r#"{"type":"assistant","message":{"model":"claude-x"}}"#).unwrap();
        assert_eq!(model, "claude-x");
        assert_eq!(time, None);
    }

    #[test]
    fn log_is_only_rescanned_when_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, SESSION).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let cache = Mutex::new(None);

        let first = ClaudeProvider::last_turn(&cache, &path, modified);
        assert!(first.is_some());
        // Same mtime: served from the cache even though the file is gone.
        std::fs::remove_file(&path).unwrap();
        assert_eq!(ClaudeProvider::last_turn(&cache, &path, modified), first);
        // New mtime: scanned again (and the file no longer exists).
        let later = modified + Duration::from_secs(5);
        assert_eq!(ClaudeProvider::last_turn(&cache, &path, later), None);
    }

    #[test]
    fn statusline_file_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATUSLINE_FILE);
        std::fs::write(&path, STATUSLINE).unwrap();

        let now = Utc::now();
        let info = ClaudeProvider::read_statusline(&path, now).unwrap();
        assert_eq!(info.windows.len(), 2);
        assert!(info.note.is_some());

        let much_later = now + chrono::Duration::seconds(STATUSLINE_MAX_AGE_SECS + 60);
        assert!(ClaudeProvider::read_statusline(&path, much_later).is_none());
        assert!(ClaudeProvider::read_statusline(&dir.path().join("missing.json"), now).is_none());
    }
}
