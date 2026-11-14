//! GitHub Copilot: monthly quota from GitHub's internal `copilot_internal/user`
//! endpoint. The response shape is undocumented and changed with the move to
//! AI Credits, so the parser accepts anything that looks like a quota.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;

use super::{humanize, parse_time, ProviderContext};
use crate::engine::cmd::run_hidden;
use crate::engine::http::{self, HttpResponse};
use crate::engine::model::{Activity, ProviderId, QuotaWindow};
use crate::engine::process::ToolKind;
use crate::engine::provider::{ActivityInfo, Provider, ProviderError, QuotaInfo};

const USER_URL: &str = "https://api.github.com/copilot_internal/user";
const API_VERSION: &str = "2025-04-01";
const POLL_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// A quota change seen within this window counts as "in use".
const ACTIVE_WINDOW_SECS: i64 = 15 * 60;

const HINT_NO_TOKEN: &str =
    "No GitHub token found. Connect GitHub in Settings, or install GitHub CLI and run `gh auth login`.";
const HINT_REJECTED: &str =
    "GitHub rejected the token for Copilot. Reconnect GitHub in Settings (the account needs Copilot access).";

/// Token obtained by the app's own device flow (set from Settings). Shared so
/// that connecting takes effect without a restart.
pub type SharedToken = Arc<RwLock<Option<String>>>;

/// Extracts the OAuth token from Copilot's `apps.json` / `hosts.json`.
pub fn parse_copilot_config(json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(json).ok()?;
    value.as_object()?.iter().find_map(|(host, entry)| {
        if !host.starts_with("github.com") {
            return None;
        }
        entry
            .get("oauth_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
    })
}

fn copilot_config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("github-copilot"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        dirs.push(PathBuf::from(home).join(".config").join("github-copilot"));
    }
    dirs
}

pub fn token_from_dirs(dirs: &[PathBuf]) -> Option<String> {
    dirs.iter()
        .flat_map(|dir| ["apps.json", "hosts.json"].map(|name| dir.join(name)))
        .find_map(|file: PathBuf| {
            std::fs::read_to_string(file)
                .ok()
                .and_then(|json| parse_copilot_config(&json))
        })
}

fn token_from_gh() -> Option<String> {
    let output = run_hidden("gh", &["auth", "token"], Duration::from_secs(8))?;
    let token = String::from_utf8_lossy(&output).trim().to_string();
    (!token.is_empty()).then_some(token)
}

fn number(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_f64))
}

fn window_identity(key: &str) -> (String, String, Option<&'static str>) {
    let lower = key.to_ascii_lowercase();
    if lower.contains("credit") {
        ("monthly".into(), "AI Credits".into(), Some("credits"))
    } else if lower == "premium_interactions" || lower.contains("premium") {
        (
            "monthly".into(),
            "Premium requests".into(),
            Some("requests"),
        )
    } else if lower == "chat" {
        ("chat".into(), "Chat".into(), Some("requests"))
    } else if lower == "completions" {
        ("completions".into(), "Completions".into(), Some("requests"))
    } else {
        (lower, humanize(key), None)
    }
}

/// Builds a window from one quota object, whatever its exact field names are.
/// Returns `None` for unlimited quotas and for objects that are not quotas.
fn window_from_quota(key: &str, quota: &Value) -> Option<QuotaWindow> {
    if !quota.is_object() || quota.get("unlimited").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let limit = number(
        quota,
        &["entitlement", "limit", "included", "total", "quota"],
    )
    .filter(|limit| *limit > 0.0);
    let remaining = number(quota, &["remaining", "quota_remaining", "balance"]);
    let used = number(quota, &["used", "consumed", "usage"]);
    let remaining = remaining.or_else(|| match (limit, used) {
        (Some(limit), Some(used)) => Some((limit - used).max(0.0)),
        _ => None,
    });

    let used_percent = number(quota, &["percent_remaining"])
        .map(|percent| 100.0 - percent)
        .or_else(|| number(quota, &["percent_used", "used_percent", "utilization"]))
        .or_else(|| match (limit, remaining) {
            (Some(limit), Some(remaining)) => Some((1.0 - remaining / limit) * 100.0),
            _ => None,
        });
    if used_percent.is_none() && remaining.is_none() {
        return None;
    }

    let (id, label, unit) = window_identity(key);
    Some(QuotaWindow {
        id,
        label,
        used_percent: used_percent.map(|percent| (percent as f32).clamp(0.0, 100.0)),
        remaining,
        limit,
        unit: unit.map(str::to_string),
        resets_at: None,
        estimated: false,
    })
}

fn parse_reset(value: &Value) -> Option<DateTime<Utc>> {
    for key in [
        "quota_reset_date_utc",
        "quota_reset_date",
        "limited_user_reset_date",
    ] {
        let Some(field) = value.get(key) else {
            continue;
        };
        if let Some(time) = parse_time(field) {
            return Some(time);
        }
        // Date-only form: "2026-12-01".
        let date = field
            .as_str()
            .and_then(|text| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok())
            .and_then(|date| date.and_hms_opt(0, 0, 0));
        if let Some(date) = date {
            return Some(date.and_utc());
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
pub struct CopilotUsage {
    pub plan: Option<String>,
    pub windows: Vec<QuotaWindow>,
}

/// Parses `copilot_internal/user`. Handles premium requests, AI Credits and the
/// free plan; unknown fields are ignored and never cause an error.
pub fn parse_user(body: &str) -> Result<CopilotUsage, ProviderError> {
    let value: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::Other("Copilot response is not valid JSON".to_string()))?;
    let Some(object) = value.as_object() else {
        return Err(ProviderError::Other(
            "Copilot response has an unexpected shape".to_string(),
        ));
    };
    let resets_at = parse_reset(&value);
    let mut windows: Vec<QuotaWindow> = Vec::new();
    let mut push = |key: &str, window: Option<QuotaWindow>| {
        let Some(mut window) = window else {
            return;
        };
        // Two quotas mapping to the same id (e.g. premium + credits): keep both.
        if windows.iter().any(|existing| existing.id == window.id) {
            window.id = key.to_ascii_lowercase();
        }
        window.resets_at = resets_at;
        windows.push(window);
    };

    if let Some(snapshots) = object.get("quota_snapshots").and_then(Value::as_object) {
        // Deterministic order: credits first, then premium requests, then the rest.
        let mut entries: Vec<(&String, &Value)> = snapshots.iter().collect();
        entries.sort_by_key(|(key, _)| {
            let lower = key.to_ascii_lowercase();
            let rank = if lower.contains("credit") {
                0
            } else if lower.contains("premium") {
                1
            } else {
                2
            };
            (rank, lower)
        });
        for (key, quota) in entries {
            push(key, window_from_quota(key, quota));
        }
    }
    // A credits object directly at the top level.
    for (key, quota) in object {
        if key.to_ascii_lowercase().contains("credit") {
            push(key, window_from_quota(key, quota));
        }
    }
    // Copilot Free: remaining counts plus monthly totals.
    if let (Some(left), Some(totals)) = (
        object.get("limited_user_quotas").and_then(Value::as_object),
        object.get("monthly_quotas").and_then(Value::as_object),
    ) {
        for (key, remaining) in left {
            let quota = serde_json::json!({
                "remaining": remaining,
                "entitlement": totals.get(key),
            });
            push(key, window_from_quota(key, &quota));
        }
    }

    // The headline quota first, the rest alphabetically.
    windows.sort_by(|a, b| {
        (a.id != "monthly", a.id.as_str()).cmp(&(b.id != "monthly", b.id.as_str()))
    });
    Ok(CopilotUsage {
        plan: object
            .get("copilot_plan")
            .and_then(Value::as_str)
            .map(str::to_string),
        windows,
    })
}

pub fn interpret_user_response(response: &HttpResponse) -> Result<CopilotUsage, ProviderError> {
    if response.status == 404 {
        // GitHub answers 404 when the account has no Copilot seat.
        return Err(ProviderError::AuthRequired(HINT_REJECTED.to_string()));
    }
    http::check_status(response, HINT_REJECTED)?;
    parse_user(&response.body)
}

/// Stable text describing the quota numbers, used to notice consumption between polls.
pub fn usage_signature(windows: &[QuotaWindow]) -> String {
    windows
        .iter()
        .map(|window| {
            format!(
                "{}:{:?}:{:?}",
                window.id, window.used_percent, window.remaining
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

#[derive(Default)]
struct State {
    signature: Option<String>,
    last_change_at: Option<DateTime<Utc>>,
}

impl State {
    /// Records a poll result; the very first observation is not a change.
    fn observe(&mut self, signature: String, now: DateTime<Utc>) {
        if self.signature.as_ref().is_some_and(|old| *old != signature) {
            self.last_change_at = Some(now);
        }
        self.signature = Some(signature);
    }

    fn activity(&self, editor_running: bool, now: DateTime<Utc>) -> Activity {
        if !editor_running {
            return Activity::NotRunning;
        }
        let recent = self
            .last_change_at
            .is_some_and(|at| (now - at).num_seconds() <= ACTIVE_WINDOW_SECS);
        if recent {
            Activity::Active
        } else {
            Activity::Idle
        }
    }
}

pub struct CopilotProvider {
    ctx: ProviderContext,
    app_token: SharedToken,
    state: Mutex<State>,
}

impl CopilotProvider {
    pub fn new(ctx: ProviderContext, app_token: SharedToken) -> Self {
        Self {
            ctx,
            app_token,
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Order: Copilot's own config files, GitHub CLI, then the app's device-flow token.
    fn find_token(&self) -> Option<String> {
        token_from_dirs(&copilot_config_dirs())
            .or_else(token_from_gh)
            .or_else(|| {
                self.app_token
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            })
    }
}

#[async_trait]
impl Provider for CopilotProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Copilot
    }

    fn poll_interval(&self) -> Duration {
        POLL_INTERVAL
    }

    async fn detect_activity(&self) -> ActivityInfo {
        let running =
            tokio::task::block_in_place(|| self.ctx.processes.is_running(ToolKind::CopilotEditor));
        let state = self.state();
        ActivityInfo {
            activity: state.activity(running, Utc::now()),
            // Copilot exposes no reliable local record of the model in use.
            current_model: None,
            last_activity_at: state.last_change_at,
        }
    }

    async fn fetch_quota(&self) -> Result<QuotaInfo, ProviderError> {
        let token = tokio::task::block_in_place(|| self.find_token())
            .ok_or_else(|| ProviderError::AuthRequired(HINT_NO_TOKEN.to_string()))?;
        let request = self
            .ctx
            .http
            .get(USER_URL)
            .header("Authorization", format!("token {token}"))
            .header("Accept", "application/json")
            .header("X-GitHub-Api-Version", API_VERSION);
        let response = http::send(request).await?;
        let usage = interpret_user_response(&response)?;

        self.state()
            .observe(usage_signature(&usage.windows), Utc::now());
        let note = usage
            .windows
            .is_empty()
            .then(|| "No limited Copilot quota on this plan.".to_string());
        Ok(QuotaInfo {
            plan: usage.plan,
            windows: usage.windows,
            note,
            ..QuotaInfo::default()
        })
    }
}

/// Used by tests and by Settings to check a directory for a token.
pub fn token_from_dir(dir: &Path) -> Option<String> {
    token_from_dirs(&[dir.to_path_buf()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const PREMIUM: &str = include_str!("../../../fixtures/copilot/user-premium.json");
    const CREDITS: &str = include_str!("../../../fixtures/copilot/user-credits.json");
    const FREE: &str = include_str!("../../../fixtures/copilot/user-free.json");

    fn december_first() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap()
    }

    #[test]
    fn premium_requests_shape() {
        let usage = parse_user(PREMIUM).unwrap();
        assert_eq!(usage.plan.as_deref(), Some("individual_pro"));
        // chat and completions are unlimited and therefore not shown.
        assert_eq!(usage.windows.len(), 1);
        let monthly = &usage.windows[0];
        assert_eq!(monthly.id, "monthly");
        assert_eq!(monthly.label, "Premium requests");
        assert!((monthly.used_percent.unwrap() - 38.7).abs() < 0.01);
        assert_eq!(monthly.remaining, Some(183.0));
        assert_eq!(monthly.limit, Some(300.0));
        assert_eq!(monthly.unit.as_deref(), Some("requests"));
        assert_eq!(monthly.resets_at, Some(december_first()));
    }

    #[test]
    fn ai_credits_shape() {
        let usage = parse_user(CREDITS).unwrap();
        assert_eq!(usage.windows.len(), 1);
        let monthly = &usage.windows[0];
        assert_eq!(monthly.id, "monthly");
        assert_eq!(monthly.label, "AI Credits");
        assert_eq!(monthly.used_percent, Some(57.5));
        assert_eq!(monthly.remaining, Some(425.0));
        assert_eq!(monthly.limit, Some(1000.0));
        assert_eq!(monthly.unit.as_deref(), Some("credits"));
        assert_eq!(monthly.resets_at, Some(december_first()));
    }

    #[test]
    fn free_plan_shape() {
        let usage = parse_user(FREE).unwrap();
        assert_eq!(usage.plan.as_deref(), Some("free"));
        let ids: Vec<&str> = usage.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["chat", "completions"]);
        assert_eq!(usage.windows[0].used_percent, Some(76.0));
        assert_eq!(usage.windows[0].remaining, Some(12.0));
        assert_eq!(usage.windows[1].used_percent, Some(25.0));
        assert_eq!(
            usage.windows[0].resets_at,
            Some(Utc.with_ymd_and_hms(2026, 12, 3, 0, 0, 0).unwrap())
        );
    }

    #[test]
    fn unknown_shapes_never_crash() {
        let usage = parse_user(
            r#"{"copilot_plan":"business","quota_snapshots":{
                 "premium_interactions":{"entitlement":300,"remaining":150,"unlimited":false},
                 "ai_credits":{"limit":2000,"used":500},
                 "mystery":{"percent_remaining":10},
                 "broken":"text","empty":{}},
               "credits":{"included":100,"consumed":25},
               "surprise":[1,2,3]}"#,
        )
        .unwrap();
        let ids: Vec<&str> = usage.windows.iter().map(|w| w.id.as_str()).collect();
        // Sorted: "monthly" first, then alphabetically; duplicates keep their own key.
        assert_eq!(
            ids,
            vec!["monthly", "credits", "mystery", "premium_interactions"]
        );
        assert_eq!(usage.windows[0].used_percent, Some(25.0));
        assert_eq!(usage.windows[0].label, "AI Credits");
        assert_eq!(usage.windows[1].remaining, Some(75.0));
        assert_eq!(usage.windows[2].used_percent, Some(90.0));
        assert_eq!(usage.windows[3].used_percent, Some(50.0));
        assert_eq!(usage.windows[3].label, "Premium requests");

        assert_eq!(parse_user("{}").unwrap().windows, Vec::new());
        assert!(parse_user("[]").is_err());
        assert!(parse_user("<html>").is_err());
    }

    #[test]
    fn http_statuses() {
        let response = |status: u16| HttpResponse {
            status,
            retry_after: None,
            body: PREMIUM.to_string(),
        };
        assert!(interpret_user_response(&response(200)).is_ok());
        for status in [401, 403, 404] {
            assert!(matches!(
                interpret_user_response(&response(status)),
                Err(ProviderError::AuthRequired(_))
            ));
        }
        assert!(matches!(
            interpret_user_response(&response(429)),
            Err(ProviderError::RateLimited { .. })
        ));
    }

    #[test]
    fn token_discovery_from_config_files() {
        assert_eq!(
            parse_copilot_config(
                r#"{"github.com:Iv1.abc":{"user":"someone","oauth_token":"REDACTED-TOKEN","githubAppId":"Iv1.abc"}}"#
            )
            .as_deref(),
            Some("REDACTED-TOKEN")
        );
        assert_eq!(
            parse_copilot_config(r#"{"example.com":{"oauth_token":"other"}}"#),
            None
        );
        assert_eq!(parse_copilot_config("nope"), None);

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(token_from_dir(dir.path()), None);
        std::fs::write(
            dir.path().join("hosts.json"),
            r#"{"github.com":{"user":"someone","oauth_token":"REDACTED-HOSTS"}}"#,
        )
        .unwrap();
        assert_eq!(
            token_from_dir(dir.path()).as_deref(),
            Some("REDACTED-HOSTS")
        );
        std::fs::write(
            dir.path().join("apps.json"),
            r#"{"github.com:Iv1.abc":{"oauth_token":"REDACTED-APPS"}}"#,
        )
        .unwrap();
        assert_eq!(token_from_dir(dir.path()).as_deref(), Some("REDACTED-APPS"));
    }

    #[test]
    fn activity_follows_quota_changes() {
        let now = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let mut state = State::default();
        assert_eq!(state.activity(false, now), Activity::NotRunning);
        assert_eq!(state.activity(true, now), Activity::Idle);

        // First observation is a baseline, not a change.
        state.observe("monthly:Some(10.0):None".into(), now);
        assert_eq!(state.activity(true, now), Activity::Idle);
        state.observe("monthly:Some(10.0):None".into(), now);
        assert_eq!(state.activity(true, now), Activity::Idle);

        state.observe("monthly:Some(11.0):None".into(), now);
        assert_eq!(state.activity(true, now), Activity::Active);
        assert_eq!(state.activity(false, now), Activity::NotRunning);
        let later = now + chrono::Duration::seconds(ACTIVE_WINDOW_SECS + 1);
        assert_eq!(state.activity(true, later), Activity::Idle);
    }

    #[test]
    fn signature_changes_with_usage() {
        let a = [QuotaWindow::percent("monthly", "M", 10.0)];
        let b = [QuotaWindow::percent("monthly", "M", 11.0)];
        assert_eq!(usage_signature(&a), usage_signature(&a));
        assert_ne!(usage_signature(&a), usage_signature(&b));
    }
}
