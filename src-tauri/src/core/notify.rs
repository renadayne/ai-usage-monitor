//! Decides when a quota window deserves a system notification.
//!
//! Each event fires once per window: crossing the warning threshold, crossing
//! the critical threshold, and the quota resetting. The state is persisted so a
//! restart does not repeat notifications.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::model::{ProviderId, ProviderSnapshot};

pub const STATE_FILE: &str = "notify-state.json";

/// A drop of at least this many points down to (almost) zero counts as a reset.
const RESET_MIN_DROP: f32 = 20.0;
const RESET_MAX_USED: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    #[default]
    Normal,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Alert {
    Threshold {
        provider: ProviderId,
        label: String,
        used_percent: f32,
        level: Level,
    },
    Reset {
        provider: ProviderId,
        label: String,
    },
}

impl Alert {
    pub fn title(&self) -> String {
        match self {
            Alert::Threshold {
                provider, level, ..
            } => {
                let severity = if *level == Level::Critical {
                    "almost out of quota"
                } else {
                    "quota running low"
                };
                format!("{}: {severity}", provider.display_name())
            }
            Alert::Reset { provider, .. } => format!("{}: quota reset", provider.display_name()),
        }
    }

    pub fn body(&self) -> String {
        match self {
            Alert::Threshold {
                label,
                used_percent,
                ..
            } => format!("{label}: {used_percent:.0}% used"),
            Alert::Reset { label, .. } => format!("{label} is available again."),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
struct WindowState {
    level: Level,
    used_percent: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Notifier {
    /// Keyed by "<provider>/<window id>".
    windows: BTreeMap<String, WindowState>,
}

fn level_for(used: f32, warn: f32, critical: f32) -> Level {
    if used >= critical {
        Level::Critical
    } else if used >= warn {
        Level::Warning
    } else {
        Level::Normal
    }
}

impl Notifier {
    /// Updates the state from a snapshot and returns the alerts to show.
    pub fn evaluate(
        &mut self,
        snapshot: &ProviderSnapshot,
        warn: f32,
        critical: f32,
    ) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let prefix = format!("{}/", snapshot.provider.as_str());

        for window in &snapshot.windows {
            let Some(used) = window.used_percent else {
                continue;
            };
            let key = format!("{prefix}{}", window.id);
            let level = level_for(used, warn, critical);
            let previous = self.windows.get(&key).cloned();

            match &previous {
                // First sighting: only speak up if it is already over a threshold.
                None => {
                    if level > Level::Normal {
                        alerts.push(Alert::Threshold {
                            provider: snapshot.provider,
                            label: window.label.clone(),
                            used_percent: used,
                            level,
                        });
                    }
                }
                Some(previous) => {
                    if level > previous.level {
                        alerts.push(Alert::Threshold {
                            provider: snapshot.provider,
                            label: window.label.clone(),
                            used_percent: used,
                            level,
                        });
                    } else if previous.used_percent - used >= RESET_MIN_DROP
                        && used <= RESET_MAX_USED
                    {
                        alerts.push(Alert::Reset {
                            provider: snapshot.provider,
                            label: window.label.clone(),
                        });
                    }
                }
            }
            self.windows.insert(
                key,
                WindowState {
                    level,
                    used_percent: used,
                },
            );
        }

        // Forget windows the provider no longer reports (only when it reported
        // something: an error snapshot keeps its old windows anyway).
        if !snapshot.windows.is_empty() {
            self.windows.retain(|key, _| {
                !key.starts_with(&prefix)
                    || snapshot
                        .windows
                        .iter()
                        .any(|window| key[prefix.len()..] == window.id)
            });
        }
        alerts
    }

    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join(STATE_FILE))
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        if let Ok(json) = serde_json::to_string(self) {
            let _ = std::fs::write(dir.join(STATE_FILE), json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::QuotaWindow;
    use chrono::Utc;

    fn snapshot(used: &[(&str, f32)]) -> ProviderSnapshot {
        let mut snapshot = ProviderSnapshot::empty(ProviderId::Claude, Utc::now());
        snapshot.windows = used
            .iter()
            .map(|(id, used)| QuotaWindow::percent(id, &id.to_uppercase(), *used))
            .collect();
        snapshot
    }

    fn check(notifier: &mut Notifier, used: &[(&str, f32)]) -> Vec<Alert> {
        notifier.evaluate(&snapshot(used), 80.0, 95.0)
    }

    #[test]
    fn each_threshold_fires_exactly_once() {
        let mut notifier = Notifier::default();
        assert!(check(&mut notifier, &[("five_hour", 50.0)]).is_empty());
        assert!(check(&mut notifier, &[("five_hour", 79.9)]).is_empty());

        let warning = check(&mut notifier, &[("five_hour", 81.0)]);
        assert_eq!(warning.len(), 1);
        assert!(matches!(
            warning[0],
            Alert::Threshold {
                level: Level::Warning,
                ..
            }
        ));
        assert_eq!(warning[0].title(), "Claude: quota running low");
        assert_eq!(warning[0].body(), "FIVE_HOUR: 81% used");

        // Still above the threshold: silent.
        assert!(check(&mut notifier, &[("five_hour", 85.0)]).is_empty());
        assert!(check(&mut notifier, &[("five_hour", 94.0)]).is_empty());

        let critical = check(&mut notifier, &[("five_hour", 96.0)]);
        assert_eq!(critical.len(), 1);
        assert_eq!(critical[0].title(), "Claude: almost out of quota");
        assert!(check(&mut notifier, &[("five_hour", 99.0)]).is_empty());
    }

    #[test]
    fn jumping_straight_to_critical_sends_one_alert() {
        let mut notifier = Notifier::default();
        check(&mut notifier, &[("weekly", 10.0)]);
        let alerts = check(&mut notifier, &[("weekly", 97.0)]);
        assert_eq!(alerts.len(), 1);
        assert!(matches!(
            alerts[0],
            Alert::Threshold {
                level: Level::Critical,
                ..
            }
        ));
    }

    #[test]
    fn reset_is_announced_once_and_thresholds_rearm() {
        let mut notifier = Notifier::default();
        check(&mut notifier, &[("five_hour", 96.0)]);

        let reset = check(&mut notifier, &[("five_hour", 0.0)]);
        assert_eq!(
            reset,
            vec![Alert::Reset {
                provider: ProviderId::Claude,
                label: "FIVE_HOUR".into()
            }]
        );
        assert_eq!(reset[0].body(), "FIVE_HOUR is available again.");
        assert!(check(&mut notifier, &[("five_hour", 1.0)]).is_empty());

        // The next cycle notifies again.
        assert_eq!(check(&mut notifier, &[("five_hour", 82.0)]).len(), 1);
    }

    #[test]
    fn small_dips_are_not_resets() {
        let mut notifier = Notifier::default();
        check(&mut notifier, &[("weekly", 60.0)]);
        // Rolling windows drift down a little without resetting.
        assert!(check(&mut notifier, &[("weekly", 52.0)]).is_empty());
        assert!(check(&mut notifier, &[("weekly", 30.0)]).is_empty());
        // A tiny value that was never high is not a reset either.
        let mut low = Notifier::default();
        check(&mut low, &[("weekly", 12.0)]);
        assert!(check(&mut low, &[("weekly", 0.0)]).is_empty());
    }

    #[test]
    fn first_sighting_above_threshold_alerts_but_restart_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut notifier = Notifier::load(dir.path());
        assert_eq!(check(&mut notifier, &[("weekly", 90.0)]).len(), 1);
        notifier.save(dir.path());

        // Same numbers after a restart: nothing new to say.
        let mut restarted = Notifier::load(dir.path());
        assert_eq!(restarted, notifier);
        assert!(check(&mut restarted, &[("weekly", 90.0)]).is_empty());
    }

    #[test]
    fn windows_are_tracked_independently_and_pruned() {
        let mut notifier = Notifier::default();
        let alerts = check(&mut notifier, &[("five_hour", 85.0), ("weekly", 20.0)]);
        assert_eq!(alerts.len(), 1);
        // An error snapshot without windows keeps the state.
        assert!(check(&mut notifier, &[]).is_empty());
        assert!(check(&mut notifier, &[("five_hour", 85.0), ("weekly", 20.0)]).is_empty());
        // A window that disappears is forgotten, so it can alert again later.
        check(&mut notifier, &[("weekly", 20.0)]);
        assert_eq!(
            check(&mut notifier, &[("five_hour", 85.0), ("weekly", 20.0)]).len(),
            1
        );

        // Other providers are untouched by pruning.
        let mut other = snapshot(&[("weekly", 90.0)]);
        other.provider = ProviderId::Codex;
        assert_eq!(notifier.evaluate(&other, 80.0, 95.0).len(), 1);
        check(&mut notifier, &[("weekly", 20.0)]);
        assert!(notifier.evaluate(&other, 80.0, 95.0).is_empty());
    }
}
