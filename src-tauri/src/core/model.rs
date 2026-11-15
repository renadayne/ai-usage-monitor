use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Claude,
    Codex,
    Antigravity,
    Copilot,
}

impl ProviderId {
    pub const ALL: [ProviderId; 4] = [
        ProviderId::Claude,
        ProviderId::Codex,
        ProviderId::Antigravity,
        ProviderId::Copilot,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderId::Claude => "claude",
            ProviderId::Codex => "codex",
            ProviderId::Antigravity => "antigravity",
            ProviderId::Copilot => "copilot",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.as_str() == value)
    }

    /// Compact name for tight spots such as the tray tooltip.
    pub fn short_name(self) -> &'static str {
        match self {
            ProviderId::Copilot => "Copilot",
            other => other.display_name(),
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderId::Claude => "Claude",
            ProviderId::Codex => "Codex",
            ProviderId::Antigravity => "Antigravity",
            ProviderId::Copilot => "GitHub Copilot",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    /// Running and recently consumed quota.
    Active,
    /// Process is running but nothing happened recently.
    Idle,
    NotRunning,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Health {
    Ok,
    /// Showing old data.
    Stale {
        since: DateTime<Utc>,
    },
    /// Not logged in / token expired.
    AuthRequired {
        hint: String,
    },
    NotInstalled,
    RateLimited {
        retry_at: DateTime<Utc>,
    },
    /// `message` must never contain a secret.
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindow {
    /// "five_hour", "weekly", "monthly", "model:<name>", ...
    pub id: String,
    pub label: String,
    /// 0..100
    pub used_percent: Option<f32>,
    pub remaining: Option<f64>,
    pub limit: Option<f64>,
    /// "credits", "requests", ...
    pub unit: Option<String>,
    pub resets_at: Option<DateTime<Utc>>,
    /// True when the value is inferred rather than reported by the provider.
    #[serde(default)]
    pub estimated: bool,
}

impl QuotaWindow {
    pub fn percent(id: &str, label: &str, used_percent: f32) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            used_percent: Some(used_percent.clamp(0.0, 100.0)),
            remaining: None,
            limit: None,
            unit: None,
            resets_at: None,
            estimated: false,
        }
    }

    pub fn with_reset(mut self, resets_at: Option<DateTime<Utc>>) -> Self {
        self.resets_at = resets_at;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub provider: ProviderId,
    pub activity: Activity,
    pub health: Health,
    pub plan: Option<String>,
    pub current_model: Option<String>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub windows: Vec<QuotaWindow>,
    /// Free-form remark for the UI ("API key mode", ...). Never a secret.
    #[serde(default)]
    pub note: Option<String>,
    pub fetched_at: DateTime<Utc>,
}

impl ProviderSnapshot {
    pub fn empty(provider: ProviderId, now: DateTime<Utc>) -> Self {
        Self {
            provider,
            activity: Activity::Unknown,
            health: Health::Ok,
            plan: None,
            current_model: None,
            last_activity_at: None,
            windows: Vec::new(),
            note: None,
            fetched_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_round_trips() {
        for id in ProviderId::ALL {
            assert_eq!(ProviderId::parse(id.as_str()), Some(id));
        }
        assert_eq!(ProviderId::parse("nope"), None);
    }

    #[test]
    fn health_serializes_with_kind_tag() {
        let json = serde_json::to_value(Health::AuthRequired {
            hint: "login".into(),
        })
        .unwrap();
        assert_eq!(json["kind"], "auth_required");
        assert_eq!(json["hint"], "login");
        assert_eq!(serde_json::to_value(Health::Ok).unwrap()["kind"], "ok");
    }

    #[test]
    fn percent_window_is_clamped() {
        assert_eq!(
            QuotaWindow::percent("a", "A", 140.0).used_percent,
            Some(100.0)
        );
        assert_eq!(QuotaWindow::percent("a", "A", -3.0).used_percent, Some(0.0));
    }
}
