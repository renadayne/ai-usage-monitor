pub mod claude;
pub mod codex;
pub mod mock;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Duration, TimeZone, Utc};

use crate::engine::model::Activity;
use crate::engine::paths::PathResolver;
use crate::engine::process::ProcessScanner;

/// A log written within this window counts as "currently consuming quota".
pub const ACTIVE_WINDOW_SECS: i64 = 120;

/// Shared services handed to every provider.
#[derive(Clone)]
pub struct ProviderContext {
    pub paths: Arc<PathResolver>,
    pub processes: Arc<ProcessScanner>,
    pub http: reqwest::Client,
    /// The app's own data directory (`%LOCALAPPDATA%\ai-usage-monitor`).
    pub data_dir: PathBuf,
}

/// Combines "is the process there" with "was the log just written".
///
/// Processes inside WSL are invisible from Windows, so there the log's mtime
/// is the only signal.
pub fn activity_from(
    process_running: bool,
    is_wsl: bool,
    last_write: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Activity {
    let recent = last_write.is_some_and(|at| now - at <= Duration::seconds(ACTIVE_WINDOW_SECS));
    if is_wsl {
        if recent {
            Activity::Active
        } else {
            Activity::Unknown
        }
    } else if !process_running {
        Activity::NotRunning
    } else if recent {
        Activity::Active
    } else {
        Activity::Idle
    }
}

pub fn to_utc(time: SystemTime) -> DateTime<Utc> {
    DateTime::<Utc>::from(time)
}

/// Accepts an RFC 3339 string or a Unix timestamp (seconds or milliseconds).
pub fn parse_time(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    match value {
        serde_json::Value::String(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|time| time.with_timezone(&Utc)),
        serde_json::Value::Number(number) => {
            let raw = number.as_f64()?;
            if !raw.is_finite() || raw <= 0.0 {
                return None;
            }
            // Anything this large can only be milliseconds.
            let seconds = if raw > 1.0e11 { raw / 1000.0 } else { raw };
            Utc.timestamp_opt(seconds as i64, 0).single()
        }
        _ => None,
    }
}

/// "seven_day_opus" -> "Seven day opus"
pub fn humanize(key: &str) -> String {
    let spaced = key.replace(['_', '-'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_matrix() {
        let now = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let recent = Some(now - Duration::seconds(30));
        let old = Some(now - Duration::seconds(600));

        assert_eq!(activity_from(true, false, recent, now), Activity::Active);
        assert_eq!(activity_from(true, false, old, now), Activity::Idle);
        assert_eq!(activity_from(true, false, None, now), Activity::Idle);
        assert_eq!(
            activity_from(false, false, recent, now),
            Activity::NotRunning
        );
        assert_eq!(activity_from(false, true, recent, now), Activity::Active);
        assert_eq!(activity_from(false, true, old, now), Activity::Unknown);
    }

    #[test]
    fn parses_times() {
        let expected = Utc.timestamp_opt(1_794_690_000, 0).unwrap();
        assert_eq!(
            parse_time(&serde_json::json!(1_794_690_000)),
            Some(expected)
        );
        assert_eq!(
            parse_time(&serde_json::json!(1_794_690_000_000_i64)),
            Some(expected)
        );
        assert_eq!(
            parse_time(&serde_json::json!(expected.to_rfc3339())),
            Some(expected)
        );
        assert_eq!(
            parse_time(&serde_json::json!("2026-11-14T20:00:00.000000+00:00"))
                .map(|t| t.timestamp()),
            Some(1_794_686_400)
        );
        assert_eq!(parse_time(&serde_json::json!(null)), None);
        assert_eq!(parse_time(&serde_json::json!("soon")), None);
        assert_eq!(parse_time(&serde_json::json!(0)), None);
    }

    #[test]
    fn humanizes_keys() {
        assert_eq!(humanize("seven_day_opus"), "Seven day opus");
        assert_eq!(humanize(""), "");
    }
}
