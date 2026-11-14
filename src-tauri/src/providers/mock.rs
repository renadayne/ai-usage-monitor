//! Fake provider used by scheduler tests and by `AIUM_MOCK=1` demo runs.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::engine::model::{Activity, ProviderId, QuotaWindow};
use crate::engine::provider::{ActivityInfo, Provider, ProviderError, QuotaInfo};

pub struct MockProvider {
    id: ProviderId,
    poll_interval: Duration,
    quota_calls: AtomicUsize,
    activity_calls: AtomicUsize,
    failure: Mutex<Option<ProviderError>>,
    hang: AtomicBool,
}

impl MockProvider {
    pub fn new(id: ProviderId, poll_interval: Duration) -> Self {
        Self {
            id,
            poll_interval,
            quota_calls: AtomicUsize::new(0),
            activity_calls: AtomicUsize::new(0),
            failure: Mutex::new(None),
            hang: AtomicBool::new(false),
        }
    }

    pub fn quota_calls(&self) -> usize {
        self.quota_calls.load(Ordering::SeqCst)
    }

    pub fn activity_calls(&self) -> usize {
        self.activity_calls.load(Ordering::SeqCst)
    }

    /// Every following `fetch_quota` fails with `error`.
    pub fn fail_with(&self, error: ProviderError) {
        *self.failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
    }

    /// Every following `fetch_quota` never completes.
    pub fn hang_forever(&self) {
        self.hang.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    async fn detect_activity(&self) -> ActivityInfo {
        let calls = self.activity_calls.fetch_add(1, Ordering::SeqCst);
        ActivityInfo {
            activity: if calls.is_multiple_of(3) {
                Activity::Idle
            } else {
                Activity::Active
            },
            current_model: Some(format!("mock-model-{}", self.id.as_str())),
            last_activity_at: Some(Utc::now()),
        }
    }

    async fn fetch_quota(&self) -> Result<QuotaInfo, ProviderError> {
        let calls = self.quota_calls.fetch_add(1, Ordering::SeqCst);
        if self.hang.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        let failure = self
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(error) = failure {
            return Err(error);
        }
        let used = ((calls * 7) % 100) as f32;
        Ok(QuotaInfo {
            plan: Some("mock".to_string()),
            windows: vec![
                QuotaWindow::percent("five_hour", "5 hours", used)
                    .with_reset(Some(Utc::now() + chrono::Duration::hours(3))),
                QuotaWindow::percent("weekly", "Weekly", used / 2.0)
                    .with_reset(Some(Utc::now() + chrono::Duration::days(4))),
            ],
            ..QuotaInfo::default()
        })
    }
}
