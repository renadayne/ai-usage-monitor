//! One tokio task per provider with two cadences: a fast local activity probe
//! and a slow quota fetch with exponential backoff. A failing provider never
//! affects the others.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use super::model::{Health, ProviderId, ProviderSnapshot};
use super::provider::{ActivityInfo, Provider, ProviderError, QuotaInfo};
use super::store::Store;

/// Receives every snapshot change (the Tauri layer emits `snapshot-updated`).
pub trait SnapshotSink: Send + Sync {
    fn publish(&self, snapshot: &ProviderSnapshot);
}

/// Sink that drops everything.
pub struct NullSink;

impl SnapshotSink for NullSink {
    fn publish(&self, _snapshot: &ProviderSnapshot) {}
}

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub activity_interval: Duration,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    /// Upper bound for a single provider call.
    pub call_timeout: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            activity_interval: Duration::from_secs(10),
            backoff_base: Duration::from_secs(60),
            backoff_max: Duration::from_secs(30 * 60),
            call_timeout: Duration::from_secs(45),
        }
    }
}

/// Per-provider runtime settings that can change without a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSettings {
    pub enabled: bool,
    /// Overrides `Provider::poll_interval` when set.
    pub poll_interval: Option<Duration>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval: None,
        }
    }
}

/// Exponential backoff: base, 2x base, 4x base ... capped at `max`.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    failures: u32,
}

impl Backoff {
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            failures: 0,
        }
    }

    pub fn next_delay(&mut self) -> Duration {
        let factor = 1u32.checked_shl(self.failures).unwrap_or(u32::MAX);
        self.failures = self.failures.saturating_add(1);
        self.base.saturating_mul(factor).min(self.max)
    }

    pub fn reset(&mut self) {
        self.failures = 0;
    }
}

pub fn apply_activity(snapshot: &mut ProviderSnapshot, info: ActivityInfo) {
    snapshot.activity = info.activity;
    if info.current_model.is_some() {
        snapshot.current_model = info.current_model;
    }
    if info.last_activity_at.is_some() {
        snapshot.last_activity_at = info.last_activity_at;
    }
}

/// Folds a quota result into the snapshot. Old windows are kept on errors so the
/// UI can keep showing the last known numbers.
pub fn apply_quota(
    snapshot: &mut ProviderSnapshot,
    result: Result<QuotaInfo, ProviderError>,
    retry_at: DateTime<Utc>,
    now: DateTime<Utc>,
) {
    match result {
        Ok(info) => {
            if !(info.keep_previous_windows && info.windows.is_empty()) {
                snapshot.windows = info.windows;
            }
            if info.plan.is_some() {
                snapshot.plan = info.plan;
            }
            if info.current_model.is_some() {
                snapshot.current_model = info.current_model;
            }
            snapshot.note = info.note;
            snapshot.health = info.health.unwrap_or(Health::Ok);
            snapshot.fetched_at = now;
        }
        Err(ProviderError::NotInstalled) => {
            snapshot.health = Health::NotInstalled;
            snapshot.windows.clear();
            snapshot.plan = None;
            snapshot.note = None;
        }
        Err(ProviderError::AuthRequired(hint)) => {
            snapshot.health = Health::AuthRequired { hint };
        }
        Err(ProviderError::RateLimited { .. }) => {
            snapshot.health = Health::RateLimited { retry_at };
        }
        Err(ProviderError::Other(message)) => {
            snapshot.health = Health::Error { message };
        }
    }
}

struct Shared {
    config: SchedulerConfig,
    store: Option<Arc<Store>>,
    sink: Arc<dyn SnapshotSink>,
    latest: Mutex<HashMap<ProviderId, ProviderSnapshot>>,
    settings: RwLock<HashMap<ProviderId, ProviderSettings>>,
}

impl Shared {
    fn settings_for(&self, id: ProviderId) -> ProviderSettings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .copied()
            .unwrap_or_default()
    }

    /// Mutates the provider's snapshot and returns the new value when it changed.
    fn update(
        &self,
        id: ProviderId,
        mutate: impl FnOnce(&mut ProviderSnapshot),
    ) -> Option<ProviderSnapshot> {
        let mut latest = self.latest.lock().unwrap_or_else(|e| e.into_inner());
        let entry = latest
            .entry(id)
            .or_insert_with(|| ProviderSnapshot::empty(id, Utc::now()));
        let before = entry.clone();
        mutate(entry);
        (*entry != before).then(|| entry.clone())
    }
}

pub struct Scheduler {
    providers: Vec<Arc<dyn Provider>>,
    refresh: HashMap<ProviderId, Arc<Notify>>,
    shared: Arc<Shared>,
}

impl Scheduler {
    pub fn new(
        providers: Vec<Arc<dyn Provider>>,
        config: SchedulerConfig,
        store: Option<Arc<Store>>,
        sink: Arc<dyn SnapshotSink>,
    ) -> Self {
        let now = Utc::now();
        let mut latest: HashMap<ProviderId, ProviderSnapshot> = providers
            .iter()
            .map(|p| (p.id(), ProviderSnapshot::empty(p.id(), now)))
            .collect();
        if let Some(store) = &store {
            for mut snapshot in store.load_latest().unwrap_or_default() {
                if let Entry::Occupied(mut slot) = latest.entry(snapshot.provider) {
                    // Activity from a previous run says nothing about now.
                    snapshot.activity = super::model::Activity::Unknown;
                    slot.insert(snapshot);
                }
            }
        }
        let refresh = providers
            .iter()
            .map(|p| (p.id(), Arc::new(Notify::new())))
            .collect();
        Self {
            providers,
            refresh,
            shared: Arc::new(Shared {
                config,
                store,
                sink,
                latest: Mutex::new(latest),
                settings: RwLock::new(HashMap::new()),
            }),
        }
    }

    /// Spawns one task per provider. Must be called from within a tokio runtime.
    pub fn start(&self) -> Vec<JoinHandle<()>> {
        self.providers
            .iter()
            .map(|provider| {
                let provider = Arc::clone(provider);
                let shared = Arc::clone(&self.shared);
                let refresh = Arc::clone(&self.refresh[&provider.id()]);
                tokio::spawn(run_provider(provider, shared, refresh))
            })
            .collect()
    }

    /// Latest snapshots in the canonical provider order.
    pub fn snapshots(&self) -> Vec<ProviderSnapshot> {
        let latest = self.shared.latest.lock().unwrap_or_else(|e| e.into_inner());
        ProviderId::ALL
            .iter()
            .filter_map(|id| latest.get(id).cloned())
            .collect()
    }

    /// Requests an immediate quota fetch. Backoff is still honoured.
    pub fn refresh(&self, provider: Option<ProviderId>) {
        for (id, notify) in &self.refresh {
            if provider.is_none_or(|wanted| wanted == *id) {
                notify.notify_one();
            }
        }
    }

    pub fn set_provider_settings(&self, id: ProviderId, settings: ProviderSettings) {
        self.shared
            .settings
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, settings);
        // Wake the task so the new settings take effect right away.
        if let Some(notify) = self.refresh.get(&id) {
            notify.notify_one();
        }
    }

    pub fn store(&self) -> Option<Arc<Store>> {
        self.shared.store.clone()
    }
}

enum Wake {
    Activity,
    Quota,
    Manual,
}

async fn run_provider(provider: Arc<dyn Provider>, shared: Arc<Shared>, refresh: Arc<Notify>) {
    let id = provider.id();
    let config = shared.config.clone();
    let mut backoff = Backoff::new(config.backoff_base, config.backoff_max);
    let mut blocked_until: Option<Instant> = None;
    let mut next_quota = Instant::now();
    let mut ticker = tokio::time::interval(config.activity_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        let wake = tokio::select! {
            _ = ticker.tick() => Wake::Activity,
            _ = tokio::time::sleep_until(next_quota) => Wake::Quota,
            _ = refresh.notified() => Wake::Manual,
        };

        let settings = shared.settings_for(id);
        if !settings.enabled {
            // Idle cheaply until re-enabled; a settings change notifies `refresh`.
            next_quota = Instant::now() + config.activity_interval;
            continue;
        }

        let fetch_quota = match wake {
            Wake::Activity => false,
            Wake::Quota => true,
            Wake::Manual => blocked_until.is_none_or(|until| Instant::now() >= until),
        };

        let changed = if fetch_quota {
            let result =
                match tokio::time::timeout(config.call_timeout, provider.fetch_quota()).await {
                    Ok(result) => result,
                    Err(_) => Err(ProviderError::Other("request timed out".to_string())),
                };

            let interval = settings
                .poll_interval
                .unwrap_or_else(|| provider.poll_interval());
            let delay = match &result {
                Ok(_) => {
                    backoff.reset();
                    blocked_until = None;
                    interval
                }
                // Nothing to back off from: just look again on the normal cadence.
                Err(ProviderError::NotInstalled) | Err(ProviderError::AuthRequired(_)) => {
                    backoff.reset();
                    blocked_until = None;
                    interval
                }
                Err(ProviderError::RateLimited { retry_after }) => {
                    let delay = backoff
                        .next_delay()
                        .max(retry_after.unwrap_or_default())
                        .min(config.backoff_max);
                    blocked_until = Some(Instant::now() + delay);
                    delay
                }
                Err(ProviderError::Other(_)) => {
                    let delay = backoff.next_delay();
                    blocked_until = Some(Instant::now() + delay);
                    delay
                }
            };
            next_quota = Instant::now() + delay;

            let succeeded = result.is_ok();
            let now = Utc::now();
            let retry_at = now + chrono::Duration::from_std(delay).unwrap_or_default();
            let changed =
                shared.update(id, |snapshot| apply_quota(snapshot, result, retry_at, now));
            if let (Some(snapshot), Some(store), true) = (&changed, &shared.store, succeeded) {
                let _ = store.record_history(snapshot);
            }
            changed
        } else {
            let info = tokio::time::timeout(config.call_timeout, provider.detect_activity())
                .await
                .unwrap_or_else(|_| ActivityInfo::unknown());
            shared.update(id, |snapshot| apply_activity(snapshot, info))
        };

        if let Some(snapshot) = changed {
            if let Some(store) = &shared.store {
                let _ = store.save_latest(&snapshot);
            }
            shared.sink.publish(&snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{Activity, QuotaWindow};
    use crate::providers::mock::MockProvider;

    #[derive(Default)]
    struct CollectSink(Mutex<Vec<ProviderSnapshot>>);

    impl SnapshotSink for CollectSink {
        fn publish(&self, snapshot: &ProviderSnapshot) {
            self.0.lock().unwrap().push(snapshot.clone());
        }
    }

    fn config() -> SchedulerConfig {
        SchedulerConfig {
            activity_interval: Duration::from_secs(10),
            backoff_base: Duration::from_secs(60),
            backoff_max: Duration::from_secs(30 * 60),
            call_timeout: Duration::from_secs(30),
        }
    }

    fn scheduler(providers: Vec<Arc<MockProvider>>, sink: Arc<dyn SnapshotSink>) -> Scheduler {
        let providers = providers
            .into_iter()
            .map(|p| p as Arc<dyn Provider>)
            .collect();
        Scheduler::new(providers, config(), None, sink)
    }

    fn ok_quota(used: f32) -> Result<QuotaInfo, ProviderError> {
        Ok(QuotaInfo {
            plan: Some("pro".into()),
            windows: vec![QuotaWindow::percent("five_hour", "5 hours", used)],
            ..QuotaInfo::default()
        })
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut backoff = Backoff::new(Duration::from_secs(60), Duration::from_secs(300));
        assert_eq!(backoff.next_delay(), Duration::from_secs(60));
        assert_eq!(backoff.next_delay(), Duration::from_secs(120));
        assert_eq!(backoff.next_delay(), Duration::from_secs(240));
        assert_eq!(backoff.next_delay(), Duration::from_secs(300));
        for _ in 0..80 {
            assert_eq!(backoff.next_delay(), Duration::from_secs(300));
        }
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(60));
    }

    #[test]
    fn quota_errors_keep_old_windows() {
        let now = Utc::now();
        let mut snapshot = ProviderSnapshot::empty(ProviderId::Claude, now);
        apply_quota(&mut snapshot, ok_quota(40.0), now, now);
        assert_eq!(snapshot.health, Health::Ok);
        assert_eq!(snapshot.windows.len(), 1);

        apply_quota(
            &mut snapshot,
            Err(ProviderError::RateLimited { retry_after: None }),
            now,
            now,
        );
        assert_eq!(snapshot.health, Health::RateLimited { retry_at: now });
        assert_eq!(snapshot.windows.len(), 1);

        apply_quota(
            &mut snapshot,
            Err(ProviderError::AuthRequired("login".into())),
            now,
            now,
        );
        assert!(matches!(snapshot.health, Health::AuthRequired { .. }));
        assert_eq!(snapshot.windows.len(), 1);

        apply_quota(
            &mut snapshot,
            Err(ProviderError::Other("boom".into())),
            now,
            now,
        );
        assert_eq!(
            snapshot.health,
            Health::Error {
                message: "boom".into()
            }
        );

        apply_quota(&mut snapshot, Err(ProviderError::NotInstalled), now, now);
        assert_eq!(snapshot.health, Health::NotInstalled);
        assert!(snapshot.windows.is_empty());
    }

    #[test]
    fn activity_does_not_erase_known_model() {
        let mut snapshot = ProviderSnapshot::empty(ProviderId::Codex, Utc::now());
        snapshot.current_model = Some("model-a".into());
        apply_activity(&mut snapshot, ActivityInfo::new(Activity::Idle));
        assert_eq!(snapshot.activity, Activity::Idle);
        assert_eq!(snapshot.current_model.as_deref(), Some("model-a"));
    }

    #[tokio::test(start_paused = true)]
    async fn polls_on_both_cadences() {
        let provider = Arc::new(MockProvider::new(
            ProviderId::Claude,
            Duration::from_secs(60),
        ));
        let sink = Arc::new(CollectSink::default());
        let scheduler = scheduler(vec![provider.clone()], sink.clone());
        let _tasks = scheduler.start();

        tokio::time::sleep(Duration::from_secs(125)).await;

        // Quota at t = 0, 60, 120.
        assert_eq!(provider.quota_calls(), 3);
        // Activity every 10 s: t = 0, 10, ..., 120.
        let activity = provider.activity_calls();
        assert!((12..=14).contains(&activity), "activity calls: {activity}");
        assert!(!sink.0.lock().unwrap().is_empty());
        assert_eq!(scheduler.snapshots().len(), 1);
        assert_eq!(scheduler.snapshots()[0].health, Health::Ok);
    }

    #[tokio::test(start_paused = true)]
    async fn failures_back_off_exponentially() {
        let provider = Arc::new(MockProvider::new(
            ProviderId::Codex,
            Duration::from_secs(60),
        ));
        provider.fail_with(ProviderError::Other("down".into()));
        let scheduler = scheduler(vec![provider.clone()], Arc::new(NullSink));
        let _tasks = scheduler.start();

        // Attempts at t = 0, 60, 180, 420 (delays 60, 120, 240).
        tokio::time::sleep(Duration::from_secs(59)).await;
        assert_eq!(provider.quota_calls(), 1);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(provider.quota_calls(), 2);
        tokio::time::sleep(Duration::from_secs(118)).await;
        assert_eq!(provider.quota_calls(), 2);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(provider.quota_calls(), 3);
        tokio::time::sleep(Duration::from_secs(240)).await;
        assert_eq!(provider.quota_calls(), 4);

        assert!(matches!(
            scheduler.snapshots()[0].health,
            Health::Error { .. }
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_honours_retry_after() {
        let provider = Arc::new(MockProvider::new(
            ProviderId::Claude,
            Duration::from_secs(60),
        ));
        provider.fail_with(ProviderError::RateLimited {
            retry_after: Some(Duration::from_secs(600)),
        });
        let scheduler = scheduler(vec![provider.clone()], Arc::new(NullSink));
        let _tasks = scheduler.start();

        tokio::time::sleep(Duration::from_secs(590)).await;
        assert_eq!(provider.quota_calls(), 1);
        assert!(matches!(
            scheduler.snapshots()[0].health,
            Health::RateLimited { .. }
        ));
        tokio::time::sleep(Duration::from_secs(20)).await;
        assert_eq!(provider.quota_calls(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn manual_refresh_respects_backoff() {
        let provider = Arc::new(MockProvider::new(
            ProviderId::Copilot,
            Duration::from_secs(600),
        ));
        let scheduler = scheduler(vec![provider.clone()], Arc::new(NullSink));
        let _tasks = scheduler.start();

        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(provider.quota_calls(), 1);

        // Healthy provider: manual refresh fetches immediately.
        scheduler.refresh(Some(ProviderId::Copilot));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(provider.quota_calls(), 2);

        // Failing provider: the refresh after the failure is ignored until backoff ends.
        provider.fail_with(ProviderError::Other("down".into()));
        scheduler.refresh(None);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(provider.quota_calls(), 3);
        scheduler.refresh(None);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(provider.quota_calls(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn one_failing_provider_does_not_affect_others() {
        let healthy = Arc::new(MockProvider::new(
            ProviderId::Claude,
            Duration::from_secs(60),
        ));
        let broken = Arc::new(MockProvider::new(
            ProviderId::Codex,
            Duration::from_secs(60),
        ));
        broken.fail_with(ProviderError::Other("down".into()));
        let hanging = Arc::new(MockProvider::new(
            ProviderId::Copilot,
            Duration::from_secs(60),
        ));
        hanging.hang_forever();

        let scheduler = scheduler(
            vec![healthy.clone(), broken.clone(), hanging.clone()],
            Arc::new(NullSink),
        );
        let _tasks = scheduler.start();
        tokio::time::sleep(Duration::from_secs(185)).await;

        assert_eq!(healthy.quota_calls(), 4);
        let snapshots = scheduler.snapshots();
        assert_eq!(snapshots.len(), 3);
        assert_eq!(snapshots[0].provider, ProviderId::Claude);
        assert_eq!(snapshots[0].health, Health::Ok);
        assert!(matches!(snapshots[1].health, Health::Error { .. }));
        assert!(matches!(snapshots[2].health, Health::Error { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn disabled_provider_is_not_polled() {
        let provider = Arc::new(MockProvider::new(
            ProviderId::Claude,
            Duration::from_secs(60),
        ));
        let scheduler = scheduler(vec![provider.clone()], Arc::new(NullSink));
        scheduler.set_provider_settings(
            ProviderId::Claude,
            ProviderSettings {
                enabled: false,
                poll_interval: None,
            },
        );
        let _tasks = scheduler.start();
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert_eq!(provider.quota_calls(), 0);
        assert_eq!(provider.activity_calls(), 0);

        scheduler.set_provider_settings(ProviderId::Claude, ProviderSettings::default());
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert!(provider.quota_calls() >= 1);
    }
}
