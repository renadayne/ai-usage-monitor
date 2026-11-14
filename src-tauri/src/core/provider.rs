use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::model::{Activity, Health, ProviderId, QuotaWindow};

/// Result of the fast, local-only activity probe.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityInfo {
    pub activity: Activity,
    pub current_model: Option<String>,
    pub last_activity_at: Option<DateTime<Utc>>,
}

impl ActivityInfo {
    pub fn new(activity: Activity) -> Self {
        Self {
            activity,
            current_model: None,
            last_activity_at: None,
        }
    }

    pub fn unknown() -> Self {
        Self::new(Activity::Unknown)
    }
}

/// Result of the slow quota fetch.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuotaInfo {
    pub plan: Option<String>,
    pub windows: Vec<QuotaWindow>,
    pub current_model: Option<String>,
    pub note: Option<String>,
    /// Overrides `Health::Ok`, e.g. `Stale` when serving a cached snapshot.
    pub health: Option<Health>,
    /// When true and `windows` is empty, keep the windows already shown.
    pub keep_previous_windows: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderError {
    NotInstalled,
    AuthRequired(String),
    RateLimited {
        retry_after: Option<Duration>,
    },
    /// The message is shown in the UI and must never contain a secret.
    Other(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::NotInstalled => write!(f, "not installed"),
            ProviderError::AuthRequired(hint) => write!(f, "authentication required: {hint}"),
            ProviderError::RateLimited { .. } => write!(f, "rate limited"),
            ProviderError::Other(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ProviderError {}

#[async_trait]
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Interval of the slow quota fetch.
    fn poll_interval(&self) -> Duration;

    /// Local and fast; called roughly every 10 seconds.
    async fn detect_activity(&self) -> ActivityInfo;

    async fn fetch_quota(&self) -> Result<QuotaInfo, ProviderError>;
}
